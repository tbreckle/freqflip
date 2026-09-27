//! Tray icon with a status tooltip and menu.
//!
//! Linux trays (StatusNotifierItem) often don't show tooltips, so the same information is
//! also in disabled menu entries.

use std::io::Cursor;

use anyhow::{bail, Context, Result};
use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::display::format_hz;

const ICON_PNG: &[u8] = include_bytes!("../images/tray.png");
/// Windows cuts tooltips off at 127 characters.
const TOOLTIP_MAX_CHARS: usize = 127;

/// Where the watched process is at.
#[derive(Clone, Debug, PartialEq)]
pub enum ProcessState {
    /// A named process that hasn't started yet.
    Waiting(String),
    Running {
        name: String,
        pid: u32,
    },
}

/// Everything the tray shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub display: String,
    pub target: f64,
    pub original: f64,
    pub current: f64,
    pub process: ProcessState,
}

impl Status {
    pub fn target_line(&self) -> String {
        format!("Target: {}", format_hz(self.target))
    }

    pub fn original_line(&self) -> String {
        format!("Original: {}", format_hz(self.original))
    }

    pub fn current_line(&self) -> String {
        format!("Current: {}", format_hz(self.current))
    }

    pub fn process_line(&self) -> String {
        match &self.process {
            ProcessState::Waiting(name) => format!("Waiting for {name} to start"),
            ProcessState::Running { name, pid } => format!("Watching {name} (PID {pid})"),
        }
    }

    pub fn exit_label(&self) -> String {
        format!("Restore {} and exit", format_hz(self.original))
    }

    pub fn tooltip(&self) -> String {
        let text = [
            format!("FreqFlip · {}", self.display),
            self.target_line(),
            self.original_line(),
            self.current_line(),
            self.process_line(),
        ]
        .join("\n");
        text.chars().take(TOOLTIP_MAX_CHARS).collect()
    }
}

pub struct Tray {
    icon: TrayIcon,
    // Kept alive for the tray; the items below are updated in place.
    _menu: Menu,
    current: MenuItem,
    process: MenuItem,
    exit: MenuItem,
}

fn info_item(text: &str) -> MenuItem {
    MenuItem::new(text, false, None)
}

fn load_icon() -> Result<Icon> {
    let decoder = png::Decoder::new(Cursor::new(ICON_PNG));
    let mut reader = decoder.read_info().context("invalid tray icon")?;
    let mut rgba = vec![0; reader.output_buffer_size().context("tray icon too large")?];
    let frame = reader.next_frame(&mut rgba).context("invalid tray icon")?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        bail!("the tray icon must be an 8-bit RGBA PNG");
    }
    rgba.truncate(frame.buffer_size());
    Ok(Icon::from_rgba(rgba, frame.width, frame.height)?)
}

impl Tray {
    pub fn new(status: &Status) -> Result<Self> {
        let current = info_item(&status.current_line());
        let process = info_item(&status.process_line());
        let exit = MenuItem::new(status.exit_label(), true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &info_item(&format!("FreqFlip {}", env!("FREQFLIP_VERSION"))),
            &PredefinedMenuItem::separator(),
            &info_item(&format!("Display: {}", status.display)),
            &info_item(&status.target_line()),
            &info_item(&status.original_line()),
            &current,
            &process,
            &PredefinedMenuItem::separator(),
            &exit,
        ])?;
        let icon = TrayIconBuilder::new()
            .with_id("freqflip")
            .with_title("FreqFlip")
            .with_tooltip(status.tooltip())
            .with_icon(load_icon()?)
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(true)
            .build()
            .context("could not create the tray icon")?;
        Ok(Self {
            icon,
            _menu: menu,
            current,
            process,
            exit,
        })
    }

    pub fn update(&self, status: &Status) {
        self.current.set_text(status.current_line());
        self.process.set_text(status.process_line());
        let _ = self.icon.set_tooltip(Some(status.tooltip()));
    }

    pub fn is_exit(&self, id: &MenuId) -> bool {
        self.exit.id() == id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status {
            display: "DP-1".into(),
            target: 60.0,
            original: 164.956,
            current: 59.951,
            process: ProcessState::Waiting("mame".into()),
        }
    }

    #[test]
    fn formats_menu_lines() {
        let mut s = status();
        assert_eq!(s.target_line(), "Target: 60 Hz");
        assert_eq!(s.original_line(), "Original: 164.96 Hz");
        assert_eq!(s.current_line(), "Current: 59.95 Hz");
        assert_eq!(s.process_line(), "Waiting for mame to start");
        assert_eq!(s.exit_label(), "Restore 164.96 Hz and exit");
        s.process = ProcessState::Running {
            name: "mame".into(),
            pid: 42,
        };
        assert_eq!(s.process_line(), "Watching mame (PID 42)");
    }

    #[test]
    fn tooltip_fits_windows_limit() {
        let mut s = status();
        assert_eq!(
            s.tooltip(),
            "FreqFlip · DP-1\nTarget: 60 Hz\nOriginal: 164.96 Hz\nCurrent: 59.95 Hz\nWaiting for mame to start"
        );
        s.process = ProcessState::Waiting("x".repeat(300));
        assert_eq!(s.tooltip().chars().count(), TOOLTIP_MAX_CHARS);
    }

    #[test]
    fn embedded_icon_is_valid() {
        let icon = load_icon();
        assert!(icon.is_ok(), "{:?}", icon.err());
    }
}
