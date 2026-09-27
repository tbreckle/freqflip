//! wlroots backend (Sway, Hyprland, labwc, river, ...): wlr-randr.

use anyhow::{Context, Result};
use serde::Deserialize;

use super::run_tool;
use crate::display::{Backend, Display, Mode};

const TOOL: &str = "wlr-randr";

pub struct Wlr;

#[derive(Deserialize)]
struct Output {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    modes: Vec<OutputMode>,
}

#[derive(Deserialize)]
struct OutputMode {
    width: u32,
    height: u32,
    refresh: f64,
    #[serde(default)]
    current: bool,
}

/// The `--mode` argument for a mode. wlr-randr converts the rate to integer mHz by truncating,
/// so a little is added to land on the exact mHz value it listed.
fn mode_arg(mode: &Mode) -> String {
    format!(
        "{}x{}@{:.4}Hz",
        mode.width,
        mode.height,
        mode.refresh + 0.0004
    )
}

fn parse(json: &str) -> Result<Vec<Display>> {
    let outputs: Vec<Output> =
        serde_json::from_str(json).context("could not parse the wlr-randr output")?;
    let mut displays: Vec<Display> = outputs
        .into_iter()
        .filter(|o| o.enabled)
        .filter_map(|o| {
            let modes: Vec<Mode> = o
                .modes
                .iter()
                .map(|m| {
                    let mut mode = Mode {
                        id: String::new(),
                        width: m.width,
                        height: m.height,
                        refresh: m.refresh,
                        interlaced: false,
                        variant: String::new(),
                    };
                    mode.id = mode_arg(&mode);
                    mode
                })
                .collect();
            let current = o.modes.iter().position(|m| m.current)?;
            Some(Display {
                id: o.name.clone(),
                name: if o.description.is_empty() {
                    o.name
                } else {
                    o.description
                },
                primary: false,
                current: modes[current].clone(),
                modes,
            })
        })
        .collect();
    // wlroots has no primary output; treat the first one as primary.
    if let Some(first) = displays.first_mut() {
        first.primary = true;
    }
    Ok(displays)
}

impl Wlr {
    pub fn open() -> Result<Self> {
        run_tool(TOOL, &["--json"])?;
        Ok(Self)
    }
}

impl Backend for Wlr {
    fn name(&self) -> &'static str {
        "wlroots (wlr-randr)"
    }

    fn displays(&mut self) -> Result<Vec<Display>> {
        parse(&run_tool(TOOL, &["--json"])?)
    }

    fn set_mode(&mut self, display: &str, mode: &Mode) -> Result<()> {
        run_tool(TOOL, &["--output", display, "--mode", &mode.id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wlr_randr_json() {
        let json = r#"[
            {"name":"DP-1","description":"Dell Inc. U2720Q","enabled":true,"modes":[
                {"width":2560,"height":1440,"refresh":164.956,"preferred":true,"current":false},
                {"width":2560,"height":1440,"refresh":59.951,"preferred":false,"current":true}]},
            {"name":"HDMI-A-1","description":"","enabled":false,"modes":[]}
        ]"#;
        let displays = parse(json).unwrap();
        assert_eq!(displays.len(), 1);
        assert_eq!(displays[0].name, "Dell Inc. U2720Q");
        assert!(displays[0].primary);
        assert_eq!(displays[0].current.refresh, 59.951);
        assert_eq!(displays[0].current.id, "2560x1440@59.9514Hz");
    }
}
