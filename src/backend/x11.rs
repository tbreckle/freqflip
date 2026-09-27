//! X11 backend: XRandR 1.3 via x11rb.

use anyhow::{bail, Context, Result};
use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{
    Connection, ConnectionExt, GetScreenResourcesCurrentReply, ModeFlag, ModeInfo, SetConfig,
};
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

use crate::display::{Backend, Display, Mode};

pub struct X11 {
    conn: RustConnection,
    root: u32,
}

impl X11 {
    pub fn open() -> Result<Self> {
        let (conn, screen) = x11rb::connect(None).context("could not connect to the X server")?;
        let root = conn.setup().roots[screen].root;
        let version = conn
            .randr_query_version(1, 3)?
            .reply()
            .context("the X server has no RandR extension")?;
        if (version.major_version, version.minor_version) < (1, 3) {
            bail!(
                "RandR {}.{} is too old, 1.3 is needed",
                version.major_version,
                version.minor_version
            );
        }
        Ok(Self { conn, root })
    }

    fn resources(&self) -> Result<GetScreenResourcesCurrentReply> {
        Ok(self
            .conn
            .randr_get_screen_resources_current(self.root)?
            .reply()?)
    }
}

/// Refresh rate of an X11 mode line in Hz.
pub fn refresh_rate(mode: &ModeInfo) -> f64 {
    let mut vtotal = f64::from(mode.vtotal);
    if mode.mode_flags.contains(ModeFlag::DOUBLE_SCAN) {
        vtotal *= 2.0;
    }
    if mode.mode_flags.contains(ModeFlag::INTERLACE) {
        vtotal /= 2.0;
    }
    let htotal = f64::from(mode.htotal);
    if htotal == 0.0 || vtotal == 0.0 {
        return 0.0;
    }
    f64::from(mode.dot_clock) / (htotal * vtotal)
}

fn to_mode(info: &ModeInfo) -> Mode {
    Mode {
        id: info.id.to_string(),
        width: info.width.into(),
        height: info.height.into(),
        refresh: refresh_rate(info),
        interlaced: info.mode_flags.contains(ModeFlag::INTERLACE),
        variant: String::new(),
    }
}

impl Backend for X11 {
    fn name(&self) -> &'static str {
        "X11 (XRandR)"
    }

    fn displays(&mut self) -> Result<Vec<Display>> {
        let res = self.resources()?;
        let primary = self
            .conn
            .randr_get_output_primary(self.root)?
            .reply()?
            .output;
        let mut displays = Vec::new();
        for &output in &res.outputs {
            let info = self
                .conn
                .randr_get_output_info(output, res.config_timestamp)?
                .reply()?;
            if info.connection != Connection::CONNECTED || info.crtc == 0 {
                continue;
            }
            let crtc = self
                .conn
                .randr_get_crtc_info(info.crtc, res.config_timestamp)?
                .reply()?;
            let find = |id: u32| res.modes.iter().find(|m| m.id == id).map(to_mode);
            let Some(current) = find(crtc.mode) else {
                continue;
            };
            let name = String::from_utf8_lossy(&info.name).into_owned();
            displays.push(Display {
                id: name.clone(),
                name,
                primary: output == primary,
                current,
                modes: info.modes.iter().filter_map(|&id| find(id)).collect(),
            });
        }
        Ok(displays)
    }

    fn set_mode(&mut self, display: &str, mode: &Mode) -> Result<()> {
        let mode_id: u32 = mode.id.parse().context("invalid X11 mode id")?;
        let res = self.resources()?;
        for &output in &res.outputs {
            let info = self
                .conn
                .randr_get_output_info(output, res.config_timestamp)?
                .reply()?;
            if info.name != display.as_bytes() {
                continue;
            }
            if info.crtc == 0 {
                bail!("{display} is not active");
            }
            let crtc = self
                .conn
                .randr_get_crtc_info(info.crtc, res.config_timestamp)?
                .reply()?;
            // Same position, rotation and outputs; a pending transform (scaling) is kept too.
            let reply = self
                .conn
                .randr_set_crtc_config(
                    info.crtc,
                    CURRENT_TIME,
                    res.config_timestamp,
                    crtc.x,
                    crtc.y,
                    mode_id,
                    crtc.rotation,
                    &crtc.outputs,
                )?
                .reply()?;
            if reply.status != SetConfig::SUCCESS {
                bail!("the X server refused the mode ({:?})", reply.status);
            }
            return Ok(());
        }
        bail!("display {display} not found")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode_line(dot_clock: u32, htotal: u16, vtotal: u16, flags: ModeFlag) -> ModeInfo {
        ModeInfo {
            id: 1,
            width: 1920,
            height: 1080,
            dot_clock,
            hsync_start: 0,
            hsync_end: 0,
            htotal,
            hskew: 0,
            vsync_start: 0,
            vsync_end: 0,
            vtotal,
            name_len: 0,
            mode_flags: flags,
        }
    }

    #[test]
    fn computes_refresh_rates() {
        let cea_1080p60 = mode_line(148_500_000, 2200, 1125, ModeFlag::from(0u32));
        assert!((refresh_rate(&cea_1080p60) - 60.0).abs() < 1e-9);
        let ntsc = mode_line(148_351_648, 2200, 1125, ModeFlag::from(0u32));
        assert!((refresh_rate(&ntsc) - 59.94).abs() < 0.001);
        let interlaced = mode_line(74_250_000, 2200, 1125, ModeFlag::INTERLACE);
        assert!((refresh_rate(&interlaced) - 60.0).abs() < 1e-9);
        let doublescan = mode_line(25_175_000, 800, 262, ModeFlag::DOUBLE_SCAN);
        assert!((refresh_rate(&doublescan) - 60.05).abs() < 0.01);
        assert_eq!(refresh_rate(&mode_line(1, 0, 0, ModeFlag::from(0u32))), 0.0);
    }
}
