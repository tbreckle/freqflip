//! Platform-neutral display model, mode selection and formatting.

use std::fmt;

use anyhow::{bail, Result};

/// A resolution with its scan type, e.g. 1920x1080 or 640x480i.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
    pub interlaced: bool,
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}x{}{}",
            self.width,
            self.height,
            if self.interlaced { "i" } else { "" }
        )
    }
}

/// What to switch to: a refresh rate, a resolution or both. A missing part keeps the
/// current resolution, or the refresh rate closest to the current one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModeRequest {
    pub resolution: Option<Resolution>,
    pub refresh: Option<f64>,
}

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
    /// Anything else that has to stay the same when only the refresh rate changes, and that is
    /// kept if possible when the resolution changes, e.g. the color depth on Windows or the
    /// HiDPI scale on macOS. Empty when not applicable.
    pub variant: String,
}

impl Mode {
    /// True if `other` has the same resolution (and scan type/variant) as this mode.
    pub fn same_resolution(&self, other: &Mode) -> bool {
        self.resolution() == other.resolution() && self.variant == other.variant
    }

    pub fn resolution(&self) -> Resolution {
        Resolution {
            width: self.width,
            height: self.height,
            interlaced: self.interlaced,
        }
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
    /// Refresh rates available at the current resolution (and variant), highest first, without
    /// duplicates.
    pub fn rates(&self) -> Vec<f64> {
        self.rates_where(|m| m.same_resolution(&self.current))
    }

    /// Refresh rates available at `resolution` in any variant, highest first, without duplicates.
    pub fn rates_at(&self, resolution: Resolution) -> Vec<f64> {
        self.rates_where(|m| m.resolution() == resolution)
    }

    fn rates_where(&self, keep: impl Fn(&Mode) -> bool) -> Vec<f64> {
        let mut rates: Vec<f64> = self
            .modes
            .iter()
            .filter(|m| keep(m) && m.refresh > 0.0)
            .map(|m| m.refresh)
            .collect();
        rates.sort_by(|a, b| b.total_cmp(a));
        rates.dedup_by(|a, b| format_hz(*a) == format_hz(*b));
        rates
    }

    /// The available resolutions: the current one first, then the largest first.
    pub fn resolutions(&self) -> Vec<Resolution> {
        let current = self.current.resolution();
        let mut resolutions: Vec<Resolution> = self
            .modes
            .iter()
            .filter(|m| m.refresh > 0.0)
            .map(Mode::resolution)
            .collect();
        resolutions.sort_by_key(|r| {
            (
                *r != current,
                std::cmp::Reverse(u64::from(r.width) * u64::from(r.height)),
                std::cmp::Reverse(r.width),
                r.interlaced,
            )
        });
        resolutions.dedup();
        resolutions
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

/// Picks the mode of `display` for `request`. Without a resolution, only modes with the
/// current resolution (and variant) count; with one, modes of the current variant are preferred.
/// Among those, the refresh rate closest to the requested one wins (it fails if that is more
/// than `tolerance` Hz off), or without a requested rate the one closest to the current rate.
pub fn pick_mode(display: &Display, request: &ModeRequest, tolerance: f64) -> Result<Mode> {
    let current = &display.current;
    let resolution = request.resolution.unwrap_or(current.resolution());
    let candidates: Vec<&Mode> = display
        .modes
        .iter()
        .filter(|m| {
            m.refresh > 0.0
                && match request.resolution {
                    Some(resolution) => m.resolution() == resolution,
                    None => m.same_resolution(current),
                }
        })
        .collect();
    if candidates.is_empty() {
        let resolutions = display
            .resolutions()
            .iter()
            .map(Resolution::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "{} has no {resolution} mode (available: {})",
            display.id,
            if resolutions.is_empty() {
                "none"
            } else {
                &resolutions
            }
        );
    }

    // Without a requested rate, stay as close to the current one as possible (or take the
    // highest if the current one is unknown).
    let target = match request.refresh {
        Some(hz) => hz,
        None if current.refresh > 0.0 => current.refresh,
        None => f64::MAX,
    };
    let distance = |m: &Mode| (m.refresh - target).abs();
    let best = candidates
        .into_iter()
        .filter(|m| request.refresh.is_none() || distance(m) <= tolerance)
        // Keep the variant if possible; on a tie (two modes with the same rate but other
        // timings) keep the current mode.
        .min_by(|a, b| {
            (a.variant != current.variant)
                .cmp(&(b.variant != current.variant))
                .then_with(|| distance(a).total_cmp(&distance(b)))
                .then_with(|| (b.id == current.id).cmp(&(a.id == current.id)))
        });
    match best {
        Some(mode) => Ok(mode.clone()),
        None => {
            let rates = match request.resolution {
                Some(resolution) => display.rates_at(resolution),
                None => display.rates(),
            };
            let rates = rates.into_iter().map(format_hz).collect::<Vec<_>>();
            bail!(
                "{} has no {} mode at {resolution} (available: {})",
                display.id,
                format_hz(target),
                rates.join(", ")
            )
        }
    }
}

/// Formats a mode as its refresh rate ("60 Hz"), with `resolution` as "1920x1080 @ 60 Hz".
pub fn format_mode(mode: &Mode, resolution: bool) -> String {
    if resolution {
        format!("{} @ {}", mode.resolution(), format_hz(mode.refresh))
    } else {
        format_hz(mode.refresh)
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

    fn hz(refresh: f64) -> ModeRequest {
        ModeRequest {
            resolution: None,
            refresh: Some(refresh),
        }
    }

    fn res(width: u32, height: u32, refresh: Option<f64>) -> ModeRequest {
        ModeRequest {
            resolution: Some(Resolution {
                width,
                height,
                interlaced: false,
            }),
            refresh,
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
        assert_eq!(pick_mode(&d, &hz(60.0), 0.5).unwrap().id, "d");
        assert_eq!(pick_mode(&d, &hz(144.0), 0.5).unwrap().id, "b");
        assert_eq!(pick_mode(&d, &hz(165.0), 0.5).unwrap().id, "a");
    }

    #[test]
    fn rejects_rates_outside_tolerance() {
        let d = display();
        // 50 Hz only exists at 1920x1080, which would change the resolution.
        let err = pick_mode(&d, &hz(50.0), 0.5).unwrap_err().to_string();
        assert!(err.contains("no 50 Hz mode at 2560x1440"), "{err}");
        assert!(
            err.contains("164.96 Hz, 143.97 Hz, 120 Hz, 59.95 Hz"),
            "{err}"
        );
        assert!(pick_mode(&d, &hz(58.0), 2.0).is_ok());
    }

    #[test]
    fn prefers_current_mode_on_tie() {
        let mut d = display();
        d.modes.push(mode("a2", 2560, 1440, 164.96));
        d.modes.swap(0, 6);
        assert_eq!(pick_mode(&d, &hz(164.96), 0.5).unwrap().id, "a");
    }

    #[test]
    fn respects_variant_and_scan_type() {
        let mut d = display();
        let mut hidpi = mode("x", 2560, 1440, 60.0);
        hidpi.variant = "1280x720pt".into();
        let mut interlaced = mode("y", 2560, 1440, 60.0);
        interlaced.interlaced = true;
        d.modes = vec![d.current.clone(), hidpi, interlaced];
        assert!(pick_mode(&d, &hz(60.0), 0.5).is_err());
    }

    #[test]
    fn picks_other_resolutions() {
        let d = display();
        assert_eq!(
            pick_mode(&d, &res(1920, 1080, Some(50.0)), 0.5).unwrap().id,
            "f"
        );
        assert_eq!(
            pick_mode(&d, &res(1920, 1080, Some(59.5)), 0.5).unwrap().id,
            "e"
        );
        // Without a rate, the one closest to the current rate (164.96 Hz).
        assert_eq!(pick_mode(&d, &res(1920, 1080, None), 0.5).unwrap().id, "e");
        assert_eq!(pick_mode(&d, &res(2560, 1440, None), 0.5).unwrap().id, "a");

        let err = pick_mode(&d, &res(1920, 1080, Some(75.0)), 0.5).unwrap_err();
        assert_eq!(
            err.to_string(),
            "DP-1 has no 75 Hz mode at 1920x1080 (available: 60 Hz, 50 Hz)"
        );
        let err = pick_mode(&d, &res(640, 480, Some(60.0)), 0.5).unwrap_err();
        assert_eq!(
            err.to_string(),
            "DP-1 has no 640x480 mode (available: 2560x1440, 1920x1080)"
        );
    }

    #[test]
    fn keeps_variant_when_changing_resolution() {
        let mut d = display();
        let variant = |id: &str, width, height, refresh, variant: &str| {
            let mut m = mode(id, width, height, refresh);
            m.variant = variant.into();
            m
        };
        d.current = variant("a", 2560, 1440, 60.0, "2x");
        d.modes = vec![
            d.current.clone(),
            variant("b", 1920, 1080, 60.0, "1x"),
            variant("c", 1920, 1080, 60.0, "2x"),
            variant("d", 1920, 1080, 120.0, "1x"),
            variant("e", 1280, 720, 60.0, "1x"),
        ];
        assert_eq!(
            pick_mode(&d, &res(1920, 1080, Some(60.0)), 0.5).unwrap().id,
            "c"
        );
        // Another variant if the current one doesn't have the rate or the resolution.
        assert_eq!(
            pick_mode(&d, &res(1920, 1080, Some(120.0)), 0.5)
                .unwrap()
                .id,
            "d"
        );
        assert_eq!(pick_mode(&d, &res(1280, 720, None), 0.5).unwrap().id, "e");
        assert_eq!(d.rates_at(d.modes[1].resolution()), vec![120.0, 60.0]);
    }

    #[test]
    fn picks_interlaced_only_on_request() {
        let mut d = display();
        let mut interlaced = mode("i", 1920, 1080, 60.0);
        interlaced.interlaced = true;
        d.modes.push(interlaced);
        let mut request = res(1920, 1080, Some(60.0));
        assert_eq!(pick_mode(&d, &request, 0.5).unwrap().id, "e");
        request.resolution = Some(Resolution {
            width: 1920,
            height: 1080,
            interlaced: true,
        });
        assert_eq!(pick_mode(&d, &request, 0.5).unwrap().id, "i");
    }

    #[test]
    fn lists_resolutions() {
        let mut d = display();
        d.current = d.modes[4].clone();
        d.modes.push(mode("g", 3840, 2160, 60.0));
        d.modes.push(mode("h", 0, 0, 0.0));
        let resolutions: Vec<String> = d.resolutions().iter().map(|r| r.to_string()).collect();
        assert_eq!(resolutions, ["1920x1080", "3840x2160", "2560x1440"]);
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

    #[test]
    fn formats_modes() {
        let mut m = mode("a", 640, 480, 59.94);
        assert_eq!(format_mode(&m, false), "59.94 Hz");
        assert_eq!(format_mode(&m, true), "640x480 @ 59.94 Hz");
        m.interlaced = true;
        assert_eq!(format_mode(&m, true), "640x480i @ 59.94 Hz");
    }
}
