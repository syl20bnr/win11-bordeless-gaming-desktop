# win11-borderless-gaming-desktop

A tiny utility to make Windows cleaner for borderless gaming.

## Demo

![Demo](https://raw.githubusercontent.com/syl20bnr/win11-borderless-gaming-desktop/refs/heads/main/crates/win11-borderless-gaming-desktop/assets/demo.gif)

## Install

### Portable pre-build from GitHub release

Download the latest portable version:

[win11-borderless-gaming-desktop-portable.exe](https://github.com/syl20bnr/win11-borderless-gaming-desktop/releases/download/v1.0.0/win11-borderless-gaming-desktop.exe)

Place the executable somewhere on your disk and then drag and drop it to the taskbar for easy toggle.

### With Cargo (for Rust developers)

```sh
cargo install win11-borderless-gaming-desktop
```

Then the executable will be located in cargo's `bin` directory. Drag and drop it to the taskbar for easy toggle.

## What it does

Run the app to toggle:
- taskbar auto-hide
- desktop icons
- desktop background image (replaced with solid black when enabled)

## Usage

Run the executable to switch between normal desktop mode and gaming mode.

You can pin it to the taskbar for one-click access.

## Build

Default build includes all features:

```bash
cargo build --release
```

## Features

These features are enabled by default and can be turned off:

- `desktop-icons`: toggles desktop icons
- `desktop-background`: toggles the desktop background image

### Examples

Build without desktop icons:

```bash
cargo build --release --no-default-features --features desktop-background
```

Build without desktop background handling:

```bash
cargo build --release --no-default-features --features desktop-icons
```

Build with only taskbar auto-hide:

```bash
cargo build --release --no-default-features
```
