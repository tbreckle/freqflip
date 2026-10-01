//! X11 backend: XRandR 1.3 via x11rb.
//!
//! A new resolution also resizes the screen (the desktop) to fit all CRTCs, like `xrandr`
//! does; other outputs keep their positions.

use anyhow::{bail, Context, Result};
use x11rb::connection::Connection as _;
use x11rb::protocol::randr::{
    Connection, ConnectionExt, GetCrtcInfoReply, GetScreenResourcesCurrentReply, ModeFlag,
    ModeInfo, Rotation, SetConfig,
};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

use crate::display::{Backend, Display, Mode};

pub struct X11 {
    conn: RustConnection,
    root: u32,
    /// Millimeters per pixel of the screen when FreqFlip started, to keep the DPI on resizes.
    mm_per_pixel: (f64, f64),
}

impl X11 {
    pub fn open() -> Result<Self> {
        let (conn, screen) = x11rb::connect(None).context("could not connect to the X server")?;
        let setup = &conn.setup().roots[screen];
        let root = setup.root;
        let mm_per_pixel = (
            f64::from(setup.width_in_millimeters) / f64::from(setup.width_in_pixels.max(1)),
            f64::from(setup.height_in_millimeters) / f64::from(setup.height_in_pixels.max(1)),
        );
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
        Ok(Self {
            conn,
            root,
            mm_per_pixel,
        })
    }

    fn resources(&self) -> Result<GetScreenResourcesCurrentReply> {
        Ok(self
            .conn
            .randr_get_screen_resources_current(self.root)?
            .reply()?)
    }

    /// Sets the mode of a CRTC; same position, rotation and outputs, and a pending transform
    /// (scaling) is kept too.
    fn set_crtc(
        &self,
        res: &GetScreenResourcesCurrentReply,
        crtc: u32,
        crtc_info: &GetCrtcInfoReply,
        mode_id: u32,
    ) -> Result<()> {
        let reply = self
            .conn
            .randr_set_crtc_config(
                crtc,
                CURRENT_TIME,
                res.config_timestamp,
                crtc_info.x,
                crtc_info.y,
                mode_id,
                crtc_info.rotation,
                &crtc_info.outputs,
            )?
            .reply()?;
        if reply.status != SetConfig::SUCCESS {
            bail!("the X server refused the mode ({:?})", reply.status);
        }
        Ok(())
    }

    fn set_screen_size(&self, (width, height): (u16, u16)) -> Result<()> {
        let mm = |pixels: u16, mm_per_pixel: f64| (f64::from(pixels) * mm_per_pixel).round() as u32;
        self.conn
            .randr_set_screen_size(
                self.root,
                width,
                height,
                mm(width, self.mm_per_pixel.0),
                mm(height, self.mm_per_pixel.1),
            )?
            .check()
            .with_context(|| format!("could not resize the screen to {width}x{height}"))
    }

    /// The screen size needed after `crtc` (at `crtc_info`) gets the size `size`: the bounding
    /// box of all active CRTCs, within the server's limits.
    fn screen_size_for(
        &self,
        res: &GetScreenResourcesCurrentReply,
        crtc: u32,
        crtc_info: &GetCrtcInfoReply,
        size: (u16, u16),
    ) -> Result<(u16, u16)> {
        let mut extents = vec![(crtc_info.x, crtc_info.y, size.0, size.1)];
        for &other in res.crtcs.iter().filter(|&&c| c != crtc) {
            let info = self
                .conn
                .randr_get_crtc_info(other, res.config_timestamp)?
                .reply()?;
            if info.mode != 0 {
                extents.push((info.x, info.y, info.width, info.height));
            }
        }
        let range = self.conn.randr_get_screen_size_range(self.root)?.reply()?;
        let (width, height) = bounding_box(&extents);
        if width > u32::from(range.max_width) || height > u32::from(range.max_height) {
            bail!(
                "the screen would need {width}x{height}, the X server allows at most {}x{}",
                range.max_width,
                range.max_height
            );
        }
        // Both fit into u16 now (the maximum is a u16).
        Ok((
            (width as u16).max(range.min_width),
            (height as u16).max(range.min_height),
        ))
    }
}

/// Width and height from the origin needed to show every (x, y, width, height) rectangle.
fn bounding_box(extents: &[(i16, i16, u16, u16)]) -> (u32, u32) {
    let end = |position: i16, length: u16| (i32::from(position) + i32::from(length)).max(0) as u32;
    extents
        .iter()
        .fold((0, 0), |(width, height), &(x, y, w, h)| {
            (width.max(end(x, w)), height.max(end(y, h)))
        })
}

/// Size of a mode on screen: rotated by 90 or 270 degrees, width and height swap.
fn rotated_size(mode: &ModeInfo, rotation: Rotation) -> (u16, u16) {
    if rotation.intersects(Rotation::ROTATE90 | Rotation::ROTATE270) {
        (mode.height, mode.width)
    } else {
        (mode.width, mode.height)
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
            let new_mode = res
                .modes
                .iter()
                .find(|m| m.id == mode_id)
                .with_context(|| format!("mode {mode_id} is no longer available"))?;
            let size = rotated_size(new_mode, crtc.rotation);

            // A new size needs a screen that fits it: grow the screen before the switch,
            // shrink it afterwards (a CRTC must always fit into the screen).
            let screen = if size == (crtc.width, crtc.height) {
                None
            } else {
                let geometry = self.conn.get_geometry(self.root)?.reply()?;
                let current = (geometry.width, geometry.height);
                let wanted = self.screen_size_for(&res, info.crtc, &crtc, size)?;
                let grown = (current.0.max(wanted.0), current.1.max(wanted.1));
                if grown != current {
                    self.set_screen_size(grown)?;
                }
                Some((current, grown, wanted))
            };

            let result = self.set_crtc(&res, info.crtc, &crtc, mode_id);
            if let Some((current, grown, wanted)) = screen {
                match &result {
                    Err(_) if grown != current => {
                        let _ = self.set_screen_size(current);
                    }
                    Ok(()) if wanted != grown => self.set_screen_size(wanted)?,
                    _ => {}
                }
            }
            return result;
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
    fn computes_screen_size() {
        assert_eq!(bounding_box(&[]), (0, 0));
        assert_eq!(bounding_box(&[(0, 0, 640, 480)]), (640, 480));
        // A second monitor right of the first one keeps its position.
        assert_eq!(
            bounding_box(&[(0, 0, 640, 480), (2560, 0, 1920, 1080)]),
            (4480, 1080)
        );
        assert_eq!(bounding_box(&[(-10, 0, 5, 5)]), (0, 5));
    }

    #[test]
    fn rotates_mode_size() {
        let mode = mode_line(0, 0, 0, ModeFlag::from(0u32));
        assert_eq!(rotated_size(&mode, Rotation::ROTATE0), (1920, 1080));
        assert_eq!(rotated_size(&mode, Rotation::ROTATE270), (1080, 1920));
        assert_eq!(
            rotated_size(&mode, Rotation::ROTATE90 | Rotation::REFLECT_X),
            (1080, 1920)
        );
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
