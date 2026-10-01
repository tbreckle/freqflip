//! Switches the mode, waits for the process and restores the mode.

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::backend;
use crate::cli::Args;
use crate::display::{format_hz, format_mode, pick_mode, select_display, Backend, Mode};
use crate::event_loop::{self, AppEvent, EventSender, Handler};
use crate::process::{self, Target, WatchEvent};
use crate::report::Reporter;
use crate::tray::{ProcessState, Status, Tray};

/// How often the current mode is read back for the tray.
const TICK: Duration = Duration::from_secs(2);
const RESTORE_ATTEMPTS: u32 = 3;
const RESTORE_RETRY_DELAY: Duration = Duration::from_millis(500);

/// `freqflip --list`
pub fn list(args: &Args) -> Result<()> {
    let mut backend = backend::open(args.backend, args.save)?;
    let displays = backend.displays()?;
    println!("Backend: {}", backend.name());
    if displays.is_empty() {
        println!("No active displays found.");
    }
    for display in displays {
        println!();
        println!(
            "{} {}{}",
            if display.primary { "*" } else { " " },
            display.label(),
            if display.primary { " [primary]" } else { "" }
        );
        println!(
            "    current:   {} @ {}",
            display.current.resolution(),
            format_hz(display.current.refresh)
        );
        let current = display.current.resolution();
        let rows: Vec<(String, Vec<f64>)> = display
            .resolutions()
            .into_iter()
            .map(|resolution| {
                // At the current resolution, only what can be picked without a resolution.
                let rates = if resolution == current {
                    display.rates()
                } else {
                    display.rates_at(resolution)
                };
                (resolution.to_string(), rates)
            })
            .collect();
        let width = rows.iter().map(|(r, _)| r.len()).max().unwrap_or(0);
        for (i, (resolution, rates)) in rows.iter().enumerate() {
            let rates = rates
                .iter()
                .map(|&hz| format_hz(hz))
                .collect::<Vec<_>>()
                .join(", ");
            let label = if i == 0 { "available:" } else { "" };
            println!("    {label:<10} {resolution:<width$}   {rates}");
        }
    }
    Ok(())
}

/// A switched display; restores the original mode when dropped, so every early return after
/// the switch puts the mode back.
struct Flip {
    backend: Box<dyn Backend>,
    display: String,
    original: Mode,
    /// The mode was saved (`--save`), so the original has to be saved back even when the
    /// display already runs at it again.
    saved: bool,
    restored: bool,
}

