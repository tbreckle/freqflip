//! Windows backend: displays and modes come from GDI (EnumDisplaySettingsW), rates are set
//! through the CCD API (SetDisplayConfig) like the Settings app does.
//!
//! ChangeDisplaySettingsExW is only a fallback: on current Windows it can report success, and
//! even read back the new rate, while the monitor keeps running at the old one. So every switch
//! is checked against the signal timing CCD reports.
//!
//! Nothing is saved to the display database / registry, so a reboot always brings back the
//! saved rate even if FreqFlip is killed.

use std::collections::HashMap;
use std::mem;
use std::ptr;

use anyhow::{bail, Context, Result};
use windows_sys::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, SetDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_MODE_INFO_TYPE_TARGET, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_RATIONAL,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS, SDC_ALLOW_CHANGES, SDC_APPLY,
    SDC_USE_SUPPLIED_DISPLAY_CONFIG,
};
use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
use windows_sys::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW,
    DISPLAYCONFIG_PATH_MODE_IDX_INVALID, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
    DISPLAY_DEVICE_PRIMARY_DEVICE, DISP_CHANGE_SUCCESSFUL, DM_DISPLAYFLAGS, DM_DISPLAYFREQUENCY,
    DM_INTERLACED, DM_PELSHEIGHT, DM_PELSWIDTH, ENUM_CURRENT_SETTINGS,
};

use crate::display::{format_hz, Backend, Display, Mode};

/// GDI rates are whole numbers (59.94 Hz shows up as 59), so the rate the monitor really runs
/// at may be up to this far off.
const RATE_TOLERANCE: f64 = 1.0;

#[derive(Default)]
pub struct Windows {
    /// Exact rate each device ran at before the first switch, with its GDI rate, so restoring
    /// brings back the same timing (e.g. 164.96 Hz, not whatever Windows picks for "165").
    originals: HashMap<String, (u32, DISPLAYCONFIG_RATIONAL)>,
}

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
        let device = format!("{DEVICE_PREFIX}{display}");
        let hz = mode.refresh.round() as u32;
        if !self.originals.contains_key(&device) {
            let rate = DisplayConfig::query()
                .ok()
                .and_then(|config| config.rational(&device));
            let gdi = enum_settings(&wide(&device), ENUM_CURRENT_SETTINGS);
            if let (Some(rate), Some(dm)) = (rate, gdi) {
                self.originals
                    .insert(device.clone(), (dm.dmDisplayFrequency, rate));
            }
        }
        let rates = match self.originals.get(&device) {
            Some(&(gdi_hz, rate)) if gdi_hz == hz => vec![rate],
            _ => candidates(hz).to_vec(),
        };

        let ccd_err = match ccd_set(&device, &rates, mode.refresh) {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };
        gdi_set(&device, mode).with_context(|| format!("{ccd_err:#}"))?;
        // Trust the signal timing, not the call's result (see the module docs).
        if let Some(actual) = DisplayConfig::query()
            .ok()
            .and_then(|config| config.refresh(&device))
        {
            if (actual - mode.refresh).abs() > RATE_TOLERANCE {
                bail!(
                    "Windows reports {} as set, but {display} still runs at {} ({ccd_err:#})",
                    format_hz(mode.refresh),
                    format_hz(actual)
                );
            }
        }
        Ok(())
    }
}

/// Exact rates a whole-number GDI rate may stand for: `hz` itself, then the NTSC-style
/// `(hz + 1) / 1.001` that GDI rounds down to `hz` (59.94 Hz is reported as 59).
fn candidates(hz: u32) -> [DISPLAYCONFIG_RATIONAL; 2] {
    [
        DISPLAYCONFIG_RATIONAL {
            Numerator: hz,
            Denominator: 1,
        },
        DISPLAYCONFIG_RATIONAL {
            Numerator: (hz + 1) * 1000,
            Denominator: 1001,
        },
    ]
}

fn to_hz(rate: DISPLAYCONFIG_RATIONAL) -> Option<f64> {
    (rate.Numerator > 0 && rate.Denominator > 0)
        .then(|| f64::from(rate.Numerator) / f64::from(rate.Denominator))
}

/// The active display paths (source = GDI device, target = monitor) and their modes.
struct DisplayConfig {
    paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    modes: Vec<DISPLAYCONFIG_MODE_INFO>,
}

impl DisplayConfig {
    fn query() -> Result<Self> {
        loop {
            let (mut path_count, mut mode_count) = (0u32, 0u32);
            // SAFETY: both out pointers point to live u32s.
            let err = unsafe {
                GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
            };
            if err != ERROR_SUCCESS {
                bail!("GetDisplayConfigBufferSizes failed (error {err})");
            }
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
            // SAFETY: the arrays hold exactly as many elements as the counts say; the topology
            // pointer must be null without QDC_DATABASE_CURRENT.
            let err = unsafe {
                QueryDisplayConfig(
                    QDC_ONLY_ACTIVE_PATHS,
                    &mut path_count,
                    paths.as_mut_ptr(),
                    &mut mode_count,
                    modes.as_mut_ptr(),
                    ptr::null_mut(),
                )
            };
            match err {
                ERROR_SUCCESS => {
                    paths.truncate(path_count as usize);
                    modes.truncate(mode_count as usize);
                    return Ok(Self { paths, modes });
                }
                // A display was plugged in between the two calls.
                ERROR_INSUFFICIENT_BUFFER => continue,
                err => bail!("QueryDisplayConfig failed (error {err})"),
            }
        }
    }

