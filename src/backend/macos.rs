//! macOS backend: Quartz Display Services.
//!
//! Modes are configured "for app only", so macOS itself restores the original rate when
//! FreqFlip exits, even if it is killed.

use anyhow::{anyhow, bail, Result};
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_graphics::display::{
    kCGDisplayShowDuplicateLowResolutionModes, CGConfigureOption, CGDirectDisplayID, CGDisplay,
    CGDisplayMode,
};

use crate::display::{Backend, Display, Mode};

pub struct MacOs;

/// All modes of a display, including the HiDPI duplicates macOS hides by default.
fn all_modes(id: CGDirectDisplayID) -> Vec<CGDisplayMode> {
    // SAFETY: a constant CFString exported by CoreGraphics, retained by wrap_under_get_rule.
    let key = unsafe { CFString::wrap_under_get_rule(kCGDisplayShowDuplicateLowResolutionModes) };
    let options =
        CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFBoolean::true_value().as_CFType())]);
    CGDisplayMode::all_display_modes(id, options.as_concrete_TypeRef()).unwrap_or_default()
}

fn to_mode(mode: &CGDisplayMode) -> Mode {
    Mode {
        // The I/O Kit id plus the point size, which tells HiDPI and native variants apart.
        id: format!("{}:{}x{}", mode.mode_id(), mode.width(), mode.height()),
        width: mode.pixel_width() as u32,
        height: mode.pixel_height() as u32,
        refresh: mode.refresh_rate(),
        interlaced: false,
        variant: format!("{}x{} pt", mode.width(), mode.height()),
    }
}

fn display_name(display: &CGDisplay) -> String {
    if display.is_builtin() {
        "Built-in display".to_string()
    } else {
        format!(
            "Display {:04x}:{:04x}",
            display.vendor_number(),
            display.model_number()
        )
    }
}

impl Backend for MacOs {
    fn name(&self) -> &'static str {
        "macOS (Quartz)"
    }

    fn displays(&mut self) -> Result<Vec<Display>> {
        let ids = CGDisplay::active_displays()
            .map_err(|e| anyhow!("could not list displays (CGError {e})"))?;
        let main = CGDisplay::main().id;
        let mut displays = Vec::new();
        for id in ids {
            let display = CGDisplay::new(id);
            let Some(current) = display.display_mode() else {
                continue;
            };
            displays.push(Display {
                id: id.to_string(),
                name: display_name(&display),
                primary: id == main,
                current: to_mode(&current),
                modes: all_modes(id).iter().map(to_mode).collect(),
            });
        }
        Ok(displays)
    }

    fn set_mode(&mut self, display: &str, mode: &Mode) -> Result<()> {
        let id: CGDirectDisplayID = display.parse().map_err(|_| anyhow!("invalid display id"))?;
        let Some(target) = all_modes(id).into_iter().find(|m| to_mode(m).id == mode.id) else {
            bail!("mode {} is no longer available", mode.id);
        };
        let cg = CGDisplay::new(id);
        let config = cg
            .begin_configuration()
            .map_err(|e| anyhow!("could not configure the display (CGError {e})"))?;
        if let Err(e) = cg.configure_display_with_display_mode(&config, &target) {
            let _ = cg.cancel_configuration(&config);
            bail!("macOS refused the mode (CGError {e})");
        }
        cg.complete_configuration(&config, CGConfigureOption::ConfigureForAppOnly)
            .map_err(|e| anyhow!("macOS refused the mode (CGError {e})"))
    }
}
