use crate::{
    app,
    policy::Policy,
    system::{self, Handle, MIB},
};
use std::{
    collections::HashMap,
    fs, io,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::{
    Win32::{
        Foundation::*,
        Security::*,
        System::{
            ProcessStatus::K32EmptyWorkingSet, SystemInformation::GetWindowsDirectoryW,
            Threading::*,
        },
        UI::WindowsAndMessaging::*,
    },
    core::w,
};

const STOP_NAME: *const u16 = w!("Local\\RamObserver-Background-Stop-v1");
const MIN_IDLE: Duration = Duration::from_secs(60);
const SCAN: Duration = Duration::from_secs(10);
const MIN_WORKING_SET: usize = 128 * 1024 * 1024;

struct Candidate {
    handle: Handle,
    since: Instant,
    sampled: Instant,
    cpu: u64,
    idle: bool,
}

fn checked_handle(raw: HANDLE) -> io::Result<Handle> {
    if raw.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(Handle(raw))
    }
}

fn token_user(process: HANDLE) -> io::Result<Vec<usize>> {
    // SAFETY: owned token handle, aligned output storage, API-checked buffer length.
    unsafe {
        let mut raw = null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut raw) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = checked_handle(raw)?;
        let mut bytes = 0;
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut bytes);
        if GetLastError() != ERROR_INSUFFICIENT_BUFFER
            || !(size_of::<TOKEN_USER>() as u32..=65536).contains(&bytes)
        {
            return Err(io::Error::other("Cannot identify token user"));
        }
        let mut storage = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(storage)
    }
}

fn same_user(process: HANDLE, own_user: &[usize]) -> io::Result<bool> {
    let user = token_user(process)?;
    // SAFETY: both aligned buffers are complete TOKEN_USER results and remain alive.
    unsafe {
        let a = &*(user.as_ptr().cast::<TOKEN_USER>());
        let b = &*(own_user.as_ptr().cast::<TOKEN_USER>());
        Ok(EqualSid(a.User.Sid, b.User.Sid) != 0)
    }
}

fn allowed(
    process: HANDLE,
    own_user: &[usize],
    windows_root: &str,
    own_image: &str,
) -> io::Result<bool> {
    // SAFETY: retained process handle, correctly sized initialized API outputs.
    unsafe {
        if WaitForSingleObject(process, 0) != WAIT_TIMEOUT {
            return Ok(false);
        }
        let mut critical = 0;
        let mut protection: PROCESS_PROTECTION_LEVEL_INFORMATION = zeroed();
        if IsProcessCritical(process, &mut critical) == 0
            || GetProcessInformation(
                process,
                ProcessProtectionLevelInfo,
                (&mut protection as *mut PROCESS_PROTECTION_LEVEL_INFORMATION).cast(),
                size_of::<PROCESS_PROTECTION_LEVEL_INFORMATION>() as u32,
            ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let image = system::image_path(process)?.to_lowercase();
        Ok(critical == 0
            && protection.ProtectionLevel == PROTECTION_LEVEL_NONE
            && !image.starts_with(windows_root)
            && image != own_image
            && same_user(process, own_user)?)
    }
}

fn cpu_time(process: HANDLE) -> io::Result<u64> {
    // SAFETY: four initialized FILETIME outputs; the process handle remains owned.
    unsafe {
        let (mut created, mut exited, mut kernel, mut user): (
            FILETIME,
            FILETIME,
            FILETIME,
            FILETIME,
        ) = zeroed();
        if GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return Err(io::Error::last_os_error());
        }
        let ticks =
            |time: FILETIME| ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64;
        Ok(ticks(kernel).saturating_add(ticks(user)))
    }
}

unsafe extern "system" fn enum_window(hwnd: HWND, context: LPARAM) -> i32 {
    // SAFETY: context points to the map borrowed by the synchronous EnumWindows call.
    unsafe {
        if IsWindowVisible(hwnd) != 0 {
            let mut pid = 0;
            if GetWindowThreadProcessId(hwnd, &mut pid) != 0 && pid != 0 {
                let windows = &mut *(context as *mut HashMap<u32, bool>);
                *windows.entry(pid).or_insert(true) &= IsIconic(hwnd) != 0;
            }
        }
        1
    }
}

