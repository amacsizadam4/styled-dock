use std::mem::{size_of, size_of_val};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Error, Result};

use crate::layout::DockSize;
use crate::{config, layout};

pub const WM_APPBAR: u32 = WM_APP + 4;
pub const WM_REPOSITION: u32 = WM_APP + 5;
pub const WM_FULLSCREEN_CHECK: u32 = WM_APP + 6;
pub const WM_SHELL_RESTART: u32 = WM_APP + 7;
pub const WM_DESKTOP_RESUMED: u32 = WM_APP + 13;

pub struct Placement {
    hwnd: HWND,
    monitor: String,
    monitor_handle: HMONITOR,
    appbar: Option<AppBar>,
    bounds: RECT,
    dock: DockSize,
    auto_hide: bool,
}

impl Placement {
    pub fn new(
        hwnd: HWND,
        settings: &config::Placement,
        dock: DockSize,
        count: usize,
    ) -> Result<Self> {
        let monitor = if settings.monitor.eq_ignore_ascii_case("cursor") {
            let mut point = Default::default();
            unsafe { GetCursorPos(&mut point)? };
            let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY) };
            device_name(&monitor_info(monitor)?)
        } else {
            settings.monitor.clone()
        };
        let mut state = Self {
            hwnd,
            monitor,
            monitor_handle: HMONITOR::default(),
            appbar: None,
            bounds: RECT::default(),
            dock,
            auto_hide: settings.auto_hide,
        };
        state.update(count)?;
        if settings.reserve_space {
            state.appbar = Some(AppBar::new(hwnd, ABE_BOTTOM)?);
            state.update(count)?;
        }
        Ok(state)
    }

    pub fn explorer_restarted(&mut self) -> Result<()> {
        if let Some(appbar) = &mut self.appbar {
            appbar.reregister()?;
        }
        Ok(())
    }

    pub fn monitor(&self) -> HMONITOR {
        self.monitor_handle
    }

    pub fn set_dock(&mut self, dock: DockSize, count: usize) -> Result<()> {
        self.dock = dock;
        self.update(count)
    }

    pub fn update(&mut self, count: usize) -> Result<()> {
        let monitor = select_monitor(&self.monitor);
        let info = monitor_info(monitor)?.monitorInfo;
        unsafe {
            // Move onto the selected monitor before reading this PMv2 window's DPI.
            if self.monitor_handle != monitor {
                self.monitor_handle = monitor;
                self.bounds = RECT::default();
                SetWindowPos(
                    self.hwnd,
                    None,
                    info.rcMonitor.left,
                    info.rcMonitor.top,
                    1,
                    1,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )?;
            }
            let dpi = GetDpiForWindow(self.hwnd) as f32;
            let scale = |dips: f32| (dips * dpi / 96.0).round() as i32;
            let band = self.dock.panel_height() + layout::BOTTOM_GAP;
            let mut work = info.rcWork;
            if self.auto_hide {
                // The reveal strip must stay at the physical monitor edge,
                // including while Explorer temporarily restores its taskbar.
                work.bottom = info.rcMonitor.bottom;
            }
            if let Some(appbar) = &mut self.appbar {
                work = appbar.reserve(info.rcMonitor, scale(band))?;
            }
            let width = scale(self.dock.panel_width(count) + self.dock.side_room())
                .min(work.right - work.left)
                .max(1);
            let height = scale(band + self.dock.headroom())
                .min(work.bottom - info.rcMonitor.top)
                .max(1);
            let x = work.left + (work.right - work.left - width) / 2;
            let bounds = RECT {
                left: x,
                top: work.bottom - height,
                right: x + width,
                bottom: work.bottom,
            };
            if bounds != self.bounds {
                SetWindowPos(
                    self.hwnd,
                    None,
                    bounds.left,
                    bounds.top,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )?;
                self.bounds = bounds;
                if let Some(appbar) = &self.appbar {
                    SHAppBarMessage(ABM_WINDOWPOSCHANGED, &mut appbar.data());
                }
            }
        }
        Ok(())
    }

    pub fn fullscreen(&self) -> bool {
        fullscreen_window(
            unsafe { GetForegroundWindow() },
            self.hwnd,
            self.monitor_handle,
        )
    }

    pub fn reveal_at(&self, point: POINT) -> bool {
        // Use screen coordinates and an exclusive bottom bound so a monitor
        // directly below this one cannot accidentally reveal the dock.
        at_reveal_edge(self.bounds, point)
    }

    pub fn show(&self) -> Result<()> {
        show_topmost(self.hwnd)
    }
}

