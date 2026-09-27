//! Platform-neutral display model, mode selection and formatting.

use anyhow::{bail, Result};

/// One video mode of a display, as reported by a backend.
#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    /// Backend-specific handle used to set the mode (X11 mode XID, Mutter mode id, ...).
    pub id: String,
    pub width: u32,
    pub height: u32,
    /// Refresh rate in Hz. Can be fractional (59.94) and is 0 when the backend doesn't know it.
    pub refresh: f64,
    pub interlaced: bool,
    /// Anything else that has to stay the same when only the refresh rate changes, e.g. the
    /// color depth on Windows or the HiDPI point size on macOS. Empty when not applicable.
    pub variant: String,
}

impl Mode {
    /// True if `other` has the same resolution (and scan type/variant) as this mode.
    pub fn same_resolution(&self, other: &Mode) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.interlaced == other.interlaced
            && self.variant == other.variant
    }

    pub fn resolution(&self) -> String {
        format!(
            "{}x{}{}",
            self.width,
            self.height,
            if self.interlaced { "i" } else { "" }
        )
    }
}

/// A connected, active display.
#[derive(Clone, Debug)]
pub struct Display {
    /// Stable identifier used with `--display`, e.g. `DP-1` or `DISPLAY1`.
    pub id: String,
    /// Human-readable name (monitor model), may equal `id`.
    pub name: String,
    pub primary: bool,
    pub current: Mode,
    pub modes: Vec<Mode>,
}

impl Display {
    /// Refresh rates available at the current resolution, highest first, without duplicates.
    pub fn rates(&self) -> Vec<f64> {
        let mut rates: Vec<f64> = self
            .modes
            .iter()
            .filter(|m| m.same_resolution(&self.current) && m.refresh > 0.0)
            .map(|m| m.refresh)
            .collect();
        rates.sort_by(|a, b| b.total_cmp(a));
        rates.dedup_by(|a, b| format_hz(*a) == format_hz(*b));
        rates
    }

    pub fn label(&self) -> String {
        if self.name.is_empty() || self.name == self.id {
            self.id.clone()
        } else {
            format!("{} ({})", self.id, self.name)
        }
    }
}

/// A display backend: lists displays and switches their modes.
pub trait Backend {
    fn name(&self) -> &'static str;
    fn displays(&mut self) -> Result<Vec<Display>>;
    /// Switches `display` (a `Display::id`) to `mode` (one of its `Display::modes`).
    fn set_mode(&mut self, display: &str, mode: &Mode) -> Result<()>;

    /// Reads the current mode of a display.
    fn current_mode(&mut self, id: &str) -> Result<Mode> {
        Ok(self.display(id)?.current)
    }

    /// Re-reads a single display.
    fn display(&mut self, id: &str) -> Result<Display> {
        match self.displays()?.into_iter().find(|d| d.id == id) {
            Some(display) => Ok(display),
            None => bail!("display {id} is gone"),
        }
    }
}

