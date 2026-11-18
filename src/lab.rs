use crate::system::{self, MIB};
use std::{
    hint::black_box,
    io::{self, BufRead, BufReader, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::{ProcessStatus::K32EmptyWorkingSet, Threading::*},
};

// Only a child created here is eligible. No API accepts a user-supplied PID.
pub struct Lab {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<io::Result<String>>,
}

impl Lab {
    pub fn start() -> io::Result<Self> {
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--lab-child")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("No child stdout"))?;
        let (tx, output) = mpsc::sync_channel(1);
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut lab = Self {
            child,
            input,
            output,
        };
        lab.probe()?;
        Ok(lab)
    }

    fn handle(&self) -> HANDLE {
        self.child.as_raw_handle() as HANDLE
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    fn probe(&mut self) -> io::Result<u128> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| io::Error::other("Child input closed"))?;
        writeln!(input, "touch")?;
        input.flush()?;
        let line = self
            .output
            .recv_timeout(Duration::from_secs(5))
            .map_err(io::Error::other)??;
        line.trim().parse().map_err(io::Error::other)
    }

    pub fn experiment(&mut self, cancelled: impl Fn() -> bool) -> io::Result<String> {
        if self.child.try_wait()?.is_some() {
            return Err(io::Error::other("Test process exited"));
        }
        let baseline = self.probe()?;
        let before = system::counters(self.handle())?;
        let ram_before = system::memory()?;
        if cancelled() {
            return Err(io::Error::other("Stopped before working-set request"));
        }
        // SAFETY: this is the retained OS handle of our child, never a reopened PID.
        if unsafe { K32EmptyWorkingSet(self.handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let trimmed = system::counters(self.handle())?;
        let io_before = self.io_counters()?;
        let resume = self.probe()?;
        let io_after = self.io_counters()?;
        let resumed = system::counters(self.handle())?;
        let ram_after = system::memory()?;
        Ok(format!(
            "LAB ONLY PID {} | API success=1 skipped=0 errors=0\r\nWorking set: {:.1} -> {:.1} -> {:.1} MiB (before / trimmed / touched)\r\nAvailable RAM: {:.1} -> {:.1} MiB (system activity also affects this)\r\nTouch time: {} -> {} us; page faults on resume: {} (includes soft faults)\r\nRead I/O on resume: {} bytes (process I/O; not a hard-fault/disk latency measurement)\r\n{} Synthetic results do not prove a benefit for external applications.",
            self.pid(),
            before.WorkingSetSize as f64 / MIB,
            trimmed.WorkingSetSize as f64 / MIB,
            resumed.WorkingSetSize as f64 / MIB,
            ram_before.available as f64 / MIB,
            ram_after.available as f64 / MIB,
            baseline,
            resume,
            resumed.PageFaultCount.wrapping_sub(trimmed.PageFaultCount),
            io_after
                .ReadTransferCount
                .saturating_sub(io_before.ReadTransferCount),
            if resume > baseline {
                "Resume was slower in this sample."
            } else {
                "One sample does not prove a performance benefit."
            }
        ))
    }

    fn io_counters(&self) -> io::Result<IO_COUNTERS> {
        // SAFETY: initialized API output with the retained handle of our child.
        unsafe {
            let mut counters: IO_COUNTERS = std::mem::zeroed();
            if GetProcessIoCounters(self.handle(), &mut counters) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(counters)
        }
    }

    pub fn negative_checks(&mut self) -> io::Result<()> {
        let restricted = system::open(self.pid(), PROCESS_QUERY_LIMITED_INFORMATION)?;
        // SAFETY: deliberately request an operation lacking PROCESS_SET_QUOTA on our own child.
        if unsafe { K32EmptyWorkingSet(restricted.0) } != 0 {
            return Err(io::Error::other("Restricted handle unexpectedly trimmed"));
        }
        self.input.take();
        self.child.wait()?;
        if self.experiment(|| false).is_ok() {
            return Err(io::Error::other("Exited process unexpectedly readable"));
        }
        Ok(())
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        // EOF asks only our test child to exit; never terminate another application.
        self.input.take();
        let _ = self.child.try_wait();
    }
}

pub fn child() -> io::Result<()> {
    let mut data = vec![0u8; 64 * 1024 * 1024];
    let mut output = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        if line? != "touch" {
            return Err(io::Error::other("Invalid lab command"));
        }
        let start = Instant::now();
        for i in (0..data.len()).step_by(4096) {
            // SAFETY: i is in bounds; volatile accesses force real page touches.
            unsafe {
                let p = data.as_mut_ptr().add(i);
                p.write_volatile(p.read_volatile().wrapping_add(1));
            }
        }
        black_box(&data);
        writeln!(output, "{}", start.elapsed().as_micros())?;
        output.flush()?;
    }
    Ok(())
}
