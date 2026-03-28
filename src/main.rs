#[cfg(target_os = "windows")]
mod app {
    use std::{mem::size_of, path::PathBuf};
    use windows::core::PCWSTR;
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA},
        UI::WindowsAndMessaging::{
            GetConsoleWindow, LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON,
            LR_DEFAULTSIZE, LR_LOADFROMFILE, WM_SETICON,
        },
    };

    fn get_icon_paths() -> (PathBuf, PathBuf) {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        (
            exe_dir.join("assets").join("taskbar-enable-autohide.ico"),
            exe_dir.join("assets").join("taskbar-disable-autohide.ico"),
        )
    }

    fn set_dynamic_icon(can_enable_autohide: bool) {
        let hwnd = unsafe { GetConsoleWindow() };
        if hwnd.is_invalid() {
            return;
        }

        let (enable_icon, disable_icon) = get_icon_paths();
        let icon_path = if can_enable_autohide {
            enable_icon
        } else {
            disable_icon
        };

        if !icon_path.exists() {
            return;
        }

        let mut wide: Vec<u16> = icon_path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .collect();
        wide.push(0);

        let icon = unsafe {
            LoadImageW(
                None,
                PCWSTR(wide.as_ptr()),
                IMAGE_ICON,
                0,
                0,
                LR_LOADFROMFILE | LR_DEFAULTSIZE,
            )
        };

        if icon.is_invalid() {
            return;
        }

        unsafe {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_SMALL as usize),
                LPARAM(icon.0),
            );
            let _ = SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_BIG as usize), LPARAM(icon.0));
        }
    }

    pub fn run() {
        unsafe {
            let mut appbar = APPBARDATA {
                cbSize: size_of::<APPBARDATA>() as u32,
                ..Default::default()
            };

            let current_state = SHAppBarMessage(ABM_GETSTATE, &mut appbar) as u32;
            let should_disable_autohide = current_state & ABS_AUTOHIDE != 0;
            set_dynamic_icon(!should_disable_autohide);
            let toggled_state = if should_disable_autohide {
                current_state & !ABS_AUTOHIDE
            } else {
                current_state | ABS_AUTOHIDE
            };

            appbar.lParam = LPARAM(toggled_state as isize);
            let _ = SHAppBarMessage(ABM_SETSTATE, &mut appbar);
        }
    }
}

#[cfg(target_os = "windows")]
fn main() {
    app::run();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("This utility only runs on Windows.");
}
