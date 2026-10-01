# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

FreqFlip switches a display's refresh rate (and optionally its resolution) for as long as a given process runs, then restores the original mode. It returns to the caller once the rate is switched and waits in the background with a tray icon. A single Rust crate at the repository root, for Windows, macOS and Linux. Project structure, CI/CD and scripts come from the sibling project B.L.A.S.T. (`tbreckle/blast`).

## Build Commands

```bash
cargo build                  # debug build
cargo build --release        # release build (LTO, opt-level "z", panic=abort, stripped)
cargo test                   # unit tests
cargo clippy --all-targets -- -D warnings   # lint (warnings are CI failures)
cargo fmt --check            # CI formatting check
# Cross-check the other platforms' backends (needs `rustup target add ...`):
cargo clippy --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo clippy --all-targets --target aarch64-apple-darwin -- -D warnings
```
No system libraries are needed on Linux: x11rb, zbus and the KSNI tray are pure Rust. The Windows build embeds `images/freqflip.ico` via `winresource` (needs `rc.exe`; on other hosts `build.rs` only warns).

Icons/logo: `python3 images/generate.py` (Pillow) regenerates `icon.png`, `freqflip.ico`, `tray.png`, `tray_preview.png` and `logo.png`. `tray.png` is embedded with `include_bytes!` and must stay 8-bit RGBA (a test checks it).

## Architecture

Flow (`app.rs::run`): parse `MODE` (`display::ModeRequest`: `60`, `1920x1080`, `640x480i@60`) and target (`process::Target`: all digits = PID, otherwise a name) → `backend::open` → `select_display` → `pick_mode` (without a resolution: same resolution/scan type/`variant`; with one: that resolution, same `variant` preferred; closest rate within `--tolerance`, or closest to the current rate without one) → `set_mode` → `Reporter::ready` → `event_loop::run` with the `App` handler until the process exits, the tray's exit entry is clicked, the start timeout expires or a signal arrives → `Flip::restore` (also runs on drop).

- **Background start** (`report.rs`): without `--foreground`, the process re-executes itself with `FREQFLIP_BACKGROUND=1`, stdout piped, stderr null, detached (`setsid` / `DETACHED_PROCESS`). The child reports `info|ready|error <msg>` lines on stdout; the parent prints them and exits with 0 on `ready`, 1 on `error`. After `ready` the child never writes stdout again (the pipe is gone); `--log` is the only output then.
- **Event loop** (`event_loop.rs`): Windows/macOS use tao (tray-icon needs the native loop on the main thread; on macOS the tray must be created in `StartCause::Init`, hence `Handler::start`). Linux uses a plain mpsc loop, because tray-icon's `ksni` feature runs the StatusNotifierItem on its own D-Bus thread (no GTK). Menu events, the process watcher and the ctrlc handler all send `AppEvent`s through `EventSender`.
- **Backends** (`backend/`, trait `display::Backend`): `Mode::id` is backend-specific (X11 mode XID, Mutter mode id, kscreen mode id, wlr-randr `--mode` string, macOS `ioModeId:WxH` in points); `Mode::variant` holds what must not change with the rate and is kept if possible with the resolution (Windows bpp, macOS HiDPI scale like `2.00x`). Messages and tray show resolutions only when they change (`format_mode`). Linux auto-detection (`backend::detect`): Wayland → GNOME/KDE/wlroots by `XDG_CURRENT_DESKTOP`, else X11 (XRandR under Wayland only sees XWayland).
  - GNOME: `ApplyMonitorsConfig` needs the full layout; `set_mode` re-sends the current logical monitors with our connector's mode changed, method 1 (temporary). On a size change our logical monitor gets a supported scale (original scale remembered for the restore) and monitors right of/below it are shifted (`shift`), since Mutter rejects gaps.
  - X11: on a size change the screen is grown before `SetCrtcConfig` and shrunk after it to the bounding box of all CRTCs, keeping the DPI.
  - Windows: modes from GDI `EnumDisplaySettingsW` (whole-number rates), set via CCD `SetDisplayConfig` without `SDC_SAVE_TO_DATABASE` unless `--save` (then also `CDS_UPDATEREGISTRY` in the fallback, and `Flip::restore` always re-applies the original so it is saved back; needed for emulators that call `ChangeDisplaySettings(NULL, 0)`) (source mode gets the resolution, target mode index invalidated, `SDC_ALLOW_CHANGES`), then verified against the target mode's `vSyncFreq` and the GDI resolution. `ChangeDisplaySettingsExW` (flags 0) is only a fallback: it can report success and read back the new rate while the monitor stays unchanged. The exact original rate is remembered for the restore.
  - macOS: `kCGConfigureForAppOnly`, so macOS reverts when the process ends.
- **Process watching** (`process.rs`, sysinfo): names match case-insensitively without `.exe` against name, exe file name and argv[0] (Linux truncates names to 15 chars). Zombies count as exited; name targets need `EXIT_GRACE` of absence; PID targets compare the start time against PID reuse.
- **Tray** (`tray.rs`): the tooltip is capped at 127 chars (Windows); the same lines are disabled menu items because many Linux trays don't show tooltips.

## Conventions

- No `unwrap()` without justification; `anyhow::Result` with `context`. `unsafe` blocks carry `// SAFETY:` comments.
- Platform code is gated with `#[cfg(target_os = ...)]`; keep every target clippy-clean (CI runs clippy on all three).
- Pure logic (mode selection, parsing, formatting) lives outside the backends and is unit-tested.

## Development Workflow & CI/CD

GitFlow as in B.L.A.S.T.: `feature/*`/`bugfix/*` from `develop`, releases via `release-start.yml` / `release-finish.yml` (see `GITFLOW.md`, `.github/WORKFLOWS.md`). `build.yml` builds Linux x86_64, Windows x86_64 and a macOS universal binary, runs clippy per platform, fmt and the script tests.

Versioning: `scripts/version.sh` (tag `vX.Y.Z` → `X.Y.Z`, `release/X.Y.Z` → `X.Y.Z-rc.N`, else `0.0.0+<sha>`), injected as `FREQFLIP_VERSION` via `build.rs`. `Cargo.toml` stays at `0.0.0`. Add changes to `## [Unreleased]` in CHANGELOG.md.
