use crate::{
    lab::Lab,
    policy::Policy,
    system::{self, MIB},
};
use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub enum Command {
    Experiment,
}

pub struct Shared {
    pub text: Mutex<String>,
    pub auto: AtomicBool,
    pub stop: AtomicBool,
    pub busy: AtomicBool,
    pub commands: SyncSender<Command>,
}

pub fn start() -> Arc<Shared> {
    let (tx, rx) = mpsc::sync_channel(1);
    let shared = Arc::new(Shared {
        text: Mutex::new("Reading memory...".into()),
        auto: AtomicBool::new(false),
        stop: AtomicBool::new(false),
        busy: AtomicBool::new(false),
        commands: tx,
    });
    let worker = shared.clone();
    thread::spawn(move || run(worker, rx));
    shared
}

pub(crate) fn log_path() -> io::Result<PathBuf> {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("LOCALAPPDATA unavailable; logging disabled"))?;
    Ok(root.join("RamObserver").join("events.log"))
}

pub(crate) fn append_log(message: &str) -> io::Result<()> {
    let path = log_path()?;
    append_log_at(&path, message)
}

fn append_log_at(path: &std::path::Path, message: &str) -> io::Result<()> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let entry = format!("unix_seconds={timestamp}\n{message}\n");
    if entry.len() > 1024 * 1024 {
        return Err(io::Error::other("Log entry exceeds 1 MiB"));
    }
    fs::create_dir_all(path.parent().unwrap())?;
    let backup = path.with_extension("previous.log");
    if path.try_exists()? && fs::metadata(path)?.len() + entry.len() as u64 > 1024 * 1024 {
        if backup.try_exists()? {
            fs::remove_file(&backup)?;
        }
        fs::rename(path, backup)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(entry.as_bytes())
}

fn run(shared: Arc<Shared>, rx: Receiver<Command>) {
    let start = Instant::now();
    let mut policy = Policy::default();
    let mut history = VecDeque::new();
    let mut lab: Option<Lab> = None;
    let mut result =
        "No reclamation performed. This UI test mode never trims external applications.".to_owned();
    let mut process_text = String::new();
    let mut last_process_read = Instant::now() - Duration::from_secs(5);
    while !shared.stop.load(Ordering::SeqCst) {
        let now = start.elapsed();
        let sample = system::memory();
        let enabled = shared.auto.load(Ordering::SeqCst);
        let trigger = policy.sample(
            now,
            sample.as_ref().ok().map(|m| m.used_percent()),
            enabled && lab.is_some(),
        );
        let manual = rx.try_recv().is_ok();
        if manual || trigger {
            shared.busy.store(true, Ordering::SeqCst);
            let action = (|| {
                sample
                    .as_ref()
                    .map_err(|e| io::Error::other(e.to_string()))?;
                if shared.stop.load(Ordering::SeqCst) {
                    return Err(io::Error::other("Stopped before round"));
                }
                if lab.is_none() {
                    lab = Some(Lab::start()?);
                }
                lab.as_mut().unwrap().experiment(|| {
                    shared.stop.load(Ordering::SeqCst)
                        || (trigger && !shared.auto.load(Ordering::SeqCst))
                })
            })();
            result = match action {
                Ok(text) => text,
                Err(e) => format!("Round incomplete: {e}; API outcome unconfirmed; errors=1"),
            };
            policy.record_run(start.elapsed());
            if let Err(e) = append_log(&result) {
                result.push_str(&format!("\r\nLOG ERROR: {e}"));
            }
            shared.busy.store(false, Ordering::SeqCst);
        }
        if last_process_read.elapsed() >= Duration::from_secs(5) {
            process_text = match system::processes() {
                Ok((rows, skipped)) => {
                    let mut text = format!(
                        "PROCESSES - working set; shared pages may be counted more than once\r\n{} readable; {} inaccessible / exited (skipped). All excluded from trimming.\r\n",
                        rows.len(),
                        skipped
                    );
                    for row in rows.iter().take(30) {
                        text.push_str(&format!(
                            "{:>7}   {:>9.1} MiB   {}\r\n",
                            row.pid,
                            row.working_set as f64 / MIB,
                            row.name
                        ));
                    }
                    text.push_str("Top 30 shown; refreshed every 5 seconds.\r\n");
                    text
                }
                Err(e) => format!("Process list unavailable: {e}\r\n"),
            };
            last_process_read = Instant::now();
        }
        // Read again after an experiment; never label a pre-trim sample as its result.
        let latest = if manual || trigger {
            system::memory()
        } else {
            sample
        };
        let memory_text = match latest {
            Ok(m) => {
                history.push_back(Some(m.used_percent()));
                let warning = if m.used_percent() >= 90.0 {
                    "WARNING: RAM is still >=90%. Reducing working sets does not fix allocations or memory leaks."
                } else {
                    "RAM below warning threshold. No performance benefit is claimed."
                };
                format!(
                    "PHYSICAL RAM: {:.1}% used | available {:.1} / total {:.1} MiB\r\nCOMMIT: {:.1} / limit {:.1} MiB (separate from physical RAM)\r\n{}\r\n",
                    m.used_percent(),
                    m.available as f64 / MIB,
                    m.total as f64 / MIB,
                    m.commit as f64 / MIB,
                    m.limit as f64 / MIB,
                    warning
                )
            }
            Err(e) => {
                history.push_back(None);
                format!("MEMORY READ ERROR: {e}. Sample unavailable; automatic round skipped.\r\n")
            }
        };
        if history.len() > 60 {
            history.pop_front();
        }
        let levels: Vec<char> = "▁▂▃▄▅▆▇█".chars().collect();
        let trend: String = history
            .iter()
            .map(|v| v.map_or('?', |v| levels[((v / 100.0 * 7.0).round() as usize).min(7)]))
            .collect();
        let mode = if !enabled {
            "OFF"
        } else if lab.is_none() {
            "WAITING: run controlled test first"
        } else {
            "ON: owned 64 MiB test child ONLY"
        };
        let text = format!(
            "RAM OBSERVER | experimental Windows memory monitor\r\n\r\n{memory_text}\r\nTREND (last 60 samples, ~1s each; ? = missing):\r\n{trend}\r\n\r\nAUTO: {mode}\r\nThreshold >=90% for 10s; cooldown 120s; rearm <85% for 10s after cooldown.\r\n{}\r\n\r\nLAST ROUND\r\n{result}\r\n\r\n{process_text}\r\nLogs: %LOCALAPPDATA%\\RamObserver\\events.log (1 MiB + one backup).\r\nNo startup task, administrator elevation, page-file changes, or network telemetry.",
            policy.status()
        );
        if let Ok(mut value) = shared.text.lock() {
            *value = text;
        }
        for _ in 0..10 {
            if shared.stop.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_rotate_and_preserve_the_last_round() -> io::Result<()> {
        let dir =
            std::env::temp_dir().join(format!("ram-observer-log-test-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let path = dir.join("events.log");
        fs::write(&path, vec![b'x'; 1024 * 1024])?;
        append_log_at(&path, "controlled round")?;
        assert_eq!(
            fs::metadata(path.with_extension("previous.log"))?.len(),
            1024 * 1024
        );
        assert!(fs::read_to_string(&path)?.contains("controlled round"));
        assert!(append_log_at(&path, &"x".repeat(1024 * 1024)).is_err());
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
