<div align="center">
  <img
    src="https://raw.githubusercontent.com/syl20bnr/win11-borderless-gaming-desktop/refs/heads/main/crates/win11-borderless-gaming-desktop/assets/readme-logo-master.png"
    width="680"
    alt="Borderless Gaming Desktop logo"
  >

  <h1>Borderless Gaming Desktop</h1>

  <p><strong>The perfect gaming XP in one click.</strong></p>
  <p>Built for Ultrawide monitors: work wide, then game focused. 🎮</p>

  <p>
    <a href="https://github.com/syl20bnr/win11-borderless-gaming-desktop/releases/latest">
      <img src="https://img.shields.io/badge/Download_for_Windows-7062FF?style=for-the-badge&logo=windows11&logoColor=white" alt="Download for Windows">
    </a>
    <img src="https://img.shields.io/badge/Portable-no_installer-4ADE80?style=for-the-badge" alt="Portable application">
  </p>

  <p>
    <a href="https://github.com/syl20bnr/win11-borderless-gaming-desktop/releases/latest">
      <img src="https://img.shields.io/github/v/release/syl20bnr/win11-borderless-gaming-desktop?style=flat-square&color=7062FF" alt="Latest release">
    </a>
    <img src="https://img.shields.io/badge/Windows-11-0078D4?style=flat-square&logo=windows11&logoColor=white" alt="Windows 11">
    <a href="https://github.com/syl20bnr/win11-borderless-gaming-desktop/blob/main/LICENSE">
      <img src="https://img.shields.io/badge/license-GPL--3.0-4ADE80?style=flat-square" alt="GPL-3.0 license">
    </a>
  </p>
</div>

---

Borderless Gaming Desktop gives your Ultrawide Windows 11 battlestation two
loadouts: a comfortable **Desktop Mode** and a focused **Gaming Mode**. Pick a
Gaming resolution, choose your cleanup actions, and switch with one purple
button. It is portable, needs no installer, and changes nothing just because you
opened it.

### Ready, player one?

1. [Download the latest portable `.exe`](https://github.com/syl20bnr/win11-borderless-gaming-desktop/releases/latest).
2. Put it wherever you like and run it.
3. Choose your Gaming resolution and actions, then press **Activate gaming mode**.

<p align="center">
  <img
    src="https://raw.githubusercontent.com/syl20bnr/win11-borderless-gaming-desktop/refs/heads/main/crates/win11-borderless-gaming-desktop/assets/gui.png"
    width="531"
    alt="Borderless Gaming Desktop in Gaming Mode with gaming options, startup, and transparency controls"
  >
</p>

### Pick your loadout

Gaming Mode can:

- switch the primary monitor to your chosen resolution when **Change resolution** is checked
- switch Windows to a chosen audio output device, then restore the previous output in Desktop Mode
- auto-hide the taskbar
- hide desktop icons
- replace the wallpaper with solid black
- minimize open windows

Every action is optional. Launch the app first at your everyday resolution; that
becomes Desktop Mode. The selected Gaming resolution is remembered even when
**Change resolution** is unchecked, and choosing it does not apply it immediately.
Press **Restore desktop mode** to bring back the captured resolution and
reversible desktop settings. Windows minimized during activation stay minimized.

Your loadout is saved automatically. Sounds, start-at-login behavior, minimized
login launches, and window transparency have their own controls too.

### Two modes. One purple button.

|                | Desktop Mode 🖥️               | Gaming Mode 🎮                |
|----------------|--------------------------------|-------------------------------|
| **Resolution** | Captured Desktop resolution    | Chosen Gaming resolution      |
| **Audio output** | Previous output restored     | Chosen Gaming output          |
| **Taskbar**    | Original setting restored      | Auto-hide when selected       |
| **Desktop**    | Icons and wallpaper restored   | Selected cleanup actions      |
| **Status LED** | Cool gray                      | Bright green                  |

### Stays out of your way

- Closing the window or pressing `Alt+F4` hides the app in the system tray.
- A normal Windows minimize still minimizes to the taskbar.
- Double-clicking the tray icon switches modes.
- Right-clicking it opens the app, switches modes, quits, and shows your loadout.
- Only one instance runs; launching it again brings the existing window forward.

> [!TIP]
> **Quit** closes the app without changing modes. Restore Desktop Mode first if
> you want your everyday setup back.

## Build it yourself

Requirements: **Windows 11** and **Rust 1.94 or newer**.

Install the published crate:

```powershell
cargo install win11-borderless-gaming-desktop
```

Or build the repository:

```powershell
cargo build --release --package win11-borderless-gaming-desktop
```

The executable is written to:

```text
target/release/win11-borderless-gaming-desktop.exe
```

### Developer power-ups

Build and launch the app:

```powershell
cargo xtask run
```

Regenerate embedded media and icons:

```powershell
cargo xtask assets
```

Run the tests and repository checks before sending your build into ranked:

```powershell
cargo test --workspace
cargo xtask check all
```

`cargo xtask check all` covers formatting, strict Clippy, dependency auditing,
and typo checks.

### License

Borderless Gaming Desktop is free software released under the [GNU General Public License v3.0 or later](LICENSE).
