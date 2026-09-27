//! GNOME backend: Mutter's org.gnome.Mutter.DisplayConfig D-Bus API (Wayland and X11).
//!
//! Changes are applied with the "temporary" method, so GNOME never saves them to monitors.xml.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

use crate::display::{Backend, Display, Mode};

const DEST: &str = "org.gnome.Mutter.DisplayConfig";
const PATH: &str = "/org/gnome/Mutter/DisplayConfig";
const METHOD_TEMPORARY: u32 = 1;

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

impl Gnome {
    pub fn open() -> Result<Self> {
        let conn = Connection::session().context("could not connect to the D-Bus session bus")?;
        let gnome = Self { conn };
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
        if !monitors.iter().any(|m| m.0 .0 == display) {
            bail!("display {display} not found");
        }

        // Mutter only takes complete configurations: re-send the current layout with the
        // new mode for our connector.
        let mut config: Vec<ApplyLogicalMonitor> = Vec::new();
        for (x, y, scale, transform, primary, specs, _) in &logical {
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
            config.push((*x, *y, *scale, *transform, *primary, members));
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
