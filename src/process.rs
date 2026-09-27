//! Finds the process to wait for and watches it on a background thread.

use std::ffi::OsStr;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use sysinfo::{
    Pid, Process, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind,
};

const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// A name-matched process has to be gone this long before it counts as exited, so launchers
/// that restart the game right away don't flip the rate back in between.
const EXIT_GRACE: Duration = Duration::from_millis(1500);

/// What to wait for, parsed from the `PROCESS` argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Pid(u32),
    /// Process name, matched case-insensitively and without `.exe`.
    Name(String),
}

impl Target {
    pub fn parse(arg: &str) -> Result<Target> {
        let arg = arg.trim();
        if arg.is_empty() {
            bail!("the process name must not be empty");
        }
        if let Ok(pid) = arg.parse::<u32>() {
            return Ok(Target::Pid(pid));
        }
        // Accept a path, but match only its file name.
        let name = Path::new(arg)
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or(arg);
        Ok(Target::Name(name.to_string()))
    }

    pub fn label(&self) -> String {
        match self {
            Target::Pid(pid) => format!("PID {pid}"),
            Target::Name(name) => name.clone(),
        }
    }
}

/// Progress reports of the watcher thread.
#[derive(Clone, Debug, PartialEq)]
pub enum WatchEvent {
    /// The process is running (reported once).
    Found { pid: u32, name: String },
    /// The process has exited.
    Exited,
    /// A named process didn't show up within the start timeout.
    NeverStarted,
}

/// Normalizes a process or query name for comparison: lower case, without `.exe`.
fn normalize(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

/// True if `query` names the process called `candidate` (a name or path).
pub fn name_matches(query: &str, candidate: &OsStr) -> bool {
    let candidate = Path::new(candidate)
        .file_name()
        .unwrap_or(candidate)
        .to_string_lossy();
    !candidate.is_empty() && normalize(query) == normalize(&candidate)
}

fn process_matches(query: &str, process: &Process) -> bool {
    // Linux truncates process names to 15 characters, so also look at the executable and argv[0].
    name_matches(query, process.name())
        || process
            .exe()
            .is_some_and(|exe| name_matches(query, exe.as_os_str()))
        || process
            .cmd()
            .first()
            .is_some_and(|arg0| name_matches(query, arg0))
}

/// Zombies have exited already, they only wait for their parent to reap them.
fn running(process: &Process) -> bool {
    process.status() != ProcessStatus::Zombie
}

fn refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
}

/// Finds running processes named `query`, lowest PID first (never this process itself).
fn find_by_name(system: &mut System, query: &str) -> Vec<(u32, String)> {
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind());
    let own = std::process::id();
    let mut found: Vec<(u32, String)> = system
        .processes()
        .values()
        .filter(|p| p.pid().as_u32() != own && running(p) && process_matches(query, p))
        .map(|p| (p.pid().as_u32(), p.name().to_string_lossy().into_owned()))
        .collect();
    found.sort();
    found
}

/// A running process identified by PID; the start time guards against PID reuse.
struct Tracked {
    pid: Pid,
    start_time: u64,
}

impl Tracked {
    fn alive(&self, system: &mut System) -> bool {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        system
            .process(self.pid)
            .is_some_and(|p| p.start_time() == self.start_time && running(p))
    }
}

