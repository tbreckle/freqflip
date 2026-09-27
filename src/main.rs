// No console window on Windows when a frontend starts FreqFlip; see report::attach_console.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod backend;
mod cli;
mod display;
mod event_loop;
mod process;
mod report;
mod tray;

use clap::Parser;

use crate::cli::Args;
use crate::report::Reporter;

fn main() {
    let background = report::is_background();
    if !background {
        report::attach_console();
    }
    let args = Args::parse();

    if args.list {
        if let Err(e) = app::list(&args) {
            eprintln!("freqflip: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    if !args.foreground && !background {
        match report::spawn_background() {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                eprintln!("freqflip: {e:#}");
                std::process::exit(1);
            }
        }
    }

    // Errors go through the reporter, which hands them to the waiting parent.
    let (reporter, result) = match Reporter::new(args.log.as_deref()) {
        Ok(reporter) => {
            let result = app::run(&args, &reporter);
            (reporter, result)
        }
        Err(e) => (Reporter::new(None).expect("opens no file"), Err(e)),
    };
    let code = result.unwrap_or_else(|e| {
        reporter.fail(&e);
        1
    });
    std::process::exit(code);
}
