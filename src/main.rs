#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_os = "windows"))]
compile_error!("RAM Observer requires Windows 10/11 x64");

mod app;
mod background;
mod lab;
mod policy;
mod system;
mod ui;

fn main() {
    if let Err(error) = run() {
        eprintln!("RAM Observer: {error}");
        if std::env::args().nth(1).as_deref() == Some("--ui") {
            ui::show_error(&error.to_string());
        }
        std::process::exit(1);
    }
}

fn run() -> std::io::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--lab-child") => lab::child(),
        Some("--snapshot") => {
            let m = system::memory()?;
            println!(
                "RAM {:.2}% used, available {} bytes / total {} bytes; commit {} / {} bytes",
                m.used_percent(),
                m.available,
                m.total,
                m.commit,
                m.limit
            );
            let (rows, skipped) = system::processes()?;
            println!("Readable processes: {}; skipped: {skipped}", rows.len());
            Ok(())
        }
        Some("--self-check") => {
            background::self_check()?;
            let mut child = lab::Lab::start()?;
            if child.experiment(|| true).is_ok() {
                return Err(std::io::Error::other(
                    "Cancellation did not prevent trimming",
                ));
            }
            println!("{}", child.experiment(|| false)?);
            child.negative_checks()?;
            println!(
                "PASS: cancellation prevented trimming; restricted handle rejected; exited process rejected."
            );
            Ok(())
        }
        None | Some("--background") => background::run(),
        Some("--stop") => background::stop(),
        Some("--status") => background::status(),
        Some("--ui") => ui::run(),
        _ => Err(std::io::Error::other(
            "Supported arguments: --background, --stop, --status, --snapshot, --self-check, --ui",
        )),
    }
}
