//! Display backends, one per platform (several on Linux).

use anyhow::Result;

use crate::cli::BackendKind;
use crate::display::Backend;

#[cfg(target_os = "linux")]
mod gnome;
#[cfg(target_os = "linux")]
mod kde;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod wlr;
#[cfg(target_os = "linux")]
mod x11;

/// Opens the display backend for this platform. `kind` only matters on Linux, `save` (store the
/// mode in the display settings) only on Windows.
#[cfg(target_os = "linux")]
pub fn open(kind: BackendKind, _save: bool) -> Result<Box<dyn Backend>> {
    use anyhow::bail;
    use std::env;

    let kind = match kind {
        BackendKind::Auto => detect(
            env::var_os("WAYLAND_DISPLAY").is_some(),
            env::var_os("DISPLAY").is_some(),
            &env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        )?,
        kind => kind,
    };
    Ok(match kind {
        BackendKind::X11 => Box::new(x11::X11::open()?),
        BackendKind::Gnome => Box::new(gnome::Gnome::open()?),
        BackendKind::Kde => Box::new(kde::Kde::open()?),
        BackendKind::Wlr => Box::new(wlr::Wlr::open()?),
        BackendKind::Auto => bail!("no display backend found"),
    })
}

/// Picks the Linux backend for the session. On Wayland, XRandR only sees XWayland's virtual
/// outputs, so the compositor has to be asked instead.
#[cfg(target_os = "linux")]
fn detect(wayland: bool, x11: bool, desktop: &str) -> Result<BackendKind> {
    let desktop = desktop.to_lowercase();
    let has = |name: &str| desktop.split(':').any(|d| d == name);
    if wayland {
        if has("gnome") || has("ubuntu") || has("unity") {
            Ok(BackendKind::Gnome)
        } else if has("kde") {
            Ok(BackendKind::Kde)
        } else {
            Ok(BackendKind::Wlr)
        }
    } else if x11 {
        Ok(BackendKind::X11)
    } else {
        anyhow::bail!("no graphical session found (neither WAYLAND_DISPLAY nor DISPLAY is set)")
    }
}

#[cfg(target_os = "windows")]
pub fn open(_kind: BackendKind, save: bool) -> Result<Box<dyn Backend>> {
    Ok(Box::new(windows::Windows::new(save)))
}

#[cfg(target_os = "macos")]
pub fn open(_kind: BackendKind, _save: bool) -> Result<Box<dyn Backend>> {
    Ok(Box::new(macos::MacOs))
}

/// Runs a command line tool and returns its stdout, with stderr in the error.
#[cfg(target_os = "linux")]
fn run_tool(program: &str, args: &[&str]) -> Result<String> {
    use anyhow::{bail, Context};
    use std::process::{Command, Stdio};

    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("could not run {program} (is it installed?)"))?;
    if !output.status.success() {
        bail!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn detects_linux_backend() {
        assert_eq!(detect(false, true, "X-Cinnamon").unwrap(), BackendKind::X11);
        assert_eq!(detect(true, true, "GNOME").unwrap(), BackendKind::Gnome);
        assert_eq!(
            detect(true, true, "ubuntu:GNOME").unwrap(),
            BackendKind::Gnome
        );
        assert_eq!(detect(true, true, "KDE").unwrap(), BackendKind::Kde);
        assert_eq!(detect(true, false, "sway").unwrap(), BackendKind::Wlr);
        assert_eq!(detect(true, false, "Hyprland").unwrap(), BackendKind::Wlr);
        assert!(detect(false, false, "").is_err());
    }
}
