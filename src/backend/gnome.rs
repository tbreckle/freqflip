//! GNOME backend: Mutter's org.gnome.Mutter.DisplayConfig D-Bus API (Wayland and X11).
//!
//! Changes are applied with the "temporary" method, so GNOME never saves them to monitors.xml.
//! A new resolution resizes the display's logical monitor; monitors right of or below it move
//! along, because Mutter refuses layouts with gaps or overlaps.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

use crate::display::{Backend, Display, Mode};

const DEST: &str = "org.gnome.Mutter.DisplayConfig";
const PATH: &str = "/org/gnome/Mutter/DisplayConfig";
const METHOD_TEMPORARY: u32 = 1;
const LAYOUT_MODE_PHYSICAL: u32 = 2;
/// Mutter's supported scales are floats computed from the mode size.
const SCALE_EPSILON: f64 = 1e-4;

type Props = HashMap<String, OwnedValue>;
/// (connector, vendor, product, serial)
type MonitorSpec = (String, String, String, String);
/// (id, width, height, refresh, preferred scale, supported scales, properties)
type MutterMode = (String, i32, i32, f64, f64, Vec<f64>, Props);
type Monitor = (MonitorSpec, Vec<MutterMode>, Props);
/// (x, y, scale, transform, primary, monitors, properties)
type LogicalMonitor = (i32, i32, f64, u32, bool, Vec<MonitorSpec>, Props);
type State = (u32, Vec<Monitor>, Vec<LogicalMonitor>, Props);

/// (connector, mode id, properties)
type ApplyMonitor<'a> = (String, String, HashMap<String, Value<'a>>);
type ApplyLogicalMonitor<'a> = (i32, i32, f64, u32, bool, Vec<ApplyMonitor<'a>>);

pub struct Gnome {
    conn: Connection,
    /// Scale of each display's logical monitor before the first switch, to restore it with the
    /// original mode if another resolution needed another scale.
    original_scales: HashMap<String, f64>,
}

fn prop_bool(props: &Props, key: &str) -> bool {
    props
        .get(key)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}

fn prop_string(props: &Props, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
}

fn to_mode(mode: &MutterMode) -> Mode {
    let (id, width, height, refresh, _, _, props) = mode;
    Mode {
        id: id.clone(),
        width: (*width).max(0) as u32,
        height: (*height).max(0) as u32,
        refresh: *refresh,
        interlaced: prop_bool(props, "is-interlaced"),
        variant: String::new(),
    }
}

fn current_mode(monitor: &Monitor) -> Option<&MutterMode> {
    monitor.1.iter().find(|m| prop_bool(&m.6, "is-current"))
}

/// The scale for a mode: the original scale of the logical monitor if the mode supports it,
/// else the current one, else the mode's preferred scale (Mutter only takes supported scales).
fn pick_scale(original: f64, current: f64, supported: &[f64], preferred: f64) -> f64 {
    if supported.is_empty() {
        return current;
    }
    [original, current]
        .iter()
        .find_map(|&scale| {
            supported
                .iter()
                .copied()
                .find(|s| (s - scale).abs() < SCALE_EPSILON)
        })
        .unwrap_or(preferred)
}

/// Size of a logical monitor showing a `width` x `height` mode. Transforms 1, 3, 5 and 7 rotate
/// by 90 or 270 degrees; in the logical layout mode the size is divided by the scale.
fn logical_size(width: i32, height: i32, scale: f64, transform: u32, physical: bool) -> (i32, i32) {
    let (width, height) = if transform % 2 == 1 {
        (height, width)
    } else {
        (width, height)
    };
    if physical || scale <= 0.0 {
        (width, height)
    } else {
        (
            (f64::from(width) / scale).round() as i32,
            (f64::from(height) / scale).round() as i32,
        )
    }
}

/// New position of a logical monitor at `position` when the one at `resized` changes from
/// `old` to `new` size: monitors right of or below it move along, so they keep touching it.
fn shift(
    position: (i32, i32),
    resized: (i32, i32),
    old: (i32, i32),
    new: (i32, i32),
) -> (i32, i32) {
    let along = |p: i32, r: i32, o: i32, n: i32| if p >= r + o { p + n - o } else { p };
    (
        along(position.0, resized.0, old.0, new.0),
        along(position.1, resized.1, old.1, new.1),
    )
}

impl Gnome {
    pub fn open() -> Result<Self> {
        let conn = Connection::session().context("could not connect to the D-Bus session bus")?;
        let gnome = Self {
            conn,
            original_scales: HashMap::new(),
        };
        gnome
            .state()
            .context("GNOME/Mutter display configuration is not available")?;
        Ok(gnome)
    }

    fn state(&self) -> Result<State> {
        let reply = self
            .conn
            .call_method(Some(DEST), PATH, Some(DEST), "GetCurrentState", &())?;
        Ok(reply.body().deserialize()?)
    }
}

