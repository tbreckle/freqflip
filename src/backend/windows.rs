//! Windows backend: GDI display settings (EnumDisplaySettingsW / ChangeDisplaySettingsExW).
//!
//! Modes are changed dynamically without CDS_UPDATEREGISTRY, so a reboot always brings back the
//! saved rate even if FreqFlip is killed.

use std::mem;
use std::ptr;

use anyhow::{bail, Result};
use windows_sys::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW, DISPLAY_DEVICEW,
    DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICE_PRIMARY_DEVICE, DISP_CHANGE_SUCCESSFUL,
    DM_DISPLAYFLAGS, DM_DISPLAYFREQUENCY, DM_INTERLACED, DM_PELSHEIGHT, DM_PELSWIDTH,
    ENUM_CURRENT_SETTINGS,
};

use crate::display::{Backend, Display, Mode};

pub struct Windows;

const DEVICE_PREFIX: &str = r"\\.\";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

fn new_devmode() -> DEVMODEW {
    DEVMODEW {
        dmSize: mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    }
}

fn display_flags(dm: &DEVMODEW) -> u32 {
    // SAFETY: both union members are plain u32s; display drivers fill in dmDisplayFlags.
    unsafe { dm.Anonymous2.dmDisplayFlags }
}

fn to_mode(dm: &DEVMODEW) -> Mode {
    let interlaced = display_flags(dm) & DM_INTERLACED != 0;
    Mode {
        id: format!(
            "{}x{}@{}{}",
            dm.dmPelsWidth,
            dm.dmPelsHeight,
            dm.dmDisplayFrequency,
            if interlaced { "i" } else { "" }
        ),
        width: dm.dmPelsWidth,
        height: dm.dmPelsHeight,
        refresh: f64::from(dm.dmDisplayFrequency),
        interlaced,
        variant: format!("{} bpp", dm.dmBitsPerPel),
    }
}

/// Reads a display mode; `index` is a mode number or ENUM_CURRENT_SETTINGS.
fn enum_settings(device: &[u16], index: u32) -> Option<DEVMODEW> {
    let mut dm = new_devmode();
    // SAFETY: `device` is NUL-terminated and `dm` has its dmSize set.
    let ok = unsafe { EnumDisplaySettingsW(device.as_ptr(), index, &mut dm) };
    (ok != 0).then_some(dm)
}

/// Attached display adapters: (device name like `\\.\DISPLAY1`, primary, monitor name).
fn devices() -> Vec<(String, bool, String)> {
    let mut result = Vec::new();
    for index in 0.. {
        let mut dev: DISPLAY_DEVICEW = unsafe { mem::zeroed() };
        dev.cb = mem::size_of::<DISPLAY_DEVICEW>() as u32;
        // SAFETY: `dev` is zeroed with cb set, a null device enumerates adapters.
        if unsafe { EnumDisplayDevicesW(ptr::null(), index, &mut dev, 0) } == 0 {
            break;
        }
        if dev.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP == 0 {
            continue;
        }
        let device = from_wide(&dev.DeviceName);
        let mut monitor: DISPLAY_DEVICEW = unsafe { mem::zeroed() };
        monitor.cb = mem::size_of::<DISPLAY_DEVICEW>() as u32;
        // SAFETY: as above; the adapter name enumerates its monitors.
        let has_monitor =
            unsafe { EnumDisplayDevicesW(dev.DeviceName.as_ptr(), 0, &mut monitor, 0) } != 0;
        let name = if has_monitor {
            from_wide(&monitor.DeviceString)
        } else {
            from_wide(&dev.DeviceString)
        };
        result.push((
            device,
            dev.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0,
            name,
        ));
    }
    result
}

impl Backend for Windows {
    fn name(&self) -> &'static str {
        "Windows (GDI)"
    }

    fn displays(&mut self) -> Result<Vec<Display>> {
        let mut displays = Vec::new();
        for (device, primary, name) in devices() {
            let device_w = wide(&device);
            let Some(current) = enum_settings(&device_w, ENUM_CURRENT_SETTINGS) else {
                continue;
            };
            let mut modes: Vec<Mode> = Vec::new();
            for index in 0.. {
                let Some(dm) = enum_settings(&device_w, index) else {
                    break;
                };
                // 0 and 1 stand for "hardware default", not real rates.
                if dm.dmDisplayFrequency <= 1 {
                    continue;
                }
                let mode = to_mode(&dm);
                if !modes.contains(&mode) {
                    modes.push(mode);
                }
            }
            displays.push(Display {
                id: device.trim_start_matches(DEVICE_PREFIX).to_string(),
                name,
                primary,
                current: to_mode(&current),
                modes,
            });
        }
        Ok(displays)
    }

    fn current_mode(&mut self, display: &str) -> Result<Mode> {
        let device = wide(&format!("{DEVICE_PREFIX}{display}"));
        match enum_settings(&device, ENUM_CURRENT_SETTINGS) {
            Some(dm) => Ok(to_mode(&dm)),
            None => bail!("display {display} not found"),
        }
    }

    fn set_mode(&mut self, display: &str, mode: &Mode) -> Result<()> {
        let device = wide(&format!("{DEVICE_PREFIX}{display}"));
        let Some(mut dm) = enum_settings(&device, ENUM_CURRENT_SETTINGS) else {
            bail!("display {display} not found");
        };
        dm.dmPelsWidth = mode.width;
        dm.dmPelsHeight = mode.height;
        dm.dmDisplayFrequency = mode.refresh.round() as u32;
        dm.Anonymous2.dmDisplayFlags = if mode.interlaced { DM_INTERLACED } else { 0 };
        dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_DISPLAYFLAGS;
        // SAFETY: `device` is NUL-terminated, `dm` is a complete DEVMODEW from the driver.
        let result = unsafe {
            ChangeDisplaySettingsExW(device.as_ptr(), &dm, ptr::null_mut(), 0, ptr::null())
        };
        if result != DISP_CHANGE_SUCCESSFUL {
            bail!("Windows refused the mode {} (error {result})", mode.id);
        }
        Ok(())
    }
}
