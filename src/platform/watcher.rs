use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Error, PWSTR, Result};

use crate::placement::WM_FULLSCREEN_CHECK;

pub const WM_APPS_CHANGED: u32 = WM_APP + 3;

thread_local! {
    static ATTENTION: RefCell<HashSet<isize>> = RefCell::new(HashSet::new());
    static SHELL_MESSAGE: Cell<u32> = const { Cell::new(0) };
    static NOTIFICATION: Cell<(HWND, bool)> = const { Cell::new((HWND(std::ptr::null_mut()), false)) };
    static KNOWN_WINDOWS: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
}

pub struct Watcher(Vec<HWINEVENTHOOK>, HWND);

impl Watcher {
    pub fn new(dock: HWND, fullscreen: bool) -> Result<Self> {
        let mut watcher = Self(Vec::new(), dock);
        SHELL_MESSAGE.set(unsafe { RegisterWindowMessageW(windows::core::w!("SHELLHOOK")) });
        unsafe {
            RegisterShellHookWindow(dock).ok()?;
        }
        NOTIFICATION.set((dock, false));
        for (first, last) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
            (EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE),
            (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        ]
        .into_iter()
        .chain(fullscreen.then_some((EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE)))
        {
            let hook = unsafe {
                SetWinEventHook(
                    first,
                    last,
                    None,
                    Some(on_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            if hook.0.is_null() {
                return Err(Error::from_thread());
            }
            watcher.0.push(hook);
        }
        request_refresh();
        Ok(watcher)
    }

    pub fn refresh(&self) {
        NOTIFICATION.with(|state| state.set((state.get().0, false)));
        request_refresh();
    }

    pub fn snapshot(&self) -> Vec<App> {
        NOTIFICATION.with(|state| state.set((state.get().0, false)));
        let mut scan = Scan::default();
        let _ = unsafe { EnumWindows(Some(collect_window), LPARAM((&raw mut scan) as isize)) };
        KNOWN_WINDOWS.with_borrow_mut(|known| {
            known.clear();
            known.extend(scan.apps.iter().flat_map(|app| app.windows.iter().copied()));
        });
        scan.apps
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = unsafe { DeregisterShellHookWindow(self.1) };
        ATTENTION.with_borrow_mut(HashSet::clear);
        NOTIFICATION.set((HWND::default(), false));
        for hook in self.0.drain(..) {
            let _ = unsafe { UnhookWinEvent(hook) };
        }
    }
}

pub fn attention(window: HWND) -> bool {
    ATTENTION.with_borrow(|windows| windows.contains(&(window.0 as isize)))
}

pub fn shell_message(message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    if SHELL_MESSAGE.get() == 0 || message != SHELL_MESSAGE.get() {
        return false;
    }
    match wparam.0 {
        0x8006 => {
            ATTENTION.with_borrow_mut(|windows| {
                windows.insert(lparam.0);
            });
        }
        4 | 0x8004 | 2 => {
            ATTENTION.with_borrow_mut(|windows| {
                windows.remove(&lparam.0);
            });
        }
        _ => {}
    }
    request_refresh();
    true
}

fn request_refresh() {
    NOTIFICATION.with(|state| {
        let (dock, pending) = state.get();
        if !dock.0.is_null()
            && !pending
            && unsafe { PostMessageW(Some(dock), WM_APPS_CHANGED, WPARAM(0), LPARAM(0)).is_ok() }
        {
            state.set((dock, true));
        }
    });
}

unsafe extern "system" fn on_event(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if hwnd.0.is_null() || object != OBJID_WINDOW.0 || child != 0 {
        return;
    }
    if event == EVENT_OBJECT_LOCATIONCHANGE {
        if unsafe { GetForegroundWindow() } == hwnd {
            let (dock, _) = NOTIFICATION.get();
            let _ = unsafe { PostMessageW(Some(dock), WM_FULLSCREEN_CHECK, WPARAM(0), LPARAM(0)) };
        }
        return;
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    if root == hwnd
        || event == EVENT_SYSTEM_FOREGROUND
        || (event == EVENT_OBJECT_DESTROY
            && KNOWN_WINDOWS.with_borrow(|known| known.contains(&hwnd)))
    {
        request_refresh();
    }
}

#[derive(Clone)]
struct Identity {
    path: String,
    app_id: Option<String>,
}

#[derive(Default)]
struct Scan {
    identities: HashMap<u32, Option<Identity>>,
    apps: Vec<App>,
}

pub struct App {
    pub key: String,
    pub target: String,
    pub path: String,
    pub name: String,
    pub windows: Vec<HWND>,
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> BOOL {
    let scan = unsafe { &mut *(data.0 as *mut Scan) };
    if let Some((pid, title)) = window_info(hwnd)
        && let Some(mut identity) = scan
            .identities
            .entry(pid)
            .or_insert_with(|| identity(pid))
            .clone()
    {
        if executable(&identity.path).eq_ignore_ascii_case("ApplicationFrameHost.exe") {
            let mut packaged: Option<Identity> = None;
            let _ = unsafe {
                EnumChildWindows(
                    Some(hwnd),
                    Some(find_packaged),
                    LPARAM((&raw mut packaged) as isize),
                )
            };
            let Some(packaged) = packaged else {
                return BOOL(1);
            };
            identity = packaged;
        }
        let explicit_id = crate::platform::app_identity::window_id(hwnd);
        if let Some(id) = &explicit_id {
            identity.app_id = Some(id.clone());
        }
        let identity_target = identity.app_id.as_ref().map_or_else(
            || identity.path.clone(),
            |id| format!("shell:AppsFolder\\{id}"),
        );
        let relaunch = explicit_id
            .as_ref()
            .and_then(|id| crate::platform::app_identity::relaunch_link(hwnd, id, &title));
        let target = relaunch
            .as_ref()
            .map_or_else(|| identity_target.clone(), |(target, _)| target.clone());
        let key = key(&identity_target);
        if let Some(app) = scan.apps.iter_mut().find(|app| app.key == key) {
            app.windows.push(hwnd);
        } else {
            let name = if let Some((_, name)) = relaunch {
                name
            } else if identity.app_id.is_some() {
                title
            } else {
                Path::new(&identity.path)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&title)
                    .to_owned()
            };
            scan.apps.push(App {
                key,
                target,
                path: identity.path,
                name,
                windows: vec![hwnd],
            });
        }
    }
    BOOL(1)
}

fn window_info(hwnd: HWND) -> Option<(u32, String)> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || hwnd == GetShellWindow() {
            return None;
        }
        let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32);
        if !ex.contains(WS_EX_APPWINDOW)
            && ((ex & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)).0 != 0
                || GetWindow(hwnd, GW_OWNER).is_ok())
        {
            return None;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&raw mut cloaked).cast(), 4);
        if cloaked != 0 {
            return None;
        }
        let mut title = [0u16; 256];
        let length = GetWindowTextW(hwnd, &mut title);
        if length <= 0 {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid != 0 && pid != std::process::id())
            .then(|| (pid, String::from_utf16_lossy(&title[..length as usize])))
    }
}

