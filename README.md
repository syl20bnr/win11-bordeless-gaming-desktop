# win11-taskbar-autohide

Small Rust utility that toggles Windows taskbar auto-hide using the official `SHAppBarMessage` Win32 API.

## What it does

On each run, it:

1. Reads the current taskbar state (`ABM_GETSTATE`).
2. Toggles `ABS_AUTOHIDE` on/off.
3. Writes the updated state (`ABM_SETSTATE`).

## Build

```bash
cargo build --release
```

The binary will be at `target/release/win11-taskbar-autohide.exe` (when built on Windows).

## App icon setup (Windows executable)

This project now supports embedding a custom icon in the Windows `.exe` via `build.rs`.

1. Download an icon/image from:
   - https://github.com/microsoft/fluentui-system-icons/blob/main/assets/Panel%20Left%20Contract/SVG/ic_fluent_panel_left_contract_24_filled.svg
2. Convert it to `.ico` (for example, 256x256).
3. Add your icons as:
   - `assets/taskbar-autohide-disabled.ico`
   - `assets/taskbar-autohide-enabled.ico`
4. Build on Windows with `cargo build --release`.

`build.rs` embeds `assets/taskbar-autohide-disabled.ico` as the executable icon (with backward-compatible fallback to `assets/taskbar-autohide.ico`).

## Usage

Run the executable to toggle auto-hide.

You can pin the `.exe` to the taskbar for one-click toggling.

## Notes

- Uses the documented appbar API; no Explorer restart required.
- Historically, some Windows 11 builds had appbar-related regressions. Current Windows 11 versions generally behave as expected.