impl Backend for Gnome {
    fn name(&self) -> &'static str {
        "GNOME (Mutter D-Bus)"
    }

    fn displays(&mut self) -> Result<Vec<Display>> {
        let (_, monitors, logical, _) = self.state()?;
        let primary: Vec<&str> = logical
            .iter()
            .filter(|l| l.4)
            .flat_map(|l| l.5.iter().map(|spec| spec.0.as_str()))
            .collect();
        Ok(monitors
            .iter()
            .filter_map(|monitor| {
                let current = current_mode(monitor)?;
                let connector = monitor.0 .0.clone();
                Some(Display {
                    primary: primary.contains(&connector.as_str()),
                    name: prop_string(&monitor.2, "display-name").unwrap_or(connector.clone()),
                    id: connector,
                    current: to_mode(current),
                    modes: monitor.1.iter().map(to_mode).collect(),
                })
            })
            .collect())
    }

    fn set_mode(&mut self, display: &str, mode: &Mode) -> Result<()> {
        let (serial, monitors, logical, props) = self.state()?;
        let Some(monitor) = monitors.iter().find(|m| m.0 .0 == display) else {
            bail!("display {display} not found");
        };
        let old_mode =
            current_mode(monitor).with_context(|| format!("{display} has no current mode"))?;
        let new_mode = monitor
            .1
            .iter()
            .find(|m| m.0 == mode.id)
            .with_context(|| format!("mode {} is no longer available", mode.id))?;
        let ours = logical
            .iter()
            .position(|l| l.5.iter().any(|spec| spec.0 == display))
            .with_context(|| format!("{display} is not part of the desktop"))?;

        // The new size of our logical monitor, and with it the new scale.
        let (x0, y0, scale, transform, ..) = logical[ours];
        let original_scale = *self
            .original_scales
            .entry(display.to_string())
            .or_insert(scale);
        let new_scale = pick_scale(original_scale, scale, &new_mode.5, new_mode.4);
        let physical = props.get("layout-mode").and_then(|v| u32::try_from(v).ok())
            == Some(LAYOUT_MODE_PHYSICAL);
        let old_size = logical_size(old_mode.1, old_mode.2, scale, transform, physical);
        let new_size = logical_size(new_mode.1, new_mode.2, new_scale, transform, physical);

        // Mutter only takes complete configurations: re-send the current layout with the
        // new mode for our connector.
        let mut config: Vec<ApplyLogicalMonitor> = Vec::new();
        for (index, (x, y, scale, transform, primary, specs, _)) in logical.iter().enumerate() {
            let mut members = Vec::new();
            for spec in specs {
                let monitor = monitors
                    .iter()
                    .find(|m| m.0 == *spec)
                    .with_context(|| format!("monitor {} vanished", spec.0))?;
                let mode_id = if spec.0 == display {
                    mode.id.clone()
                } else {
                    current_mode(monitor)
                        .with_context(|| format!("{} has no current mode", spec.0))?
                        .0
                        .clone()
                };
                let mut monitor_props = HashMap::new();
                if prop_bool(&monitor.2, "is-underscanning") {
                    monitor_props.insert("underscanning".to_string(), Value::from(true));
                }
                if let Some(color_mode) = monitor
                    .2
                    .get("color-mode")
                    .and_then(|v| u32::try_from(v).ok())
                {
                    monitor_props.insert("color-mode".to_string(), Value::from(color_mode));
                }
                members.push((spec.0.clone(), mode_id, monitor_props));
            }
            let ((x, y), scale) = if index == ours {
                ((*x, *y), new_scale)
            } else {
                (shift((*x, *y), (x0, y0), old_size, new_size), *scale)
            };
            config.push((x, y, scale, *transform, *primary, members));
        }

        let mut apply_props: HashMap<String, Value> = HashMap::new();
        if prop_bool(&props, "supports-changing-layout-mode") {
            if let Some(layout) = props.get("layout-mode").and_then(|v| u32::try_from(v).ok()) {
                apply_props.insert("layout-mode".to_string(), Value::from(layout));
            }
        }

        self.conn
            .call_method(
                Some(DEST),
                PATH,
                Some(DEST),
                "ApplyMonitorsConfig",
                &(serial, METHOD_TEMPORARY, config, apply_props),
            )
            .context("Mutter refused the configuration")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_supported_scale() {
        let supported = [1.0, 1.25, 1.5, 2.0];
        assert_eq!(pick_scale(1.25, 1.0, &supported, 1.0), 1.25);
        assert_eq!(pick_scale(1.75, 1.5, &supported, 1.0), 1.5);
        assert_eq!(pick_scale(1.75, 1.75, &supported, 2.0), 2.0);
        assert_eq!(pick_scale(1.75, 1.5000001, &supported, 1.0), 1.5);
        assert_eq!(pick_scale(2.0, 1.5, &[], 1.0), 1.5);
    }

    #[test]
    fn computes_logical_size() {
        assert_eq!(logical_size(2560, 1440, 2.0, 0, false), (1280, 720));
        assert_eq!(logical_size(2560, 1440, 2.0, 0, true), (2560, 1440));
        assert_eq!(logical_size(1920, 1080, 1.0, 1, false), (1080, 1920));
        assert_eq!(logical_size(1920, 1080, 1.0, 7, false), (1080, 1920));
        assert_eq!(logical_size(1920, 1080, 1.0, 6, false), (1920, 1080));
    }

    #[test]
    fn shifts_neighbors() {
        let (old, new) = ((2560, 1440), (1920, 1080));
        // Right of and below the resized monitor at the origin.
        assert_eq!(shift((2560, 0), (0, 0), old, new), (1920, 0));
        assert_eq!(shift((0, 1440), (0, 0), old, new), (0, 1080));
        assert_eq!(shift((2560, 1440), (0, 0), old, new), (1920, 1080));
        // Left of it: unchanged; and back again on restore.
        assert_eq!(shift((0, 0), (1920, 0), old, new), (0, 0));
        assert_eq!(shift((1920, 0), (0, 0), new, old), (2560, 0));
    }
}