pub(crate) fn show_topmost(hwnd: HWND) -> Result<()> {
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    }
}

fn at_reveal_edge(bounds: RECT, point: POINT) -> bool {
    point.x >= bounds.left
        && point.x < bounds.right
        && point.y >= bounds.bottom - 3
        && point.y < bounds.bottom
}

fn fullscreen_window(foreground: HWND, dock: HWND, monitor: HMONITOR) -> bool {
    unsafe {
        if foreground.0.is_null()
            || foreground == dock
            || foreground == GetShellWindow()
            || IsIconic(foreground).as_bool()
            || MonitorFromWindow(foreground, MONITOR_DEFAULTTONULL) != monitor
        {
            return false;
        }
        // Notification suppression is global shell state and can outlive a
        // video transition. Inspect the foreground window's current bounds.
        let style = WINDOW_STYLE(GetWindowLongPtrW(foreground, GWL_STYLE) as u32);
        if IsZoomed(foreground).as_bool() && style.contains(WS_CAPTION) {
            return false;
        }
        let Ok(info) = monitor_info(monitor) else {
            return false;
        };
        let mut bounds = RECT::default();
        if DwmGetWindowAttribute(
            foreground,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut bounds).cast(),
            size_of_val(&bounds) as u32,
        )
        .is_err()
            && GetWindowRect(foreground, &mut bounds).is_err()
        {
            return false;
        }
        let screen = info.monitorInfo.rcMonitor;
        bounds.left <= screen.left
            && bounds.top <= screen.top
            && bounds.right >= screen.right
            && bounds.bottom >= screen.bottom
    }
}

pub(crate) fn monitor_info(monitor: HMONITOR) -> Result<MONITORINFOEXW> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo).ok()? };
    Ok(info)
}

fn device_name(info: &MONITORINFOEXW) -> String {
    let len = info
        .szDevice
        .iter()
        .position(|&ch| ch == 0)
        .unwrap_or(info.szDevice.len());
    String::from_utf16_lossy(&info.szDevice[..len])
}

pub fn display_names() -> Vec<String> {
    let mut names = Vec::new();
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        if let Ok(info) = monitor_info(monitor) {
            unsafe { (&mut *(data.0 as *mut Vec<String>)).push(device_name(&info)) };
        }
        BOOL(1)
    }
    let _ = unsafe {
        EnumDisplayMonitors(None, None, Some(collect), LPARAM((&raw mut names) as isize))
    };
    names.sort();
    names
}

pub(crate) fn select_monitor(name: &str) -> HMONITOR {
    let primary = unsafe { MonitorFromPoint(Default::default(), MONITOR_DEFAULTTOPRIMARY) };
    if name.eq_ignore_ascii_case("primary") {
        return primary;
    }
    let mut lookup = (name, primary);
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(find_monitor),
            LPARAM((&raw mut lookup) as isize),
        )
    };
    lookup.1
}

pub(crate) fn resolve_monitor_name(name: &str) -> Result<String> {
    let monitor = if name.eq_ignore_ascii_case("cursor") {
        let mut point = POINT::default();
        unsafe {
            GetCursorPos(&mut point)?;
            MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY)
        }
    } else {
        select_monitor(name)
    };
    Ok(device_name(&monitor_info(monitor)?))
}

unsafe extern "system" fn find_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let lookup = unsafe { &mut *(data.0 as *mut (&str, HMONITOR)) };
    if monitor_info(monitor).is_ok_and(|info| device_name(&info).eq_ignore_ascii_case(lookup.0)) {
        lookup.1 = monitor;
        return BOOL(0);
    }
    BOOL(1)
}

pub(crate) struct AppBar {
    hwnd: HWND,
    edge: u32,
    reserved: Option<RECT>,
}

