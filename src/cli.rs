//! Command line interface.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::display::{ModeRequest, Resolution};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "freqflip",
    version = env!("FREQFLIP_VERSION"),
    about = "Flips the display refresh rate (and resolution) while a game runs and flips it back \
             when it exits.",
    after_help = "Examples:\n  \
        freqflip 60 mame                 switch the primary display to 60 Hz until MAME exits\n  \
        freqflip 59.94 Supermodel.exe    NTSC rate for Supermodel\n  \
        freqflip 640x480@60 mame         switch the resolution as well\n  \
        freqflip 1920x1080 mame          only switch the resolution\n  \
        freqflip -d HDMI-1 50 retroarch  switch another display\n  \
        freqflip --list                  show displays, resolutions and refresh rates\n\n\
        FreqFlip returns as soon as the mode is switched and keeps waiting in the background,\n\
        shown as a tray icon. Its menu restores the original mode early."
)]
pub struct Args {
    /// Target refresh rate in Hz (60, 59.94), resolution (1920x1080, 640x480i) or both
    /// (1920x1080@60). Without a resolution the current one is kept; without a rate the one
    /// closest to the current rate is used.
    #[arg(
        value_name = "MODE",
        required_unless_present = "list",
        value_parser = parse_mode
    )]
    pub mode: Option<ModeRequest>,

    /// Process to wait for: a name (mame, mame.exe) or a PID. A named process may start after
    /// FreqFlip; the mode is restored once it has exited.
    #[arg(required_unless_present = "list")]
    pub process: Option<String>,

    /// Display to switch (id or name from --list). Defaults to the primary display.
    #[arg(short, long, value_name = "ID")]
    pub display: Option<String>,

    /// Maximum difference in Hz between the requested and the chosen refresh rate.
    #[arg(short, long, value_name = "HZ", default_value_t = 0.5, value_parser = parse_tolerance)]
    pub tolerance: f64,

    /// Seconds a named process may take to start before the mode is restored (0 = no limit).
    #[arg(short = 's', long, value_name = "SECONDS", default_value_t = 60)]
    pub start_timeout: u64,

    /// Stay in the foreground instead of returning once the mode is switched.
    #[arg(short, long)]
    pub foreground: bool,

    /// Don't show the tray icon.
    #[arg(long)]
    pub no_tray: bool,

    /// Append progress messages to this file (useful when started by a frontend).
    #[arg(long, value_name = "FILE")]
    pub log: Option<PathBuf>,

    /// Windows only: also save the mode in the display settings, like the Settings app does,
    /// for programs that reset the display to its saved mode at startup (some emulators). The
    /// original mode is saved back on restore; if FreqFlip is killed or Windows crashes first,
    /// the switched mode stays saved.
    #[arg(long)]
    pub save: bool,

    /// Display backend (Linux only; auto picks X11, GNOME, KDE or wlroots from the session).
    #[arg(short, long, value_enum, default_value_t = BackendKind::Auto)]
    pub backend: BackendKind,

    /// List the displays with their resolutions and refresh rates.
    #[arg(short, long, conflicts_with_all = ["mode", "process"])]
    pub list: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    Auto,
    /// X11 via XRandR.
    X11,
    /// GNOME / Mutter via D-Bus (Wayland and X11).
    Gnome,
    /// KDE Plasma via kscreen-doctor.
    Kde,
    /// wlroots compositors (Sway, Hyprland, labwc, ...) via wlr-randr.
    Wlr,
}

/// `60`, `1920x1080` or `1920x1080@60`.
fn parse_mode(s: &str) -> Result<ModeRequest, String> {
    let s = s.trim();
    let (resolution, hz) = match s.split_once('@') {
        Some((resolution, hz)) => (Some(resolution), Some(hz)),
        None if s.contains(['x', 'X']) => (Some(s), None),
        None => (None, Some(s)),
    };
    Ok(ModeRequest {
        resolution: resolution.map(parse_resolution).transpose()?,
        refresh: hz.map(parse_hz).transpose()?,
    })
}

