//! A compact, animated Downloads stack, anchored to the dock shortcut.
use std::cell::Cell;
use std::path::PathBuf;
use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Result, w};

use crate::config::{Appearance, Pin};
use crate::popup::{self, Font};

struct Entry {
    pin: Pin,
    icon: HICON,
}

impl Drop for Entry {
    fn drop(&mut self) {
        if !self.icon.is_invalid() {
            let _ = unsafe { DestroyIcon(self.icon) };
        }
    }
}

fn recent(path: &std::path::Path, limit: usize) -> std::io::Result<Vec<PathBuf>> {
    let mut entries: Vec<_> = std::fs::read_dir(path)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let metadata = entry.metadata().ok()?;
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            // In-progress browser downloads should not be offered as ready files.
            if !metadata.is_file()
                || [".crdownload", ".part", ".partial", ".tmp"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
            {
                return None;
            }
            Some((
                metadata.created().or_else(|_| metadata.modified()).ok(),
                entry.path(),
            ))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(entries
        .into_iter()
        .take(limit)
        .map(|(_, path)| path)
        .collect())
}

fn entry(pin: Pin) -> Entry {
    let mut info = SHFILEINFOW::default();
    unsafe {
        SHGetFileInfoW(
            &HSTRING::from(&pin.target),
            Default::default(),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
    }
    Entry {
        pin,
        icon: info.hIcon,
    }
}

struct Fan {
    entries: Vec<Entry>,
    font: Font,
    dpi: i32,
    width: i32,
    height: i32,
    row: i32,
    speed: f32,
    started: Cell<Instant>,
    closing: Cell<bool>,
    close_from: Cell<f32>,
    choice: Cell<Option<usize>>,
    hot: Cell<usize>,
    empty: bool,
    pressed: Cell<Option<(usize, POINT)>>,
    interacting: Cell<bool>,
}

impl Fan {
    fn progress(&self) -> f32 {
        let elapsed = self.started.get().elapsed().as_secs_f32();
        if self.speed == 0.0 {
            return if self.closing.get() { 0.0 } else { 1.0 };
        }
        if self.closing.get() {
            (self.close_from.get() - elapsed * self.speed / 0.20).max(0.0)
        } else {
            (elapsed * self.speed / 0.32).min(1.0)
        }
    }

    fn rect(&self, index: usize) -> RECT {
        let p = self.progress();
        let p = (p * 1.18 - index as f32 * 0.025).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - p).powi(3);
        let px = |dip| popup::scale(dip, self.dpi);
        let fraction = index as f32 / (self.entries.len().max(2) - 1) as f32;
        let curve = px(58) as f32 * fraction * fraction;
        let left = (px(12) as f32 + curve * ease) as i32;
        let bottom = self.height - px(8) - (index as f32 * self.row as f32 * ease) as i32;
        RECT {
            left,
            top: bottom - self.row + px(6),
            right: left + self.width - px(84),
            bottom,
        }
    }

    fn update(&self, hwnd: HWND) {
        unsafe {
            let region = CreateRectRgn(0, 0, 0, 0);
            for index in 0..self.entries.len() {
                let r = self.rect(index);
                let rounded = CreateRoundRectRgn(
                    r.left,
                    r.top,
                    r.right + 1,
                    r.bottom + 1,
                    popup::scale(18, self.dpi),
                    popup::scale(18, self.dpi),
                );
                CombineRgn(Some(region), Some(region), Some(rounded), RGN_OR);
                let _ = DeleteObject(rounded.into());
            }
            if SetWindowRgn(hwnd, Some(region), true) == 0 {
                let _ = DeleteObject(region.into());
            }
            let _ = SetLayeredWindowAttributes(
                hwnd,
                COLORREF(0),
                (255.0 * self.progress().min(1.0)).round() as u8,
                LWA_ALPHA,
            );
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    fn close(&self, choice: Option<usize>) {
        if !self.closing.get() {
            self.close_from.set(self.progress());
            self.closing.set(true);
            self.choice.set(choice);
            self.started.set(Instant::now());
        }
    }

    fn paint(&self, hwnd: HWND) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let memory = CreateCompatibleDC(Some(dc));
            let bitmap = CreateCompatibleBitmap(dc, self.width, self.height);
            let old = SelectObject(memory, bitmap.into());
            popup::fill(
                memory,
                &RECT {
                    left: 0,
                    top: 0,
                    right: self.width,
                    bottom: self.height,
                },
                popup::MENU_SURFACE,
            );
            // Bottom row stays in front while rows unfurl from the same origin.
            for index in (0..self.entries.len()).rev() {
                let mut r = self.rect(index);
                popup::rounded(
                    memory,
                    &r,
                    if self.hot.get() == index {
                        popup::SELECTED
                    } else {
                        popup::PANEL
                    },
                    popup::scale(18, self.dpi),
                );
                let size = popup::scale(30, self.dpi);
                let x = r.left + popup::scale(10, self.dpi);
                let y = r.top + (r.bottom - r.top - size) / 2;
                let item = &self.entries[index];
                if !item.icon.is_invalid() {
                    let _ = DrawIconEx(memory, x, y, item.icon, size, size, 0, None, DI_NORMAL);
                }
                r.left = x + size + popup::scale(10, self.dpi);
                r.right -= popup::scale(12, self.dpi);
                let label = if index == 0 && self.empty {
                    "No recent files · Open Downloads"
                } else {
                    &item.pin.name
                };
                popup::draw_text(memory, label, &mut r, self.font.0, popup::INK);
            }
            let _ = BitBlt(
                dc,
                0,
                0,
                self.width,
                self.height,
                Some(memory),
                0,
                0,
                SRCCOPY,
            );
            SelectObject(memory, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            let _ = EndPaint(hwnd, &ps);
        }
    }
}

pub fn show(owner: HWND, anchor: POINT, appearance: Appearance) -> Result<Option<Pin>> {
    let folder = crate::shell::folder_target("shell:Downloads")?;
    let monitor = unsafe { MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST) };
    let work = crate::placement::monitor_info(monitor)?.monitorInfo.rcWork;
    let dpi = unsafe { GetDpiForWindow(owner) } as i32;
    let row = popup::scale(54, dpi);
    let limit = ((anchor.y.min(work.bottom) - work.top - popup::scale(16, dpi)) / row - 1)
        .clamp(0, 7) as usize;
    let paths = recent(std::path::Path::new(&folder), limit)
        .map_err(|error| windows::core::Error::new(E_FAIL, error.to_string()))?;
    let empty = paths.is_empty();
    let mut entries = vec![entry(Pin {
        name: "Open Downloads".into(),
        target: folder,
    })];
    entries.extend(paths.into_iter().map(|path| {
        entry(Pin {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            target: path.to_string_lossy().into_owned(),
        })
    }));
    let width = popup::scale(388, dpi).min(work.right - work.left);
    let height = (entries.len() as i32 * row + popup::scale(12, dpi)).min(work.bottom - work.top);
    let fan = Fan {
        entries,
        font: Font::new(13, dpi, FW_MEDIUM),
        dpi,
        width,
        height,
        row,
        speed: if popup::high_contrast() {
            0.0
        } else {
            appearance.animation_speed_percent as f32 / 100.0
        },
        started: Cell::new(Instant::now()),
        closing: Cell::new(false),
        close_from: Cell::new(1.0),
        choice: Cell::new(None),
        hot: Cell::new(0),
        empty,
        pressed: Cell::new(None),
        interacting: Cell::new(false),
    };
    unsafe {
        let instance = GetModuleHandleW(None)?.into();
        let class = WNDCLASSW {
            hInstance: instance,
            lpszClassName: w!("StyledDockDownloads"),
            lpfnWndProc: Some(window_proc),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&class);
        let previous = GetForegroundWindow();
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED,
            class.lpszClassName,
            w!("Recent Downloads"),
            WS_POPUP,
            (anchor.x - popup::scale(30, dpi)).clamp(work.left, work.right - width),
            (anchor.y - height).clamp(work.top, work.bottom - height),
            width,
            height,
            Some(owner),
            None,
            Some(instance),
            Some((&raw const fan).cast()),
        )?;
        fan.update(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        SetTimer(Some(hwnd), 1, 16, None);
        let mut message = MSG::default();
        let mut deferred = Vec::new();
        let result = loop {
            let status = GetMessageW(&mut message, None, 0, 0).0;
            if status <= 0 {
                if status == 0 {
                    PostQuitMessage(message.wParam.0 as i32);
                }
                break if status < 0 {
                    Err(windows::core::Error::from_thread())
                } else {
                    Ok(None)
                };
            }
            if message.hwnd != hwnd && (WM_APP..=0xbfff).contains(&message.message) {
                // Dock and top-bar events are handled by their outer loop.
                // Dispatching here would consume shell results and tray updates.
                deferred.push(message);
                continue;
            }
            if message.hwnd == owner && matches!(message.message, WM_LBUTTONDOWN | WM_RBUTTONDOWN) {
                fan.close(None);
                SetTimer(Some(hwnd), 1, 16, None);
                deferred.push(message);
            } else if message.hwnd == hwnd && message.message == WM_KEYDOWN {
                match message.wParam.0 as u16 {
                    0x1b => fan.close(None),
                    0x0d => fan.close(Some(fan.hot.get())),
                    0x5d => {
                        let r = fan.rect(fan.hot.get());
                        let mut point = POINT {
                            x: r.left,
                            y: r.top,
                        };
                        let _ = ClientToScreen(hwnd, &mut point);
                        fan.interacting.set(true);
                        if let Err(error) =
                            crate::files::menu(hwnd, &fan.entries[fan.hot.get()].pin.target, point)
                        {
                            crate::files::error(hwnd, &error);
                        }
                        fan.interacting.set(false);
                        fan.close(None);
                    }
                    0x26 => fan.hot.set((fan.hot.get() + 1).min(fan.entries.len() - 1)),
                    0x28 => fan.hot.set(fan.hot.get().saturating_sub(1)),
                    _ => {}
                }
                if fan.closing.get() {
                    SetTimer(Some(hwnd), 1, 16, None);
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
            } else {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if fan.closing.get() && fan.progress() == 0.0 {
                break Ok(fan.choice.get().map(|index| fan.entries[index].pin.clone()));
            }
        };
        let _ = KillTimer(Some(hwnd), 1);
        let restore = GetForegroundWindow() == hwnd;
        let _ = DestroyWindow(hwnd);
        // Native drag/context menus pump messages themselves. Recheck work whose
        // notification may have been dispatched inside that nested shell loop.
        for message in [
            crate::shell::WM_SHELL_RESULT,
            crate::watcher::WM_APPS_CHANGED,
        ] {
            let _ = PostMessageW(Some(owner), message, WPARAM(0), LPARAM(0));
        }
        for message in deferred {
            let _ = PostMessageW(
                Some(message.hwnd),
                message.message,
                message.wParam,
                message.lParam,
            );
        }
        if restore && !previous.is_invalid() {
            let _ = SetForegroundWindow(previous);
        }
        result
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Fan;
    if let Some(fan) = unsafe { pointer.as_ref() } {
        match message {
            WM_PAINT => {
                fan.paint(hwnd);
                return LRESULT(0);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_TIMER => {
                fan.update(hwnd);
                if fan.progress() == 1.0 && !fan.closing.get() {
                    let _ = unsafe { KillTimer(Some(hwnd), 1) };
                }
                return LRESULT(0);
            }
            WM_ACTIVATE if wparam.0 & 0xffff == WA_INACTIVE as usize && !fan.interacting.get() => {
                fan.close(None);
                unsafe { SetTimer(Some(hwnd), 1, 16, None) };
            }
            WM_CLOSE => {
                fan.close(None);
                unsafe { SetTimer(Some(hwnd), 1, 16, None) };
                return LRESULT(0);
            }
            WM_CAPTURECHANGED => {
                fan.pressed.set(None);
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP => {
                let point = POINT {
                    x: lparam.0 as i16 as i32,
                    y: (lparam.0 >> 16) as i16 as i32,
                };
                let hit = (0..fan.entries.len())
                    .find(|&index| unsafe { PtInRect(&fan.rect(index), point).as_bool() });
                if message == WM_MOUSEMOVE
                    && let Some((index, origin)) = fan.pressed.get()
                    && index > 0
                {
                    let threshold_x = unsafe { GetSystemMetrics(SM_CXDRAG) }.max(1);
                    let threshold_y = unsafe { GetSystemMetrics(SM_CYDRAG) }.max(1);
                    if (point.x - origin.x).abs() >= threshold_x
                        || (point.y - origin.y).abs() >= threshold_y
                    {
                        fan.pressed.set(None);
                        fan.interacting.set(true);
                        let _ = unsafe {
                            windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture()
                        };
                        if let Err(error) = crate::files::drag(hwnd, &fan.entries[index].pin.target)
                        {
                            crate::files::error(hwnd, &error);
                        }
                        fan.interacting.set(false);
                        fan.close(None);
                        unsafe { SetTimer(Some(hwnd), 1, 16, None) };
                        return LRESULT(0);
                    }
                }
                let pressed = if message == WM_LBUTTONUP {
                    fan.pressed.take()
                } else {
                    None
                };
                if message == WM_LBUTTONUP {
                    let _ =
                        unsafe { windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture() };
                }
                if let Some(index) = hit {
                    fan.hot.set(index);
                    if message == WM_LBUTTONDOWN {
                        fan.pressed.set(Some((index, point)));
                        unsafe { windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd) };
                    }
                    if message == WM_RBUTTONUP {
                        fan.interacting.set(true);
                        let mut screen = point;
                        let _ = unsafe { ClientToScreen(hwnd, &mut screen) };
                        if let Err(error) =
                            crate::files::menu(hwnd, &fan.entries[index].pin.target, screen)
                        {
                            crate::files::error(hwnd, &error);
                        }
                        fan.interacting.set(false);
                        fan.close(None);
                        unsafe { SetTimer(Some(hwnd), 1, 16, None) };
                    }
                    if message == WM_LBUTTONUP && pressed.is_some_and(|(down, _)| down == index) {
                        fan.close(Some(index));
                        unsafe { SetTimer(Some(hwnd), 1, 16, None) };
                    }
                    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
                }
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "system" fn choose_folder(owner: HWND, _: u32, id: usize, _: u32) {
        unsafe {
            if let Ok(fan) = GetWindow(owner, GW_ENABLEDPOPUP)
                && fan != owner
            {
                let _ = KillTimer(Some(owner), id);
                let _ = InvalidateRect(Some(fan), None, false);
                let _ = PostMessageW(
                    Some(owner),
                    crate::tray::WM_TRAY_CHANGED,
                    WPARAM(71),
                    LPARAM(0),
                );
                let _ = PostMessageW(Some(fan), WM_KEYDOWN, WPARAM(0x0d), LPARAM(0));
            }
        }
    }

    #[test]
    fn fan_selection_returns_after_idle_with_animation_on_and_off() -> Result<()> {
        unsafe {
            let owner = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Downloads test"),
                WS_OVERLAPPED,
                0,
                0,
                100,
                100,
                None,
                None,
                None,
                None,
            )?;
            let thread = windows::Win32::System::Threading::GetCurrentThreadId();
            let (done, wait) = std::sync::mpsc::channel();
            let watchdog = std::thread::spawn(move || {
                if wait
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .is_err()
                {
                    let _ = PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            });
            let result = (|| {
                for speed in [100, 0] {
                    SetTimer(Some(owner), 123, 500, Some(choose_folder));
                    let choice = show(
                        owner,
                        POINT { x: 400, y: 650 },
                        Appearance {
                            animation_speed_percent: speed,
                            ..Default::default()
                        },
                    )?;
                    assert_eq!(
                        choice.unwrap().target,
                        crate::shell::folder_target("shell:Downloads")?
                    );
                    let mut message = MSG::default();
                    assert!(
                        PeekMessageW(
                            &mut message,
                            Some(owner),
                            crate::tray::WM_TRAY_CHANGED,
                            crate::tray::WM_TRAY_CHANGED,
                            PM_REMOVE
                        )
                        .as_bool(),
                        "tray events must survive the fan's nested message loop"
                    );
                    assert_eq!(message.wParam.0, 71);
                }
                Ok(())
            })();
            let _ = done.send(());
            let _ = watchdog.join();
            let _ = KillTimer(Some(owner), 123);
            let _ = DestroyWindow(owner);
            let mut message = MSG::default();
            let _ = PeekMessageW(&mut message, None, WM_QUIT, WM_QUIT, PM_REMOVE);
            result
        }
    }

    #[test]
    fn recent_files_skip_partial_downloads_and_directories() -> std::io::Result<()> {
        let path = std::env::current_dir()?
            .join(".test-tmp")
            .join(format!("downloads-{}", std::process::id()));
        std::fs::create_dir_all(path.join("folder"))?;
        for name in [
            "ready.txt",
            "incomplete.crdownload",
            "incomplete.part",
            "incomplete.partial",
            "temporary.tmp",
        ] {
            std::fs::write(path.join(name), "test")?;
        }
        assert_eq!(recent(&path, 7)?, vec![path.join("ready.txt")]);
        assert!(recent(&path, 0)?.is_empty());
        std::fs::remove_dir_all(path)
    }
}