impl AppBar {
    pub fn new(hwnd: HWND, edge: u32) -> Result<Self> {
        let mut bar = Self {
            hwnd,
            edge,
            reserved: None,
        };
        bar.register()?;
        Ok(bar)
    }

    fn data(&self) -> APPBARDATA {
        APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: self.hwnd,
            uCallbackMessage: WM_APPBAR,
            uEdge: self.edge,
            ..Default::default()
        }
    }

    fn register(&mut self) -> Result<()> {
        self.reserved = None;
        if unsafe { SHAppBarMessage(ABM_NEW, &mut self.data()) } == 0 {
            return Err(Error::new(
                windows::Win32::Foundation::E_FAIL,
                crate::i18n::text("Windows did not register the AppBar"),
            ));
        }
        Ok(())
    }

    pub fn reregister(&mut self) -> Result<()> {
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut self.data()) };
        self.register()
    }

    pub fn position_changed(&self) {
        unsafe { SHAppBarMessage(ABM_WINDOWPOSCHANGED, &mut self.data()) };
    }

    pub fn reserve(&mut self, monitor: RECT, height: i32) -> Result<RECT> {
        let mut data = self.data();
        data.rc = monitor;
        if self.edge == ABE_TOP {
            data.rc.bottom = monitor.top + height;
        } else {
            data.rc.top = monitor.bottom - height;
        }
        unsafe { SHAppBarMessage(ABM_QUERYPOS, &mut data) };
        if self.edge == ABE_TOP {
            data.rc.bottom = (data.rc.top + height).min(monitor.bottom);
        } else {
            data.rc.top = (data.rc.bottom - height).max(monitor.top);
        }
        // Explorer can recalculate rcWork while keeping our registered rectangle.
        // An unchanged cached position does not mean the reservation still exists.
        // Hidden appbars do not contribute to rcWork; wait until shown to repair
        // them, so their position notifications cannot cause a retry loop.
        let reservation_lost = if unsafe { IsWindowVisible(self.hwnd).as_bool() } {
            let display = unsafe { MonitorFromRect(&monitor, MONITOR_DEFAULTTONEAREST) };
            let work = monitor_info(display)?.monitorInfo.rcWork;
            if self.edge == ABE_TOP {
                work.top < data.rc.bottom
            } else {
                work.bottom > data.rc.top
            }
        } else {
            false
        };
        if self.reserved != Some(data.rc) || reservation_lost {
            if unsafe { SHAppBarMessage(ABM_SETPOS, &mut data) } == 0 {
                return Err(Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    crate::i18n::text("Windows did not reserve the AppBar's work area"),
                ));
            }
            self.reserved = Some(data.rc);
        }
        Ok(data.rc)
    }
}

impl Drop for AppBar {
    fn drop(&mut self) {
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut self.data()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;

    struct TestWindow(HWND);

    impl Drop for TestWindow {
        fn drop(&mut self) {
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }

    fn popup() -> Result<TestWindow> {
        Ok(TestWindow(unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                w!("STATIC"),
                w!("Styled Dock fullscreen recovery test"),
                WS_POPUP,
                0,
                0,
                200,
                100,
                None,
                None,
                None,
                None,
            )?
        }))
    }

    #[test]
    fn reveal_strip_excludes_adjacent_monitors_and_other_edges() {
        let bounds = RECT {
            left: -1200,
            top: -200,
            right: -400,
            bottom: -100,
        };
        assert!(at_reveal_edge(bounds, POINT { x: -800, y: -101 }));
        assert!(at_reveal_edge(bounds, POINT { x: -1200, y: -103 }));
        assert!(!at_reveal_edge(bounds, POINT { x: -800, y: -100 }));
        assert!(!at_reveal_edge(bounds, POINT { x: -800, y: -104 }));
        assert!(!at_reveal_edge(bounds, POINT { x: -1201, y: -101 }));
        assert!(!at_reveal_edge(bounds, POINT { x: -400, y: -101 }));
    }