unsafe extern "system" fn find_packaged(hwnd: HWND, data: LPARAM) -> BOOL {
    let mut class = [0; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut class) };
    if length > 0
        && String::from_utf16_lossy(&class[..length as usize]) == "Windows.UI.Core.CoreWindow"
    {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if let Some(app) = identity(pid).filter(|app| app.app_id.is_some()) {
            unsafe { *(data.0 as *mut Option<Identity>) = Some(app) };
            return BOOL(0);
        }
    }
    BOOL(1)
}

fn identity(pid: u32) -> Option<Identity> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut path = [0; 4096];
        let mut size = path.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut size,
        );
        let mut length = 0;
        let mut app_id = None;
        if GetApplicationUserModelId(process, &mut length, None) == ERROR_INSUFFICIENT_BUFFER
            && length > 1
        {
            let mut buffer = vec![0; length as usize];
            if GetApplicationUserModelId(process, &mut length, Some(PWSTR(buffer.as_mut_ptr()))).0
                == 0
            {
                app_id = Some(String::from_utf16_lossy(&buffer[..length as usize - 1]));
            }
        }
        let _ = CloseHandle(process);
        result.ok()?;
        Some(Identity {
            path: String::from_utf16_lossy(&path[..size as usize]),
            app_id,
        })
    }
}

