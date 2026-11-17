#![windows_subsystem = "windows"]

mod config;
mod dock;
mod i18n;
mod platform;
mod topbar;
mod ui;

// Keep shared services available under one crate path across UI modules.
use dock::{animation, downloads, layout, render, window};
use platform::{files, lifecycle, placement, shell, startup, system, taskbar, tray, watcher};
use topbar::{bar_render, bar_status, panels};
use ui::{app_picker, icons, popup, settings, theme};

use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
use windows::core::{HSTRING, w};

fn main() -> std::process::ExitCode {
    i18n::configure("auto");
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("--taskbar-watch") => {
            return if taskbar::watch(args).is_ok() {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::FAILURE
            };
        }
        Some("--restore-taskbar") => lifecycle::stop().and_then(|()| taskbar::restore()),
        Some("--quit") => lifecycle::stop(),
        Some("--settings") => settings::open_standalone(),
        _ => window::run(),
    };
    if let Err(error) = result {
        let message = HSTRING::from(crate::tr!(
            "Could not run Styled Dock.\n\n{error}",
            error = error
        ));
        unsafe { MessageBoxW(None, &message, w!("Styled Dock"), MB_ICONERROR) };
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
