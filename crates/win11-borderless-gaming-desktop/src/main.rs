#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod behavior;
mod display;
mod gui;
mod sound;

use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
        System::Threading::CreateMutexW,
        UI::WindowsAndMessaging::{FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow},
    },
    core::{Result as WindowsResult, w},
};

const APP_TITLE: windows::core::PCWSTR = w!("Borderless Gaming Desktop");
const INSTANCE_MUTEX_NAME: windows::core::PCWSTR =
    w!(r"Local\syl20bnr.win11-borderless-gaming-desktop");

struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn acquire_instance() -> WindowsResult<Option<InstanceGuard>> {
    let handle = unsafe { CreateMutexW(None, false, INSTANCE_MUTEX_NAME) }?;
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        let _ = unsafe { CloseHandle(handle) };
        show_existing_instance();
        Ok(None)
    } else {
        Ok(Some(InstanceGuard(handle)))
    }
}

fn show_existing_instance() {
    if let Ok(window) = unsafe { FindWindowW(None, APP_TITLE) } {
        let _ = unsafe { ShowWindow(window, SW_RESTORE) };
        let _ = unsafe { SetForegroundWindow(window) };
    }
}

fn show_startup_error(error: &dyn std::fmt::Display) {
    use windows::{
        Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
        core::{HSTRING, w},
    };

    let message = HSTRING::from(format!(
        "Borderless Gaming Desktop could not start.\n\n{error}"
    ));
    let _ = unsafe {
        MessageBoxW(
            None,
            &message,
            w!("Borderless Gaming Desktop"),
            MB_OK | MB_ICONERROR,
        )
    };
}

fn main() -> std::process::ExitCode {
    let _instance_guard = match acquire_instance() {
        Ok(Some(guard)) => guard,
        Ok(None) => return std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Could not enforce single-instance mode: {error}");
            show_startup_error(&error);
            return std::process::ExitCode::FAILURE;
        }
    };

    if let Err(error) = gui::run() {
        eprintln!("Could not start the GUI: {error}");
        show_startup_error(&error);
        return std::process::ExitCode::FAILURE;
    }

    std::process::ExitCode::SUCCESS
}