pub fn key(target: &str) -> String {
    target.trim_start_matches(r"\\?\").to_lowercase()
}

fn executable(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

pub fn matches(target: &str, app: &App) -> bool {
    let target = key(target);
    target == app.key || ((app.key == key(&app.path) || key(&app.target) == key(&app.path)) && target == key(&app.path))
        // The default Windows Notepad launcher can open the packaged Notepad.
        || (app.key.starts_with("shell:appsfolder\\microsoft.windowsnotepad_")
            && executable(&target) == "notepad.exe"
            && std::env::var("WINDIR").is_ok_and(|dir| {
                let dir = key(&dir);
                target == format!("{dir}\\system32\\notepad.exe") || target == format!("{dir}\\notepad.exe")
            }))
}

pub enum Activation {
    Minimized,
    Focused,
    Missing,
    Unchanged,
}

pub fn activate(windows: &[HWND]) -> Activation {
    unsafe {
        let foreground = GetForegroundWindow();
        let owner = GetAncestor(foreground, GA_ROOTOWNER);
        if let Some(&active) = windows
            .iter()
            .find(|&&window| window == foreground || window == owner)
        {
            return if ShowWindowAsync(active, SW_MINIMIZE).as_bool() {
                Activation::Minimized
            } else {
                Activation::Unchanged
            };
        }
        let Some(window) = windows
            .iter()
            .copied()
            .find(|&window| IsWindow(Some(window)).as_bool())
        else {
            return Activation::Missing;
        };
        focus(window);
        Activation::Focused
    }
}

pub fn title(window: HWND) -> String {
    let mut title = [0u16; 512];
    let length = unsafe { GetWindowTextW(window, &mut title) };
    if length > 0 {
        String::from_utf16_lossy(&title[..length as usize])
    } else {
        "Untitled window".into()
    }
}

pub fn focus(window: HWND) {
    unsafe {
        if !IsWindow(Some(window)).as_bool() {
            return;
        }
        if IsIconic(window).as_bool() {
            let _ = ShowWindowAsync(window, SW_RESTORE);
        }
        let popup = GetLastActivePopup(window);
        let target = if IsWindowVisible(popup).as_bool() {
            popup
        } else {
            window
        };
        if !SetForegroundWindow(target).as_bool() {
            let _ = FlashWindowEx(&FLASHWINFO {
                cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                hwnd: window,
                dwFlags: FLASHW_TRAY,
                uCount: 2,
                dwTimeout: 0,
            });
        }
    }
}

/// HWND order remains stable when activation changes the enumeration's Z order.
pub fn cycle_window(windows: &[HWND], active: HWND, reverse: bool) -> Option<HWND> {
    let mut windows: Vec<_> = windows
        .iter()
        .copied()
        .filter(|&window| unsafe { IsWindow(Some(window)).as_bool() })
        .collect();
    windows.sort_by_key(|window| window.0 as usize);
    windows.dedup();
    cycle_index(
        windows.len(),
        windows.iter().position(|&window| window == active),
        reverse,
    )
    .map(|index| windows[index])
}

fn cycle_index(count: usize, active: Option<usize>, reverse: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match active {
        Some(index) if reverse => (index + count - 1) % count,
        Some(index) => (index + 1) % count,
        None if reverse => count - 1,
        None => 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cycling_wraps_in_both_directions_and_handles_missing_windows() {
        assert_eq!(cycle_index(3, Some(2), false), Some(0));
        assert_eq!(cycle_index(3, Some(0), true), Some(2));
        assert_eq!(cycle_index(3, Some(1), false), Some(2));
        assert_eq!(cycle_index(1, Some(0), true), Some(0));
        assert_eq!(cycle_index(0, None, false), None);
        assert_eq!(cycle_index(3, None, true), Some(2));
    }
    #[test]
    fn hosted_apps_do_not_match_their_browser_executable() {
        let app = App {
            key: key(r"shell:AppsFolder\MSEdge._crx_whatsapp.Profile2"),
            target: "whatsapp.lnk".into(),
            path: r"C:\Edge\msedge.exe".into(),
            name: "WhatsApp".into(),
            windows: vec![],
        };
        assert!(!matches(r"C:\Edge\msedge.exe", &app));
        assert!(matches(
            r"shell:AppsFolder\MSEdge._crx_whatsapp.Profile2",
            &app
        ));
        assert!(!matches(
            r"shell:AppsFolder\MSEdge._crx_whatsapp.Default",
            &app
        ));
    }
}