/// Picks the display named by `wanted` (id or name, case-insensitive), or the primary display.
pub fn select_display(displays: Vec<Display>, wanted: Option<&str>) -> Result<Display> {
    if displays.is_empty() {
        bail!("no active display found");
    }
    let Some(wanted) = wanted else {
        let index = displays.iter().position(|d| d.primary).unwrap_or(0);
        return Ok(displays.into_iter().nth(index).expect("index is in bounds"));
    };
    let normalize = |s: &str| s.trim_start_matches(r"\\.\").to_lowercase();
    let wanted_norm = normalize(wanted);
    let known = displays
        .iter()
        .map(|d| d.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    displays
        .into_iter()
        .find(|d| normalize(&d.id) == wanted_norm || normalize(&d.name) == wanted_norm)
        .ok_or_else(|| anyhow::anyhow!("display '{wanted}' not found (available: {known})"))
}

/// Picks the mode of `display` with the current resolution whose refresh rate is closest to
/// `target` Hz. Fails if the closest one is more than `tolerance` Hz off.
pub fn pick_mode(display: &Display, target: f64, tolerance: f64) -> Result<Mode> {
    let current = &display.current;
    let best = display
        .modes
        .iter()
        .filter(|m| m.same_resolution(current) && m.refresh > 0.0)
        // On a tie (two modes with the same rate but other timings) keep the current mode.
        .min_by(|a, b| {
            let da = (a.refresh - target).abs();
            let db = (b.refresh - target).abs();
            da.total_cmp(&db)
                .then_with(|| (b.id == current.id).cmp(&(a.id == current.id)))
        });
    match best {
        Some(mode) if (mode.refresh - target).abs() <= tolerance => Ok(mode.clone()),
        _ => {
            let rates = display
                .rates()
                .into_iter()
                .map(format_hz)
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "{} has no {} mode at {} (available: {})",
                display.id,
                format_hz(target),
                current.resolution(),
                if rates.is_empty() { "none" } else { &rates }
            )
        }
    }
}

/// Formats a refresh rate: whole numbers without decimals ("60 Hz"), others with two ("59.94 Hz").
pub fn format_hz(hz: f64) -> String {
    if hz <= 0.0 {
        "? Hz".to_string()
    } else if (hz - hz.round()).abs() < 0.005 {
        format!("{} Hz", hz.round() as i64)
    } else {
        format!("{hz:.2} Hz")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(id: &str, width: u32, height: u32, refresh: f64) -> Mode {
        Mode {
            id: id.to_string(),
            width,
            height,
            refresh,
            interlaced: false,
            variant: String::new(),
        }
    }

    fn display() -> Display {
        Display {
            id: "DP-1".into(),
            name: "Arcade CRT".into(),
            primary: true,
            current: mode("a", 2560, 1440, 164.96),
            modes: vec![
                mode("a", 2560, 1440, 164.96),
                mode("b", 2560, 1440, 143.97),
                mode("c", 2560, 1440, 120.0),
                mode("d", 2560, 1440, 59.95),
                mode("e", 1920, 1080, 60.0),
                mode("f", 1920, 1080, 50.0),
            ],
        }
    }

    #[test]
    fn picks_closest_rate_at_current_resolution() {
        let d = display();
        assert_eq!(pick_mode(&d, 60.0, 0.5).unwrap().id, "d");
        assert_eq!(pick_mode(&d, 144.0, 0.5).unwrap().id, "b");
        assert_eq!(pick_mode(&d, 165.0, 0.5).unwrap().id, "a");
    }

    #[test]
    fn rejects_rates_outside_tolerance() {
        let d = display();
        // 50 Hz only exists at 1920x1080, which would change the resolution.
        let err = pick_mode(&d, 50.0, 0.5).unwrap_err().to_string();
        assert!(err.contains("no 50 Hz mode at 2560x1440"), "{err}");
        assert!(
            err.contains("164.96 Hz, 143.97 Hz, 120 Hz, 59.95 Hz"),
            "{err}"
        );
        assert!(pick_mode(&d, 58.0, 2.0).is_ok());
    }

    #[test]
    fn prefers_current_mode_on_tie() {
        let mut d = display();
        d.modes.push(mode("a2", 2560, 1440, 164.96));
        d.modes.swap(0, 6);
        assert_eq!(pick_mode(&d, 164.96, 0.5).unwrap().id, "a");
    }

    #[test]
    fn respects_variant_and_scan_type() {
        let mut d = display();
        let mut hidpi = mode("x", 2560, 1440, 60.0);
        hidpi.variant = "1280x720pt".into();
        let mut interlaced = mode("y", 2560, 1440, 60.0);
        interlaced.interlaced = true;
        d.modes = vec![d.current.clone(), hidpi, interlaced];
        assert!(pick_mode(&d, 60.0, 0.5).is_err());
    }

    #[test]
    fn selects_display() {
        let mut other = display();
        other.id = "HDMI-1".into();
        other.name = "TV".into();
        other.primary = false;
        let displays = vec![other, display()];
        assert_eq!(
            select_display(displays.clone(), None).unwrap().id,
            "DP-1",
            "primary by default"
        );
        assert_eq!(
            select_display(displays.clone(), Some("hdmi-1")).unwrap().id,
            "HDMI-1"
        );
        assert_eq!(
            select_display(displays.clone(), Some("tv")).unwrap().id,
            "HDMI-1"
        );
        let err = select_display(displays, Some("DP-9")).unwrap_err();
        assert!(err.to_string().contains("available: HDMI-1, DP-1"));
        assert!(select_display(Vec::new(), None).is_err());
    }

    #[test]
    fn windows_device_prefix_is_optional() {
        let mut d = display();
        d.id = r"DISPLAY1".into();
        assert!(select_display(vec![d.clone()], Some(r"\\.\DISPLAY1")).is_ok());
        assert!(select_display(vec![d], Some("display1")).is_ok());
    }

    #[test]
    fn lists_rates_without_duplicates() {
        let mut d = display();
        d.modes.push(mode("z", 2560, 1440, 120.001));
        assert_eq!(d.rates(), vec![164.96, 143.97, 120.001, 59.95]);
    }

    #[test]
    fn formats_rates() {
        assert_eq!(format_hz(60.0), "60 Hz");
        assert_eq!(format_hz(60.001), "60 Hz");
        assert_eq!(format_hz(59.94), "59.94 Hz");
        assert_eq!(format_hz(164.958), "164.96 Hz");
        assert_eq!(format_hz(0.0), "? Hz");
    }
}