/// Checks the target before the rate is switched: a PID has to exist already.
pub fn check(target: &Target) -> Result<()> {
    if let Target::Pid(pid) = target {
        let mut system = System::new();
        let pid = Pid::from_u32(*pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        if system.process(pid).is_none() {
            bail!("no process with PID {pid} is running");
        }
    }
    Ok(())
}

/// Watches `target` on a background thread and reports through `report` until it has exited.
/// `start_timeout` (None = forever) limits how long a named process may take to show up.
pub fn watch<F>(target: Target, start_timeout: Option<Duration>, report: F) -> Result<()>
where
    F: Fn(WatchEvent) + Send + 'static,
{
    thread::Builder::new()
        .name("process-watcher".into())
        .spawn(move || {
            let event = match target {
                Target::Pid(pid) => watch_pid(pid, &report),
                Target::Name(name) => watch_name(&name, start_timeout, &report),
            };
            report(event);
        })?;
    Ok(())
}

fn watch_pid(pid: u32, report: &dyn Fn(WatchEvent)) -> WatchEvent {
    let mut system = System::new();
    let pid = Pid::from_u32(pid);
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, refresh_kind());
    let Some(process) = system.process(pid) else {
        return WatchEvent::Exited;
    };
    report(WatchEvent::Found {
        pid: pid.as_u32(),
        name: process.name().to_string_lossy().into_owned(),
    });
    let tracked = Tracked {
        pid,
        start_time: process.start_time(),
    };
    while tracked.alive(&mut system) {
        thread::sleep(POLL_INTERVAL);
    }
    WatchEvent::Exited
}

fn watch_name(
    query: &str,
    start_timeout: Option<Duration>,
    report: &dyn Fn(WatchEvent),
) -> WatchEvent {
    let mut system = System::new();
    let started = Instant::now();

    // Wait for the process to show up.
    let (pid, name) = loop {
        if let Some(first) = find_by_name(&mut system, query).into_iter().next() {
            break first;
        }
        if start_timeout.is_some_and(|timeout| started.elapsed() >= timeout) {
            return WatchEvent::NeverStarted;
        }
        thread::sleep(POLL_INTERVAL);
    };
    report(WatchEvent::Found { pid, name });

    // Wait until no process with that name is left for EXIT_GRACE.
    let mut gone_since: Option<Instant> = None;
    loop {
        thread::sleep(POLL_INTERVAL);
        if find_by_name(&mut system, query).is_empty() {
            let since = *gone_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= EXIT_GRACE {
                return WatchEvent::Exited;
            }
        } else {
            gone_since = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn parses_targets() {
        assert_eq!(Target::parse("1234").unwrap(), Target::Pid(1234));
        assert_eq!(Target::parse("mame").unwrap(), Target::Name("mame".into()));
        assert_eq!(
            Target::parse("/usr/games/mame").unwrap(),
            Target::Name("mame".into())
        );
        assert_eq!(
            Target::parse(" Supermodel.exe ").unwrap(),
            Target::Name("Supermodel.exe".into())
        );
        assert!(Target::parse("  ").is_err());
    }

    #[test]
    fn matches_names() {
        assert!(name_matches("mame", OsStr::new("mame")));
        assert!(name_matches("MAME", OsStr::new("mame.exe")));
        assert!(name_matches("mame.exe", OsStr::new("Mame")));
        assert!(name_matches("mame", OsStr::new("/usr/bin/mame")));
        assert!(!name_matches("mame", OsStr::new("mame64")));
        assert!(!name_matches("mame", OsStr::new("")));
    }

    #[test]
    fn missing_pid_fails_check() {
        // PIDs are far below u32::MAX on every supported OS.
        assert!(check(&Target::Pid(u32::MAX - 1)).is_err());
        assert!(check(&Target::Pid(std::process::id())).is_ok());
        assert!(check(&Target::Name("anything".into())).is_ok());
    }

    #[test]
    fn watches_a_child_process() {
        #[cfg(windows)]
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 2 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        #[cfg(not(windows))]
        let mut child = std::process::Command::new("sleep")
            .arg("1")
            .spawn()
            .unwrap();

        let (tx, rx) = mpsc::channel();
        watch(Target::Pid(child.id()), None, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(first, WatchEvent::Found { pid, .. } if pid == child.id()));
        child.wait().unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            WatchEvent::Exited
        );
    }

    #[test]
    fn named_process_that_never_starts_times_out() {
        let (tx, rx) = mpsc::channel();
        watch(
            Target::Name("freqflip-no-such-process".into()),
            Some(Duration::from_millis(200)),
            move |e| {
                let _ = tx.send(e);
            },
        )
        .unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            WatchEvent::NeverStarted
        );
    }
}
