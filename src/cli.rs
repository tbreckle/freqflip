//! Command line interface.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "freqflip",
    version = env!("FREQFLIP_VERSION"),
    about = "Flips the display refresh rate while a game runs and flips it back when it exits.",
    after_help = "Examples:\n  \
        freqflip 60 mame                 switch the primary display to 60 Hz until MAME exits\n  \
        freqflip 59.94 Supermodel.exe    NTSC rate for Supermodel\n  \
        freqflip -d HDMI-1 50 retroarch  switch another display\n  \
        freqflip --list                  show displays and their refresh rates\n\n\
        FreqFlip returns as soon as the rate is switched and keeps waiting in the background,\n\
        shown as a tray icon. Its menu restores the original rate early."
)]
pub struct Args {
    /// Target refresh rate in Hz, e.g. 60 or 59.94.
    #[arg(required_unless_present = "list", value_parser = parse_hz)]
    pub hz: Option<f64>,

    /// Process to wait for: a name (mame, mame.exe) or a PID. A named process may start after
    /// FreqFlip; the rate is restored once it has exited.
    #[arg(required_unless_present = "list")]
    pub process: Option<String>,

    /// Display to switch (id or name from --list). Defaults to the primary display.
    #[arg(short, long, value_name = "ID")]
    pub display: Option<String>,

    /// Maximum difference in Hz between the requested and the chosen refresh rate.
    #[arg(short, long, value_name = "HZ", default_value_t = 0.5, value_parser = parse_tolerance)]
    pub tolerance: f64,

    /// Seconds a named process may take to start before the rate is restored (0 = no limit).
    #[arg(short = 's', long, value_name = "SECONDS", default_value_t = 60)]
    pub start_timeout: u64,

    /// Stay in the foreground instead of returning once the rate is switched.
    #[arg(short, long)]
    pub foreground: bool,

    /// Don't show the tray icon.
    #[arg(long)]
    pub no_tray: bool,

    /// Append progress messages to this file (useful when started by a frontend).
    #[arg(long, value_name = "FILE")]
    pub log: Option<PathBuf>,

    /// Display backend (Linux only; auto picks X11, GNOME, KDE or wlroots from the session).
    #[arg(short, long, value_enum, default_value_t = BackendKind::Auto)]
    pub backend: BackendKind,

    /// List the displays and the refresh rates available at their current resolution.
    #[arg(short, long, conflicts_with_all = ["hz", "process"])]
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
    fn requires_rate_and_process_unless_listing() {
        assert!(Args::try_parse_from(["freqflip", "60"]).is_err());
        assert!(Args::try_parse_from(["freqflip", "--list"]).is_ok());
        assert!(Args::try_parse_from(["freqflip", "--list", "60"]).is_err());
        let args = Args::try_parse_from(["freqflip", "-d", "DP-1", "59.94", "mame"]).unwrap();
        assert_eq!(args.hz, Some(59.94));
        assert_eq!(args.process.as_deref(), Some("mame"));
        assert_eq!(args.display.as_deref(), Some("DP-1"));
        assert_eq!(args.tolerance, 0.5);
    }
}
