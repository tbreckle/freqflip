//! Progress output and detaching into the background.
//!
//! Without `--foreground`, FreqFlip starts a copy of itself in the background and waits until
//! that copy reports over its stdout that the rate is switched (or why it isn't). So the
//! caller, e.g. a frontend's launch script, gets a proper exit code and error message and can
//! go on to start the game while the background copy waits for it to exit.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};

/// Set in the environment of the background copy.
const BACKGROUND_ENV: &str = "FREQFLIP_BACKGROUND";

pub fn is_background() -> bool {
    std::env::var_os(BACKGROUND_ENV).is_some()
}

/// Where messages go: the terminal (foreground), the pipe to the waiting parent (background,
/// until ready) and an optional log file.
#[derive(Clone)]
pub struct Reporter(Arc<Mutex<Inner>>);

struct Inner {
    background: bool,
    /// The parent still reads our stdout.
    attached: bool,
    log: Option<File>,
}

impl Reporter {
    pub fn new(log: Option<&Path>) -> Result<Self> {
        let log = log
            .map(|path| {
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .with_context(|| format!("could not open log file {}", path.display()))
            })
            .transpose()?;
        let background = is_background();
        Ok(Self(Arc::new(Mutex::new(Inner {
            background,
            attached: background,
            log,
        }))))
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut inner)
    }

    /// A progress message: shown in the terminal while there is one, always logged.
    pub fn info(&self, msg: &str) {
        self.with(|inner| {
            inner.log(msg);
            inner.emit("info", msg);
        });
    }

    /// The rate is switched (or nothing needs to be done): the caller may continue.
    pub fn ready(&self, msg: &str) {
        self.with(|inner| {
            inner.log(msg);
            inner.emit("ready", msg);
            inner.attached = false;
        });
    }

    /// FreqFlip failed.
    pub fn fail(&self, err: &anyhow::Error) {
        let msg = format!("{err:#}");
        self.with(|inner| {
            inner.log(&format!("error: {msg}"));
            if inner.background && inner.attached {
                inner.emit("error", &msg);
                inner.attached = false;
            } else if !inner.background {
                eprintln!("freqflip: {msg}");
            }
        });
    }
}

impl Inner {
    fn emit(&mut self, kind: &str, msg: &str) {
        if !self.background {
            println!("{msg}");
        } else if self.attached {
            let mut out = std::io::stdout().lock();
            let line = msg.replace('\n', " ");
            if writeln!(out, "{kind} {line}")
                .and_then(|()| out.flush())
                .is_err()
            {
                self.attached = false;
            }
        }
    }

    fn log(&mut self, msg: &str) {
        if let Some(file) = &mut self.log {
            let secs = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(file, "[{secs}] [{}] {msg}", std::process::id());
        }
    }
}

/// Starts the background copy with the same arguments and relays its messages until it is
/// ready. Returns the exit code for this process.
pub fn spawn_background() -> Result<i32> {
    let exe = std::env::current_exe().context("could not find the freqflip executable")?;
    let mut command = Command::new(exe);
    command
        .args(std::env::args_os().skip(1))
        .env(BACKGROUND_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    detach(&mut command);
    let mut child = command
        .spawn()
        .context("could not start FreqFlip in the background")?;
    let stdout = child
        .stdout
        .take()
        .context("no pipe to the background process")?;

    for line in BufReader::new(stdout).lines() {
        let line = line?;
        let (kind, msg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match kind {
            "info" => println!("{msg}"),
            "ready" => {
                println!("{msg}");
                return Ok(0);
            }
            "error" => {
                eprintln!("freqflip: {msg}");
                let _ = child.wait();
                return Ok(1);
            }
            _ => {}
        }
    }
    let status = child.wait()?;
    bail!("the background process exited unexpectedly ({status})")
}

/// Detaches the background copy from the terminal session, so closing the terminal or
/// pressing Ctrl+C there doesn't stop it before the rate is restored.
#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe and touches no memory of the parent.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

/// FreqFlip is a GUI-subsystem app on Windows (no console window pops up when a frontend
/// starts it). When it's run from a console, attach to it so messages and --help show up.
#[cfg(windows)]
pub fn attach_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // SAFETY: plain Win32 call; failing just means there is no console to attach to.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
pub fn attach_console() {}
