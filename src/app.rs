//! Switches the rate, waits for the process and restores the rate.

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::backend;
use crate::cli::Args;
use crate::display::{format_hz, pick_mode, select_display, Backend, Mode};
use crate::event_loop::{self, AppEvent, EventSender, Handler};
use crate::process::{self, Target, WatchEvent};
use crate::report::Reporter;
use crate::tray::{ProcessState, Status, Tray};

/// How often the current rate is read back for the tray.
const TICK: Duration = Duration::from_secs(2);
const RESTORE_ATTEMPTS: u32 = 3;
const RESTORE_RETRY_DELAY: Duration = Duration::from_millis(500);

/// `freqflip --list`
pub fn list(args: &Args) -> Result<()> {
    let mut backend = backend::open(args.backend)?;
    let displays = backend.displays()?;
    println!("Backend: {}", backend.name());
    if displays.is_empty() {
        println!("No active displays found.");
    }
    for display in displays {
        let rates = display
            .rates()
            .into_iter()
            .map(format_hz)
            .collect::<Vec<_>>()
            .join(", ");
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
        println!("    available: {rates}");
    }
    Ok(())
}

/// A switched display; restores the original mode when dropped, so every early return after
/// the switch puts the rate back.
struct Flip {
    backend: Box<dyn Backend>,
    display: String,
    original: Mode,
    restored: bool,
}

impl Flip {
    fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        if self
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
            if current.refresh != self.status.current {
                self.status.current = current.refresh;
                self.update_tray();
            }
        }
        true
    }
}

/// `freqflip HZ PROCESS`: returns the exit code.
pub fn run(args: &Args, reporter: &Reporter) -> Result<i32> {
    let hz = args.hz.context("no refresh rate given")?;
    let target = Target::parse(args.process.as_deref().context("no process given")?)?;
    process::check(&target)?;

    let mut backend = backend::open(args.backend)?;
    let display = select_display(backend.displays()?, args.display.as_deref())?;
    let mode = pick_mode(&display, hz, args.tolerance)?;
    let original = display.current.clone();
    if mode == original {
        reporter.ready(&format!(
            "{} already runs at {}, nothing to do",
            display.label(),
            format_hz(original.refresh)
        ));
        return Ok(0);
    }

    backend.set_mode(&display.id, &mode).with_context(|| {
        format!(
            "could not switch {} to {}",
            display.label(),
            format_hz(mode.refresh)
        )
    })?;
    let mut flip = Flip {
        backend,
        display: display.id.clone(),
        original: original.clone(),
        restored: false,
    };
    let current = flip
        .backend
        .current_mode(&display.id)
        .map(|m| m.refresh)
        .unwrap_or(mode.refresh);
    reporter.ready(&format!(
        "{}: {} -> {}, restoring when {} exits",
        display.label(),
        format_hz(original.refresh),
        format_hz(current),
        target.label()
    ));

    let status = Status {
        display: display.id.clone(),
        target: mode.refresh,
        original: original.refresh,
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
    reporter.info(&format!(
        "{}, restoring {}",
        app.reason,
        format_hz(app.status.original)
    ));
    match app.flip.restore() {
        Ok(()) => {
            reporter.info("restored");
            Ok(0)
        }
        Err(e) => {
            let e = e.context(format!(
                "could not restore {} on {}",
                format_hz(app.status.original),
                app.status.display
            ));
            reporter.fail(&e);
            Ok(1)
        }
    }
}
