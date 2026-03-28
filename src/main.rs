#[cfg(target_os = "windows")]
mod app {
    use std::mem::size_of;
    use windows::Win32::{
        Foundation::LPARAM,
        UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA},
    };

    pub fn run() {
        unsafe {
            let mut appbar = APPBARDATA {
                cbSize: size_of::<APPBARDATA>() as u32,
                ..Default::default()
            };

            let current_state = SHAppBarMessage(ABM_GETSTATE, &mut appbar) as u32;
            let toggled_state = if current_state & ABS_AUTOHIDE != 0 {
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