fn minimized_windows() -> io::Result<HashMap<u32, bool>> {
    let mut windows = HashMap::new();
    // SAFETY: callback is synchronous and context stays valid throughout enumeration.
    if unsafe {
        EnumWindows(
            Some(enum_window),
            (&mut windows as *mut HashMap<u32, bool>) as LPARAM,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(windows)
}

fn cpu_idle(delta_ticks: u64, elapsed: Duration) -> bool {
    // CPU times are in 100ns units. Accept at most 1% of one CPU core.
    !elapsed.is_zero() && u128::from(delta_ticks) * 100 * 100 <= elapsed.as_nanos()
}

fn ready(candidate: &Candidate, now: Instant) -> bool {
    candidate.idle
        && now.duration_since(candidate.since) >= MIN_IDLE
        && now.duration_since(candidate.sampled) <= SCAN + Duration::from_secs(3)
}

fn refresh(
    candidates: &mut HashMap<u32, Candidate>,
    own_user: &[usize],
    root: &str,
    own_image: &str,
) -> io::Result<()> {
    let windows = minimized_windows()?;
    let now = Instant::now();
    candidates.retain(|pid, _| windows.get(pid) == Some(&true));
    // shortcut: sampled minimized windows plus low CPU approximate inactivity; use app-specific opt-in if background workloads need protection.
    for (pid, minimized) in windows {
        if !minimized || pid == std::process::id() {
            continue;
        }
        if let Some(candidate) = candidates.get_mut(&pid) {
            if let Ok(cpu) = cpu_time(candidate.handle.0)
                && unsafe { WaitForSingleObject(candidate.handle.0, 0) } == WAIT_TIMEOUT
            {
                let gap = now.duration_since(candidate.sampled);
                candidate.idle = gap <= SCAN + Duration::from_secs(3)
                    && cpu_idle(cpu.saturating_sub(candidate.cpu), gap);
                if !candidate.idle {
                    candidate.since = now;
                }
                candidate.cpu = cpu;
                candidate.sampled = now;
                continue;
            }
            candidates.remove(&pid);
        }
        if candidates.len() >= 32 {
            continue;
        }
        let attempt = (|| {
            let handle = system::open(
                pid,
                PROCESS_QUERY_INFORMATION
                    | PROCESS_VM_READ
                    | PROCESS_SET_QUOTA
                    | SYNCHRONIZATION_SYNCHRONIZE,
            )?;
            if !allowed(handle.0, own_user, root, own_image)? {
                return Err(io::Error::other("Excluded process"));
            }
            let cpu = cpu_time(handle.0)?;
            Ok(Candidate {
                handle,
                since: now,
                sampled: now,
                cpu,
                idle: false,
            })
        })();
        if let Ok(candidate) = attempt {
            candidates.insert(pid, candidate);
        }
    }
    Ok(())
}

fn round(
    candidates: &HashMap<u32, Candidate>,
    own_user: &[usize],
    root: &str,
    own_image: &str,
    stop: HANDLE,
) -> io::Result<String> {
    let before = system::memory()?;
    let mut successes = 0;
    let mut skipped = 0;
    let mut errors = 0;
    let mut details = String::new();
    for (pid, candidate) in candidates {
        if successes + errors >= 2 {
            break;
        }
        // Recheck pressure, cancellation and window state immediately before each request.
        if unsafe { WaitForSingleObject(stop, 0) } != WAIT_TIMEOUT
            || system::memory()?.used_percent() < 90.0
        {
            break;
        }
        let foreground = unsafe { GetForegroundWindow() };
        let mut active_pid = 0;
        if foreground.is_null()
            || unsafe { GetWindowThreadProcessId(foreground, &mut active_pid) } == 0
        {
            break;
        }
        if *pid == active_pid
            || !ready(candidate, Instant::now())
            || minimized_windows()?.get(pid) != Some(&true)
        {
            skipped += 1;
            continue;
        }
        let outcome: io::Result<String> = (|| {
            if !allowed(candidate.handle.0, own_user, root, own_image)? {
                return Err(io::Error::other("Identity no longer eligible"));
            }
            let usage = system::counters(candidate.handle.0)?;
            if usage.WorkingSetSize < MIN_WORKING_SET {
                return Ok(String::new());
            }
            // SAFETY: same retained, live, verified user-process handle with SET_QUOTA.
            if unsafe { K32EmptyWorkingSet(candidate.handle.0) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let after = system::counters(candidate.handle.0)
                .ok()
                .map(|c| format!("{:.1}", c.WorkingSetSize as f64 / MIB))
                .unwrap_or_else(|| "unavailable".into());
            Ok(format!(
                "pid={pid} working_set_MiB={:.1}->{after}\n",
                usage.WorkingSetSize as f64 / MIB
            ))
        })();
        match outcome {
            Ok(text) if text.is_empty() => skipped += 1,
            Ok(text) => {
                successes += 1;
                details.push_str(&text);
            }
            Err(error) => {
                errors += 1;
                details.push_str(&format!("pid={pid} skipped/error={error}\n"));
            }
        }
    }
    let after = system::memory()?;
    Ok(format!(
        "BACKGROUND EXPERIMENTAL api_success={successes} skipped={skipped} errors={errors}\n{details}available_MiB={:.1}->{:.1}; RAM_after={:.1}%\n{} No speed improvement is claimed.",
        before.available as f64 / MIB,
        after.available as f64 / MIB,
        after.used_percent(),
        if after.used_percent() >= 90.0 {
            "WARNING: RAM remains high; no repeated purge until rearmed."
        } else {
            "RAM below high threshold."
        }
    ))
}

fn status_path() -> io::Result<std::path::PathBuf> {
    Ok(app::log_path()?.with_file_name("background-status.txt"))
}

fn write_status(text: &str) -> io::Result<()> {
    let path = status_path()?;
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, text)
}

pub fn stop() -> io::Result<()> {
    // SAFETY: named local event, minimal right; the wrapper closes our handle.
    let event = checked_handle(unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, STOP_NAME) })?;
    if unsafe { SetEvent(event.0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn status() -> io::Result<()> {
    let _event = checked_handle(unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, STOP_NAME) })?;
    println!(
        "Running (live stop event found)\n{}",
        fs::read_to_string(status_path()?)?
    );
    Ok(())
}

pub fn run() -> io::Result<()> {
    // SAFETY: local named synchronization objects, default per-user security, owned handles.
    let raw = unsafe { CreateMutexW(null(), 0, w!("Local\\RamObserver-Background-Singleton-v1")) };
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let _singleton = checked_handle(raw)?;
    if already_running {
        return Ok(());
    }
    let stop = checked_handle(unsafe { CreateEventW(null(), 1, 0, STOP_NAME) })?;
    let own_process = unsafe { GetCurrentProcess() };
    if unsafe { SetPriorityClass(own_process, BELOW_NORMAL_PRIORITY_CLASS) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let own_user = token_user(own_process)?;
    let own_image = system::image_path(own_process)?.to_lowercase();
    let mut root = vec![0u16; 32768];
    let len = unsafe { GetWindowsDirectoryW(root.as_mut_ptr(), root.len() as u32) };
    if len == 0 || len as usize >= root.len() {
        return Err(io::Error::other("Windows directory unavailable"));
    }
    let root = format!(
        "{}\\",
        String::from_utf16_lossy(&root[..len as usize])
            .trim_end_matches('\\')
            .to_lowercase()
    );
    let started = Instant::now();
    let mut last_scan = started - SCAN;
    let mut last_status = started - Duration::from_secs(60);
    let mut candidates = HashMap::new();
    let mut policy = Policy::default();
    let mut last_result = "No working-set requests performed.".to_owned();
    write_status(&format!(
        "pid={} Starting quiet background mode.\n",
        std::process::id()
    ))?;
    app::append_log(&format!(
        "BACKGROUND START pid={}; poll=2s scan=10s below-normal-priority; experimental minimized-app scope",
        std::process::id()
    ))?;
    loop {
        if unsafe { WaitForSingleObject(stop.0, 0) } != WAIT_TIMEOUT {
            break;
        }
        let sample = system::memory();
        if let Ok(memory) = &sample {
            if memory.used_percent() < 85.0 {
                candidates.clear();
            } else if last_scan.elapsed() >= SCAN {
                if let Err(error) = refresh(&mut candidates, &own_user, &root, &own_image) {
                    candidates.clear();
                    last_result = format!("Window scan failed; round skipped: {error}");
                }
                last_scan = Instant::now();
            }
        }
        let now = Instant::now();
        let eligible = candidates.values().any(|c| ready(c, now));
        if policy.sample(
            started.elapsed(),
            sample.as_ref().ok().map(|m| m.used_percent()),
            eligible || policy.cooling_down(),
        ) {
            last_result = round(&candidates, &own_user, &root, &own_image, stop.0)
                .unwrap_or_else(|e| format!("Round incomplete: {e}; API outcome may be partial."));
            app::append_log(&last_result)?;
            policy.record_run(started.elapsed());
            last_status = started - Duration::from_secs(60);
        }
        if last_status.elapsed() >= Duration::from_secs(60) {
            let memory = match sample {
                Ok(m) => format!(
                    "RAM={:.1}% available_MiB={:.1}",
                    m.used_percent(),
                    m.available as f64 / MIB
                ),
                Err(e) => format!("Memory read error; automatic round skipped: {e}"),
            };
            write_status(&format!(
                "pid={} uptime_s={} {memory}\ntracked_minimized={}\n{}\n{last_result}\n",
                std::process::id(),
                started.elapsed().as_secs(),
                candidates.len(),
                policy.status()
            ))?;
            last_status = Instant::now();
        }
        match unsafe { WaitForSingleObject(stop.0, 2000) } {
            WAIT_TIMEOUT => {}
            WAIT_OBJECT_0 => break,
            _ => return Err(io::Error::last_os_error()),
        }
    }
    write_status("Stopped cleanly. No persistent system settings changed.\n")?;
    app::append_log("BACKGROUND STOP")
}

pub fn self_check() -> io::Result<()> {
    let process = unsafe { GetCurrentProcess() };
    let own_user = token_user(process)?;
    if !same_user(process, &own_user)? {
        return Err(io::Error::other("Own token SID mismatch"));
    }
    let own_image = system::image_path(process)?.to_lowercase();
    if !allowed(process, &own_user, "\\__not_a_windows_root__\\", "")?
        || allowed(process, &own_user, "\\__not_a_windows_root__\\", &own_image)?
    {
        return Err(io::Error::other("Process identity/protection guard failed"));
    }
    let _windows = minimized_windows()?;
    println!(
        "PASS: token SID, critical/protected-process checks, own-image exclusion and read-only window enumeration."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_cpu_units_and_zero_interval() {
        assert!(cpu_idle(1_000_000, Duration::from_secs(10)));
        assert!(!cpu_idle(1_000_001, Duration::from_secs(10)));
        assert!(!cpu_idle(0, Duration::ZERO));
    }

    #[test]
    fn eligibility_rejects_active_recent_and_stale_candidates() -> io::Result<()> {
        let now = Instant::now();
        // Only an owned unnamed event is needed to exercise readiness; no process is trimmed.
        let handle = checked_handle(unsafe { CreateEventW(null(), 1, 0, null()) })?;
        let mut candidate = Candidate {
            handle,
            since: now - MIN_IDLE,
            sampled: now - SCAN,
            cpu: 0,
            idle: true,
        };
        assert!(ready(&candidate, now));
        candidate.idle = false;
        assert!(!ready(&candidate, now));
        candidate.idle = true;
        candidate.since = now - Duration::from_secs(59);
        assert!(!ready(&candidate, now));
        candidate.since = now - MIN_IDLE;
        candidate.sampled = now - Duration::from_secs(14);
        assert!(!ready(&candidate, now));
        Ok(())
    }
}
