//! KDE Plasma backend: kscreen-doctor (Wayland and X11).

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::run_tool;
use crate::display::{Backend, Display, Mode};

const TOOL: &str = "kscreen-doctor";

pub struct Kde;

#[derive(Deserialize)]
struct Config {
    outputs: Vec<Output>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Output {
    name: String,
    #[serde(default)]
    connected: bool,
    #[serde(default)]
    enabled: bool,
    /// Plasma < 6 reports `primary`, Plasma 6 a `priority` where 1 is the primary display.
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    priority: Option<u32>,
    #[serde(default)]
    current_mode_id: Value,
    #[serde(default)]
    modes: Vec<OutputMode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutputMode {
    id: Value,
    refresh_rate: f64,
    size: Size,
}

#[derive(Deserialize)]
struct Size {
    width: u32,
    height: u32,
}

/// kscreen-doctor prints ids as strings or numbers depending on the version.
fn id_string(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn parse(json: &str) -> Result<Vec<Display>> {
    let config: Config =
        serde_json::from_str(json).context("could not parse the kscreen-doctor output")?;
    Ok(config
        .outputs
        .into_iter()
        .filter(|o| o.connected && o.enabled)
        .filter_map(|o| {
            let current_id = id_string(&o.current_mode_id);
            let modes: Vec<Mode> = o
                .modes
                .iter()
                .map(|m| Mode {
                    id: id_string(&m.id),
                    width: m.size.width,
                    height: m.size.height,
                    refresh: m.refresh_rate,
                    interlaced: false,
                    variant: String::new(),
                })
                .collect();
            let current = modes.iter().find(|m| m.id == current_id)?.clone();
            Some(Display {
                id: o.name.clone(),
                name: o.name,
                primary: o.primary || o.priority == Some(1),
                current,
                modes,
            })
        })
        .collect())
}

impl Kde {
    pub fn open() -> Result<Self> {
        run_tool(TOOL, &["--json"])?;
        Ok(Self)
    }
}

impl Backend for Kde {
    fn name(&self) -> &'static str {
        "KDE (kscreen-doctor)"
    }

    fn displays(&mut self) -> Result<Vec<Display>> {
        parse(&run_tool(TOOL, &["--json"])?)
    }

    fn set_mode(&mut self, display: &str, mode: &Mode) -> Result<()> {
        run_tool(TOOL, &[&format!("output.{display}.mode.{}", mode.id)])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kscreen_doctor_json() {
        let json = r#"{"outputs":[
            {"id":1,"name":"DP-1","connected":true,"enabled":true,"priority":1,
             "currentModeId":"2","modes":[
                {"id":"1","name":"2560x1440@165","refreshRate":164.956,"size":{"width":2560,"height":1440}},
                {"id":"2","name":"2560x1440@60","refreshRate":59.951,"size":{"width":2560,"height":1440}}]},
            {"id":2,"name":"HDMI-A-1","connected":true,"enabled":true,"priority":2,
             "currentModeId":7,"modes":[
                {"id":7,"name":"1920x1080@50","refreshRate":50.0,"size":{"width":1920,"height":1080}}]},
            {"id":3,"name":"DP-2","connected":false,"enabled":false,"modes":[]}
        ]}"#;
        let displays = parse(json).unwrap();
        assert_eq!(displays.len(), 2);
        assert_eq!(displays[0].id, "DP-1");
        assert!(displays[0].primary);
        assert_eq!(displays[0].current.id, "2");
        assert_eq!(displays[0].modes.len(), 2);
        assert_eq!(displays[1].current.id, "7");
        assert!(!displays[1].primary);
    }
}