    #[test]
    fn fullscreen_geometry_recovers_without_another_window_event() -> Result<()> {
        let window = popup()?;
        let monitor = unsafe { MonitorFromWindow(window.0, MONITOR_DEFAULTTOPRIMARY) };
        let screen = monitor_info(monitor)?.monitorInfo.rcMonitor;
        for _ in 0..3 {
            unsafe {
                SetWindowPos(
                    window.0,
                    None,
                    screen.left,
                    screen.top,
                    screen.right - screen.left,
                    screen.bottom - screen.top,
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
                )?;
                windows::Win32::Graphics::Dwm::DwmFlush()?;
            }
            assert!(fullscreen_window(window.0, HWND::default(), monitor));
            assert!(!fullscreen_window(window.0, window.0, monitor));
            unsafe {
                SetWindowPos(
                    window.0,
                    None,
                    screen.left + 20,
                    screen.top + 20,
                    200,
                    100,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )?;
                windows::Win32::Graphics::Dwm::DwmFlush()?;
            }
            // The periodic recovery path reads current geometry even if the
            // final location-change notification never reaches the dock.
            assert!(!fullscreen_window(window.0, HWND::default(), monitor));
        }
        Ok(())
    }

    #[test]
    fn showing_dock_preserves_topmost_without_taking_focus() -> Result<()> {
        let window = popup()?;
        let foreground = unsafe { GetForegroundWindow() };
        for _ in 0..3 {
            show_topmost(window.0)?;
            assert!(unsafe { IsWindowVisible(window.0).as_bool() });
            assert_ne!(
                unsafe { GetWindowLongPtrW(window.0, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0,
                0
            );
            assert_eq!(unsafe { GetForegroundWindow() }, foreground);
            let _ = unsafe { ShowWindow(window.0, SW_HIDE) };
        }
        Ok(())
    }

    #[test]
    fn auto_hide_stays_at_monitor_edge_when_taskbar_reserves_space() -> Result<()> {
        let window = popup()?;
        let settings = config::Placement {
            auto_hide: true,
            ..Default::default()
        };
        let placement = Placement::new(
            window.0,
            &settings,
            DockSize::new(config::Appearance::default()),
            2,
        )?;
        let screen = monitor_info(placement.monitor())?.monitorInfo.rcMonitor;
        let mut bounds = RECT::default();
        unsafe { GetWindowRect(window.0, &mut bounds)? };
        assert_eq!(bounds.bottom, screen.bottom);
        assert!(placement.reveal_at(POINT {
            x: bounds.left,
            y: screen.bottom - 1
        }));
        Ok(())
    }

    #[test]
    fn restores_top_bar_reservation_when_shell_work_area_changes() -> Result<()> {
        let window = TestWindow(unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!("Styled Dock reservation test"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )?
        });
        let monitor = unsafe { MonitorFromWindow(window.0, MONITOR_DEFAULTTOPRIMARY) };
        let screen = monitor_info(monitor)?.monitorInfo.rcMonitor;
        let mut bar = AppBar::new(window.0, ABE_TOP)?;
        let _ = unsafe { ShowWindow(window.0, SW_SHOWNOACTIVATE) };
        let reserved = bar.reserve(screen, 32)?;
        assert!(monitor_info(monitor)?.monitorInfo.rcWork.top >= reserved.bottom);

        // Emulate a shell work-area recalculation without changing the cached
        // rectangle or the top bar's size, as can happen during hide/show.
        let mut released = bar.data();
        released.rc = RECT {
            bottom: reserved.top,
            ..reserved
        };
        unsafe { SHAppBarMessage(ABM_SETPOS, &mut released) };
        assert!(monitor_info(monitor)?.monitorInfo.rcWork.top < reserved.bottom);

        assert_eq!(bar.reserve(screen, 32)?, reserved);
        assert!(monitor_info(monitor)?.monitorInfo.rcWork.top >= reserved.bottom);
        // Repeated updates must settle on the same band, without growing it.
        assert_eq!(bar.reserve(screen, 32)?, reserved);

        let _ = unsafe { ShowWindow(window.0, SW_HIDE) };
        assert_eq!(bar.reserve(screen, 32)?, reserved);
        let _ = unsafe { ShowWindow(window.0, SW_SHOWNOACTIVATE) };
        assert_eq!(bar.reserve(screen, 32)?, reserved);
        assert!(monitor_info(monitor)?.monitorInfo.rcWork.top >= reserved.bottom);
        Ok(())
    }
}