impl Flip {
    fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        if !self.saved
            && self
                .backend
                .current_mode(&self.display)
                .is_ok_and(|current| current == self.original)
        {
            return Ok(());
        }
        let mut attempt = 1;
        loop {
            match self.backend.set_mode(&self.display, &self.original) {
                Ok(()) => return Ok(()),
                // The display may be busy while the game tears down its fullscreen window.
                Err(_) if attempt < RESTORE_ATTEMPTS => {
                    attempt += 1;
                    thread::sleep(RESTORE_RETRY_DELAY);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for Flip {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

struct App {
    flip: Flip,
    status: Status,
    show_tray: bool,
    tray: Option<Tray>,
    reporter: Reporter,
    target: Target,
    start_timeout: u64,
    /// Why the loop ended.
    reason: String,
}

impl App {
    fn update_tray(&self) {
        if let Some(tray) = &self.tray {
            tray.update(&self.status);
        }
    }
}

impl Handler for App {
    fn start(&mut self) {
        if !self.show_tray {
            return;
        }
        match Tray::new(&self.status) {
            Ok(tray) => self.tray = Some(tray),
            Err(e) => self
                .reporter
                .info(&format!("no tray icon, running without one: {e:#}")),
        }
    }

    fn event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::Menu(id) => {
                if self.tray.as_ref().is_some_and(|t| t.is_exit(&id)) {
                    self.reason = "exit chosen from the tray menu".into();
                    return false;
                }
                true
            }
            AppEvent::Watch(WatchEvent::Found { pid, name }) => {
                self.reporter
                    .info(&format!("{name} (PID {pid}) is running"));
                self.status.process = ProcessState::Running { name, pid };
                self.update_tray();
                true
            }
            AppEvent::Watch(WatchEvent::Exited) => {
                self.reason = format!("{} exited", self.target.label());
                false
            }
            AppEvent::Watch(WatchEvent::NeverStarted) => {
                self.reason = format!(
                    "{} did not start within {} s",
                    self.target.label(),
                    self.start_timeout
                );
                false
            }
            AppEvent::Terminate => {
                self.reason = "terminated".into();
                false
            }
        }
    }

    fn tick(&mut self) -> bool {
        if let Ok(current) = self.flip.backend.current_mode(&self.flip.display) {
            if current != self.status.current {
                self.status.current = current;
                self.update_tray();
            }
        }
        true
    }
}

/// `freqflip MODE PROCESS`: returns the exit code.
pub fn run(args: &Args, reporter: &Reporter) -> Result<i32> {
    let request = args.mode.context("no mode given")?;
    let target = Target::parse(args.process.as_deref().context("no process given")?)?;
    process::check(&target)?;

    let mut backend = backend::open(args.backend, args.save)?;
    let display = select_display(backend.displays()?, args.display.as_deref())?;
    let mode = pick_mode(&display, &request, args.tolerance)?;
    let original = display.current.clone();
    // Resolutions only show up in the messages when they change.
    let resolution = mode.resolution() != original.resolution();
    if mode == original {
        reporter.ready(&format!(
            "{} already runs at {}, nothing to do",
            display.label(),
            format_mode(&original, request.resolution.is_some())
        ));
        return Ok(0);
    }

    backend.set_mode(&display.id, &mode).with_context(|| {
        format!(
            "could not switch {} to {}",
            display.label(),
            format_mode(&mode, resolution)
        )
    })?;
    let mut flip = Flip {
        backend,
        display: display.id.clone(),
        original: original.clone(),
        saved: args.save,
        restored: false,
    };
    let current = flip
        .backend
        .current_mode(&display.id)
        .unwrap_or_else(|_| mode.clone());
    reporter.ready(&format!(
        "{}: {} -> {}, restoring when {} exits",
        display.label(),
        format_mode(&original, resolution),
        format_mode(&current, resolution),
        target.label()
    ));

    let status = Status {
        display: display.id.clone(),
        target: mode,
        original,
        current,
        process: match &target {
            Target::Name(name) => ProcessState::Waiting(name.clone()),
            Target::Pid(pid) => ProcessState::Running {
                name: target.label(),
                pid: *pid,
            },
        },
    };
    let start_timeout = (args.start_timeout > 0).then(|| Duration::from_secs(args.start_timeout));

    let mut app = event_loop::run(TICK, |sender: EventSender| {
        let watch_sender = sender.clone();
        process::watch(target.clone(), start_timeout, move |event| {
            watch_sender.send(AppEvent::Watch(event));
        })?;
        if let Err(e) = ctrlc::set_handler(move || sender.send(AppEvent::Terminate)) {
            reporter.info(&format!("could not install the signal handler: {e}"));
        }
        Ok(App {
            flip,
            status,
            show_tray: !args.no_tray,
            tray: None,
            reporter: reporter.clone(),
            target: target.clone(),
            start_timeout: args.start_timeout,
            reason: "event loop ended".into(),
        })
    })?;

    app.tray = None;
    let original = format_mode(&app.status.original, resolution);
    reporter.info(&format!("{}, restoring {original}", app.reason));
    match app.flip.restore() {
        Ok(()) => {
            reporter.info("restored");
            Ok(0)
        }
        Err(e) => {
            let e = e.context(format!(
                "could not restore {original} on {}",
                app.status.display
            ));
            reporter.fail(&e);
            Ok(1)
        }
    }
}
