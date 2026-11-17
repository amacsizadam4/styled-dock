use std::{
    mem::size_of,
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        },
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
            WindowsAndMessaging::{FindWindowW, GetWindowRect, IsWindowVisible},
        },
    },
    core::{Result, w},
};

fn main() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let bar = unsafe { FindWindowW(w!("StyledDockTopBar"), None)? };
    if std::env::args().any(|arg| arg == "--close") {
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(bar),
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                Default::default(),
                Default::default(),
            )?;
        }
        return Ok(());
    }
    let started = Instant::now();
    let once = std::env::args().any(|arg| arg == "--once");
    let mut previous = String::new();
    let mut changes = 0;
    for sample in 0..if once { 1 } else { 400 } {
        if matches!(sample, 20 | 80 | 140 | 200 | 260 | 320)
            && std::env::args().any(|arg| arg == "--refresh-tray")
        {
            unsafe {
                let bridge = FindWindowW(w!("Shell_TrayWnd"), w!("Styled Dock Tray Bridge"))?;
                windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(bridge),
                    windows::Win32::UI::WindowsAndMessaging::WM_APP + 31,
                    Default::default(),
                    Default::default(),
                )?;
            }
        }
        let mut rect = RECT::default();
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe {
            GetWindowRect(bar, &mut rect)?;
            GetMonitorInfoW(MonitorFromWindow(bar, MONITOR_DEFAULTTONEAREST), &mut info).ok()?;
        }
        let dock = unsafe { FindWindowW(w!("StyledDockWindow"), None)? };
        let mut dock_rect = RECT::default();
        unsafe { GetWindowRect(dock, &mut dock_rect)? };
        let current = format!(
            "visible={} bounds={rect:?} work={:?} screen={:?} dock={dock_rect:?}",
            unsafe { IsWindowVisible(bar).as_bool() },
            info.rcWork,
            info.rcMonitor,
        );
        if current != previous {
            println!("{:>5}ms {current}", started.elapsed().as_millis());
            previous = current;
            changes += 1;
        }
        thread::sleep(Duration::from_millis(25));
    }
    println!(
        "Observed {changes} distinct placement states over {:?}",
        started.elapsed()
    );
    Ok(())
}