    /// Indexes of the paths showing GDI device `device` (several when cloned).
    fn paths_of(&self, device: &str) -> Vec<usize> {
        (0..self.paths.len())
            .filter(|&i| {
                source_name(&self.paths[i]).is_some_and(|name| name.eq_ignore_ascii_case(device))
            })
            .collect()
    }

    /// The exact rate of the first path showing `device`, as the target mode's signal timing.
    fn rational(&self, device: &str) -> Option<DISPLAYCONFIG_RATIONAL> {
        let path = &self.paths[*self.paths_of(device).first()?];
        // SAFETY: without QDC_VIRTUAL_MODE_AWARE the union holds the plain mode index.
        let index = unsafe { path.targetInfo.Anonymous.modeInfoIdx };
        let timing = self
            .modes
            .get(index as usize)
            .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_TARGET)
            // SAFETY: infoType says the union holds a target mode.
            .map(|m| unsafe { m.Anonymous.targetMode.targetVideoSignalInfo.vSyncFreq });
        timing
            .filter(|&rate| to_hz(rate).is_some())
            .or_else(|| to_hz(path.targetInfo.refreshRate).map(|_| path.targetInfo.refreshRate))
    }

    fn refresh(&self, device: &str) -> Option<f64> {
        self.rational(device).and_then(to_hz)
    }
}

/// The GDI device name (`\\.\DISPLAY1`) of a path's source.
fn source_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
    let mut name = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    name.header.size = mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
    name.header.adapterId = path.sourceInfo.adapterId;
    name.header.id = path.sourceInfo.id;
    // SAFETY: the header starts the struct and its size field covers the whole struct.
    let err = unsafe { DisplayConfigGetDeviceInfo(&mut name.header) };
    (err == 0).then(|| from_wide(&name.viewGdiDeviceName))
}

/// Sets the rate of `device` with SetDisplayConfig, trying `rates` in order until the monitor
/// runs at `want` Hz. The target mode is dropped from the path, so Windows picks the monitor
/// timing for the new rate.
fn ccd_set(device: &str, rates: &[DISPLAYCONFIG_RATIONAL], want: f64) -> Result<()> {
    let mut last_err = anyhow::anyhow!("no rate to try");
    for &rate in rates {
        let mut config = DisplayConfig::query()?;
        let indexes = config.paths_of(device);
        if indexes.is_empty() {
            bail!("no active display path for {device}");
        }
        for i in indexes {
            let path = &mut config.paths[i];
            path.targetInfo.refreshRate = rate;
            path.targetInfo.Anonymous.modeInfoIdx = DISPLAYCONFIG_PATH_MODE_IDX_INVALID;
        }
        // Without SDC_SAVE_TO_DATABASE the change is temporary.
        // SAFETY: the counts match the arrays, which come from QueryDisplayConfig.
        let err = unsafe {
            SetDisplayConfig(
                config.paths.len() as u32,
                config.paths.as_ptr(),
                config.modes.len() as u32,
                config.modes.as_ptr(),
                SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES,
            )
        };
        if err != 0 {
            last_err = anyhow::anyhow!("SetDisplayConfig refused the rate (error {err})");
            continue;
        }
        match DisplayConfig::query()?.refresh(device) {
            Some(actual) if (actual - want).abs() > RATE_TOLERANCE => {
                last_err = anyhow::anyhow!(
                    "SetDisplayConfig picked {} instead of {}",
                    format_hz(actual),
                    format_hz(want)
                );
            }
            _ => return Ok(()),
        }
    }
    Err(last_err)
}

/// Fallback: the legacy GDI mode change, not saved to the registry.
fn gdi_set(device: &str, mode: &Mode) -> Result<()> {
    let device = wide(device);
    let Some(mut dm) = enum_settings(&device, ENUM_CURRENT_SETTINGS) else {
        bail!("display not found");
    };
    dm.dmPelsWidth = mode.width;
    dm.dmPelsHeight = mode.height;
    dm.dmDisplayFrequency = mode.refresh.round() as u32;
    dm.Anonymous2.dmDisplayFlags = if mode.interlaced { DM_INTERLACED } else { 0 };
    dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_DISPLAYFLAGS;
    // SAFETY: `device` is NUL-terminated, `dm` is a complete DEVMODEW from the driver.
    let result =
        unsafe { ChangeDisplaySettingsExW(device.as_ptr(), &dm, ptr::null_mut(), 0, ptr::null()) };
    if result != DISP_CHANGE_SUCCESSFUL {
        bail!("Windows refused the mode {} (error {result})", mode.id);
    }
    Ok(())
}