/// `1920x1080`, with an optional `i` (interlaced) or `p` (progressive) suffix.
fn parse_resolution(s: &str) -> Result<Resolution, String> {
    let lower = s.trim().to_lowercase();
    let (size, interlaced) = match lower.strip_suffix('i') {
        Some(size) => (size, true),
        None => (lower.strip_suffix('p').unwrap_or(&lower), false),
    };
    let size = size.split_once('x').and_then(|(width, height)| {
        Some((
            width.trim().parse::<u32>().ok()?,
            height.trim().parse::<u32>().ok()?,
        ))
    });
    match size {
        Some((width, height)) if (1..=65535).contains(&width) && (1..=65535).contains(&height) => {
            Ok(Resolution {
                width,
                height,
                interlaced,
            })
        }
        _ => Err(format!("'{s}' is not a resolution like 1920x1080")),
    }
}

fn parse_hz(s: &str) -> Result<f64, String> {
    let hz: f64 = s
        .trim()
        .trim_end_matches("Hz")
        .trim_end_matches("hz")
        .trim()
        .parse()
        .map_err(|_| format!("'{s}' is not a number"))?;
    if !(1.0..=1000.0).contains(&hz) {
        return Err(format!("{hz} Hz is out of range (1-1000)"));
    }
    Ok(hz)
}

fn parse_tolerance(s: &str) -> Result<f64, String> {
    let tolerance: f64 = s.parse().map_err(|_| format!("'{s}' is not a number"))?;
    if !(0.0..=100.0).contains(&tolerance) {
        return Err(format!("{tolerance} is out of range (0-100)"));
    }
    Ok(tolerance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_consistent() {
        Args::command().debug_assert();
    }

    #[test]
    fn parses_rates() {
        assert_eq!(parse_hz("60"), Ok(60.0));
        assert_eq!(parse_hz("59.94"), Ok(59.94));
        assert_eq!(parse_hz("50Hz"), Ok(50.0));
        assert!(parse_hz("fast").is_err());
        assert!(parse_hz("0").is_err());
    }

    #[test]
    fn parses_modes() {
        let res = |width, height, interlaced| {
            Some(Resolution {
                width,
                height,
                interlaced,
            })
        };
        assert_eq!(
            parse_mode("59.94"),
            Ok(ModeRequest {
                resolution: None,
                refresh: Some(59.94)
            })
        );
        assert_eq!(
            parse_mode("1920x1080"),
            Ok(ModeRequest {
                resolution: res(1920, 1080, false),
                refresh: None
            })
        );
        assert_eq!(
            parse_mode("640X480i@60Hz"),
            Ok(ModeRequest {
                resolution: res(640, 480, true),
                refresh: Some(60.0)
            })
        );
        assert_eq!(
            parse_mode("1280x720p@50").unwrap().resolution,
            res(1280, 720, false)
        );
        assert!(parse_mode("1920x@60").is_err());
        assert!(parse_mode("0x480").is_err());
        assert!(parse_mode("640x480@").is_err());
        assert!(parse_mode("@60").is_err());
        assert!(parse_mode("1920*1080").is_err());
    }

    #[test]
    fn requires_mode_and_process_unless_listing() {
        assert!(Args::try_parse_from(["freqflip", "60"]).is_err());
        assert!(Args::try_parse_from(["freqflip", "--list"]).is_ok());
        assert!(Args::try_parse_from(["freqflip", "--list", "60"]).is_err());
        let args = Args::try_parse_from(["freqflip", "-d", "DP-1", "59.94", "mame"]).unwrap();
        assert_eq!(args.mode.and_then(|m| m.refresh), Some(59.94));
        assert_eq!(args.process.as_deref(), Some("mame"));
        assert_eq!(args.display.as_deref(), Some("DP-1"));
        assert_eq!(args.tolerance, 0.5);
    }
}
