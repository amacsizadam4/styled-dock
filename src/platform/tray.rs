//! Mirror Shell notification messages while forwarding them to Explorer.
//! The Shell_TrayWnd wire protocol is undocumented; keep this optional and
//! never take ownership of Explorer's windows or change its tray preferences.
use std::cell::RefCell;
use std::path::PathBuf;

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, Result, w};

use crate::shell::Icon;

mod cache;

pub const WM_TRAY_CHANGED: u32 = WM_APP + 30;
const WM_REANNOUNCE: u32 = WM_APP + 31;

pub struct TrayIcon {
    pub serial: u64,
    pub image: Icon,
    pub title: String,
    window: HWND,
    id: u32,
    callback: u32,
    version: u32,
    guid: [u8; 16],
    hidden: bool,
    identity: Option<cache::ProcessIdentity>,
}

impl TrayIcon {
    pub fn menu_key(&self) -> (isize, u32, [u8; 16]) {
        (self.window.0 as isize, self.id, self.guid)
    }
    pub fn click(&self, message: u32, point: POINT) {
        if self.callback == 0
            || !unsafe { IsWindow(Some(self.window)).as_bool() }
            || self
                .identity
                .as_ref()
                .is_some_and(|identity| !identity.matches(self.window))
        {
            return;
        }
        // Apps expect their own window to be eligible to show a foreground menu.
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(self.window, Some(&mut pid)) };
        let _ = unsafe { AllowSetForegroundWindow(pid) };
        let (wparam, lparam) = if self.version >= 4 {
            let event = match message {
                WM_LBUTTONUP => WM_USER, // NIN_SELECT
                WM_RBUTTONUP => WM_CONTEXTMENU,
                other => other,
            };
            (
                (point.x as u16 as u32 | (point.y as u16 as u32) << 16) as usize,
                (event & 0xffff | (self.id & 0xffff) << 16) as isize,
            )
        } else {
            (self.id as usize, message as isize)
        };
        let _ = unsafe {
            PostMessageW(
                Some(self.window),
                self.callback,
                WPARAM(wparam),
                LPARAM(lparam),
            )
        };
    }
}

#[derive(Default)]
pub struct TrayState {
    items: Vec<TrayIcon>,
    next_serial: u64,
    owner: HWND,
    dirty: bool,
    cache_path: Option<PathBuf>,
    announcing: bool,
    startup_ticks: u32,
    lookup_lost: bool,
}

impl TrayState {
    pub fn icons(&self) -> impl Iterator<Item = &TrayIcon> {
        self.items.iter().filter(|icon| !icon.hidden)
    }

    fn receive(&mut self, bytes: &[u8]) {
        let Some(packet) = Packet::parse(bytes) else {
            return;
        };
        let index = self.items.iter().position(|icon| {
            if packet.guid != [0; 16] {
                icon.guid == packet.guid
            } else {
                icon.window == packet.window && icon.id == packet.id
            }
        });
        if packet.command == 2 {
            // NIM_DELETE
            if let Some(index) = index {
                self.items.remove(index);
            }
        } else if packet.command == 4 {
            // NIM_SETVERSION
            if let Some(index) = index {
                self.items[index].version = packet.version;
            }
        } else if packet.command <= 1 {
            if !unsafe { IsWindow(Some(packet.window)).as_bool() } {
                return;
            }
            // Existing status controls already cover these Windows icons.
            let system = [0x73, 0x74, 0x75].iter().any(|&suffix| {
                packet.guid
                    == [
                        suffix, 0xae, 0x20, 0x78, 0xe3, 0x23, 0x29, 0x42, 0x82, 0xc1, 0xe4, 0x1c,
                        0xb6, 0x7d, 0x5b, 0x9c,
                    ]
            });
            if system {
                return;
            }
            let image = (packet.flags & 2 != 0)
                .then(|| icon_image(packet.icon).ok())
                .flatten();
            self.next_serial += 1;
            let index = if let Some(index) = index {
                index
            } else {
                let Some(image) = image else { return };
                self.items.push(TrayIcon {
                    serial: self.next_serial,
                    image,
                    title: packet.title.clone(),
                    window: packet.window,
                    id: packet.id,
                    callback: packet.callback,
                    version: 0,
                    guid: packet.guid,
                    hidden: false,
                    identity: cache::ProcessIdentity::for_window(packet.window),
                });
                self.apply(self.items.len() - 1, &packet, None);
                self.changed();
                return;
            };
            if packet.command == 0 {
                // A re-registration supersedes cached ownership and callback mode.
                let item = &mut self.items[index];
                item.window = packet.window;
                item.id = packet.id;
                item.identity = cache::ProcessIdentity::for_window(packet.window);
                item.version = 0;
                item.hidden = false;
            }
            self.apply(index, &packet, image);
        }
        self.changed();
    }

    fn apply(&mut self, index: usize, packet: &Packet, image: Option<Icon>) {
        let item = &mut self.items[index];
        if let Some(image) = image {
            item.image = image;
            item.serial = self.next_serial;
        }
        if packet.flags & 1 != 0 {
            item.callback = packet.callback;
        }
        if packet.flags & 4 != 0 {
            item.title.clone_from(&packet.title);
        }
        if packet.flags & 8 != 0 && packet.state_mask & 1 != 0 {
            item.hidden = packet.state & 1 != 0;
        }
        if packet.flags & 0x20 != 0 {
            item.guid = packet.guid;
        }
    }

    fn changed(&mut self) {
        self.dirty = true;
        let _ = unsafe { PostMessageW(Some(self.owner), WM_TRAY_CHANGED, WPARAM(0), LPARAM(0)) };
    }
}

pub struct Tray {
    hwnd: HWND,
    state: Box<RefCell<TrayState>>,
    _com: ComApartment,
}

struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

impl Tray {
    pub fn new(instance: HINSTANCE, owner: HWND) -> Result<Self> {
        Self::with_cache(instance, owner, cache::path())
    }

    pub(crate) fn with_cache(
        instance: HINSTANCE,
        owner: HWND,
        cache_path: Option<PathBuf>,
    ) -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        let com = ComApartment;
        let mut state = Box::new(RefCell::new(cache_path.as_deref().map_or_else(
            || TrayState {
                owner,
                ..Default::default()
            },
            |path| cache::load(path, owner),
        )));
        state.borrow_mut().cache_path = cache_path;
        let hwnd = unsafe {
            let class = WNDCLASSW {
                hInstance: instance,
                lpszClassName: w!("Shell_TrayWnd"),
                lpfnWndProc: Some(window_proc),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(Error::from_thread());
            }
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                class.lpszClassName,
                w!("Styled Dock Tray Bridge"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                Some((&raw mut *state).cast()),
            )?
        };
        let tray = Self {
            hwnd,
            state,
            _com: com,
        };
        unsafe { SetTimer(Some(hwnd), 1, 1000, None) };
        // Establish lookup priority before asking apps to register. Defer the
        // broadcast until the owner has entered its message pump.
        prioritize(hwnd);
        unsafe {
            let _ = ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, None);
            let _ = PostMessageW(Some(hwnd), WM_REANNOUNCE, WPARAM(0), LPARAM(0));
        }
        Ok(tray)
    }

    pub fn state(&self) -> std::cell::Ref<'_, TrayState> {
        self.state.borrow()
    }

    fn reannounce(&self) {
        request_registrations(self.hwnd);
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let state = self.state.borrow();
        if let Some(path) = &state.cache_path {
            let _ = cache::save(path, &state);
        }
        drop(state);
        let _ = unsafe { DestroyWindow(self.hwnd) };
        // Apps register with Explorer again after the bridge has gone away.
        self.reannounce();
    }
}

fn prioritize(hwnd: HWND) {
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
}

fn announce(hwnd: HWND) {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const RefCell<TrayState>;
    let Some(cell) = (unsafe { pointer.as_ref() }) else {
        return;
    };
    let Ok(mut state) = cell.try_borrow_mut() else {
        let _ = unsafe { PostMessageW(Some(hwnd), WM_REANNOUNCE, WPARAM(0), LPARAM(0)) };
        return;
    };
    if state.announcing {
        return;
    }
    state.announcing = true;
    drop(state);
    prioritize(hwnd);
    request_registrations(hwnd);
    if let Ok(mut state) = cell.try_borrow_mut() {
        state.announcing = false;
    }
}

// Ask icon owners to register without telling Explorer or our own dock that
// the shell restarted. The dock responds by removing/re-registering AppBars,
// briefly releasing rcWork even when Explorer has not actually restarted.
fn request_registrations(bridge: HWND) {
    struct Request {
        bridge: HWND,
        shell_pid: u32,
        message: u32,
    }
    unsafe extern "system" fn notify(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
        let request = unsafe { &*(data.0 as *const Request) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == std::process::id() {
            let mut class = [0u16; 64];
            let length = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
            if matches!(
                String::from_utf16_lossy(&class[..length]).as_str(),
                "StyledDockWindow" | "StyledDockTopBar"
            ) {
                return windows::core::BOOL(1);
            }
        }
        if hwnd != request.bridge && pid != request.shell_pid {
            let _ = unsafe { SendNotifyMessageW(hwnd, request.message, WPARAM(0), LPARAM(0)) };
        }
        windows::core::BOOL(1)
    }
    let mut request = Request {
        bridge,
        shell_pid: 0,
        message: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
    };
    if request.message != 0 {
        unsafe {
            GetWindowThreadProcessId(GetShellWindow(), Some(&mut request.shell_pid));
            let _ = EnumWindows(Some(notify), LPARAM((&raw const request) as isize));
        }
    }
}

struct Packet {
    command: u32,
    window: HWND,
    id: u32,
    flags: u32,
    callback: u32,
    icon: HICON,
    title: String,
    state: u32,
    state_mask: u32,
    version: u32,
    guid: [u8; 16],
}

impl Packet {
    fn parse(bytes: &[u8]) -> Option<Self> {
        // Shell32 marshals handles as 32-bit values, including for 64-bit callers.
        let dword = |offset: usize| -> Option<u32> {
            Some(u32::from_le_bytes(
                bytes.get(offset..offset + 4)?.try_into().ok()?,
            ))
        };
        if dword(0)? != 0x34753423 || bytes.len() < 32 {
            return None;
        }
        let size = dword(8)? as usize;
        if size < 24 || size.checked_add(8)? > bytes.len() {
            return None;
        }
        let command = dword(4)?;
        if command > 4 {
            return None;
        }
        let title_end = (size + 8).min(288);
        let title: Vec<_> = bytes[32..title_end]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .collect();
        let mut guid = [0; 16];
        let flags = dword(20)?;
        if size >= 952 && flags & 0x20 != 0 {
            guid.copy_from_slice(bytes.get(944..960)?);
        }
        Some(Self {
            command,
            window: HWND(dword(12)? as usize as *mut _),
            id: dword(16)?,
            flags,
            callback: dword(24)?,
            icon: HICON(dword(28)? as usize as *mut _),
            title: String::from_utf16_lossy(&title),
            state: if size >= 288 { dword(288)? } else { 0 },
            state_mask: if size >= 288 { dword(292)? } else { 0 },
            version: if size >= 804 { dword(808)? } else { 0 },
            guid,
        })
    }
}

fn icon_image(icon: HICON) -> Result<Icon> {
    unsafe {
        let imaging: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bitmap = imaging.CreateBitmapFromHICON(icon)?;
        let converted = imaging.CreateFormatConverter()?;
        converted.Initialize(
            &bitmap,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let (mut width, mut height) = (0, 0);
        converted.GetSize(&mut width, &mut height)?;
        if width == 0 || height == 0 || width > 256 || height > 256 {
            return Err(Error::empty());
        }
        let mut pixels = vec![0; (width * height * 4) as usize];
        converted.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
        Ok(Icon {
            width,
            height,
            pixels,
        })
    }
}

fn explorer() -> Option<HWND> {
    let mut previous = None;
    let mut shell_pid = 0;
    unsafe { GetWindowThreadProcessId(GetShellWindow(), Some(&mut shell_pid)) };
    loop {
        let hwnd = unsafe { FindWindowExW(None, previous, w!("Shell_TrayWnd"), None) }.ok()?;
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid != 0 && pid == shell_pid {
            return Some(hwnd);
        }
        previous = Some(hwnd);
    }
}

fn send_copydata(native: HWND, sender: WPARAM, data: &COPYDATASTRUCT) -> Option<LRESULT> {
    let mut result = 0;
    let sent = unsafe {
        SendMessageTimeoutW(
            native,
            WM_COPYDATA,
            sender,
            LPARAM((data as *const COPYDATASTRUCT) as isize),
            SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
            500,
            Some(&mut result),
        )
    };
    (sent.0 != 0).then_some(LRESULT(result as isize))
}

// SHAppBarMessage targets the first Shell_TrayWnd, just like tray icons do.
// Its query/set replies live in memory allocated for that recipient. Forwarding
// the packet alone reports success but leaves Explorer unable to read/write it.
// Re-marshal the shared APPBARDATA for Explorer and copy its reply back.
fn forward_appbar(native: HWND, sender: WPARAM, data: &COPYDATASTRUCT) -> Option<LRESULT> {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::UI::Shell::{SHAllocShared, SHFreeShared, SHLockShared, SHUnlockShared};

    if data.dwData != 0 || data.cbData != 64 || data.lpData.is_null() {
        return None;
    }
    let mut packet = [0u8; 64];
    unsafe { std::ptr::copy_nonoverlapping(data.lpData.cast::<u8>(), packet.as_mut_ptr(), 64) };
    if u32::from_le_bytes(packet[..4].try_into().unwrap()) != 40 {
        return None;
    }
    let handle = u64::from_le_bytes(packet[48..56].try_into().unwrap());
    if handle == 0 {
        return None; // Registration and notifications have no shared reply.
    }
    let source_pid = u32::from_le_bytes(packet[56..60].try_into().unwrap());
    unsafe {
        let old = SHLockShared(HANDLE(handle as usize as *mut _), source_pid);
        if old.is_null() {
            return Some(LRESULT(0));
        }
        let mut native_pid = 0;
        GetWindowThreadProcessId(native, Some(&mut native_pid));
        let shared = SHAllocShared(Some(old), 40, native_pid);
        if shared.is_invalid() {
            let _ = SHUnlockShared(old);
            return Some(LRESULT(0));
        }
        packet[48..56].copy_from_slice(&(shared.0 as usize as u64).to_le_bytes());
        packet[56..60].copy_from_slice(&native_pid.to_le_bytes());
        let forwarded = COPYDATASTRUCT {
            dwData: 0,
            cbData: packet.len() as u32,
            lpData: packet.as_mut_ptr().cast(),
        };
        // Keep this synchronous: the original caller owns the reply buffer and
        // may free it as soon as WM_COPYDATA returns.
        let result = send_copydata(native, sender, &forwarded);
        let reply = SHLockShared(shared, native_pid);
        if !reply.is_null() {
            if result.is_some() {
                std::ptr::copy_nonoverlapping(reply.cast::<u8>(), old.cast::<u8>(), 40);
            }
            let _ = SHUnlockShared(reply);
        }
        let _ = SHFreeShared(shared, native_pid);
        let _ = SHUnlockShared(old);
        Some(result.unwrap_or(LRESULT(0)))
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
    if message == WM_REANNOUNCE {
        announce(hwnd);
        return LRESULT(0);
    }
    if message >= 0xc000 && message == unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) } {
        let pointer =
            unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const RefCell<TrayState>;
        if let Some(cell) = unsafe { pointer.as_ref() } {
            match cell.try_borrow_mut() {
                Ok(mut state) if !state.announcing => {
                    prioritize(hwnd);
                    state.startup_ticks = 0;
                }
                Err(_) => {
                    let _ =
                        unsafe { PostMessageW(Some(hwnd), WM_REANNOUNCE, WPARAM(0), LPARAM(0)) };
                }
                _ => {}
            }
        }
        return LRESULT(0);
    }
    if message == WM_TIMER {
        let lost_priority = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }.ok() != Some(hwnd);
        let mut reannounce = false;
        // Stay ahead of Explorer for Shell_NotifyIcon's FindWindow lookup.
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
        let pointer =
            unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const RefCell<TrayState>;
        if let Some(cell) = unsafe { pointer.as_ref() }
            && let Ok(mut state) = cell.try_borrow_mut()
        {
            state.startup_ticks = state.startup_ticks.saturating_add(1);
            // Explorer can repeatedly regain lookup priority during ordinary
            // shell activity. Repair ordering each tick, but request fresh icons
            // only once per loss episode: TaskbarCreated makes apps delete/add
            // their icons and can make shell components reset the work area.
            reannounce |=
                matches!(state.startup_ticks, 1 | 3) || (lost_priority && !state.lookup_lost);
            state.lookup_lost = lost_priority;
            let before = state.items.len();
            state.items.retain(|icon| {
                icon.identity.as_ref().map_or_else(
                    || unsafe { IsWindow(Some(icon.window)).as_bool() },
                    |identity| identity.matches(icon.window),
                )
            });
            if before != state.items.len() {
                state.changed();
            }
            // Batch updates on the timer instead of writing during app callbacks.
            // Drop also flushes so a restart before this tick cannot lose icons.
            if state.dirty {
                if let Some(path) = &state.cache_path
                    && cache::save(path, &state).is_ok()
                {
                    state.dirty = false;
                }
            }
        }
        if reannounce {
            announce(hwnd);
        }
        return LRESULT(0);
    }
    if message == WM_COPYDATA && lparam.0 != 0 {
        let data = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
        let mut handled = false;
        if data.dwData == 1 && !data.lpData.is_null() && (32..=4096).contains(&data.cbData) {
            let bytes = unsafe {
                std::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize)
            };
            handled = Packet::parse(bytes).is_some();
            let pointer =
                unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const RefCell<TrayState>;
            if let Some(cell) = unsafe { pointer.as_ref() }
                && let Ok(mut state) = cell.try_borrow_mut()
            {
                state.receive(bytes);
            }
        }
        // Explorer remains the owner of balloons and its native notification area.
        if let Some(native) = explorer() {
            if let Some(result) = forward_appbar(native, wparam, data) {
                return result;
            }
            let result = send_copydata(native, wparam, data).unwrap_or(LRESULT(0));
            // The mirror handled notification messages even when Explorer does
            // not accept an entry (for example while its tray is restarting).
            return LRESULT(if handled { 1 } else { result.0 });
        }
        return LRESULT(isize::from(handled));
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::*;

    #[test]
    fn stalled_recipient_does_not_block_tray_forwarding() {
        let (window_tx, window_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let recipient = std::thread::spawn(move || unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Stalled tray recipient test"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            window_tx.send(hwnd.0 as usize).unwrap();
            // No message pump until the caller has returned from its deadline.
            let _ = release_rx.recv_timeout(std::time::Duration::from_secs(5));
            let _ = DestroyWindow(hwnd);
        });
        let hwnd = HWND(window_rx.recv().unwrap() as *mut _);
        let packet = COPYDATASTRUCT::default();
        let started = std::time::Instant::now();
        let result = send_copydata(hwnd, WPARAM(0), &packet);
        let elapsed = started.elapsed();
        let _ = release_tx.send(());
        recipient.join().unwrap();
        assert!(result.is_none());
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "forwarding stalled for {elapsed:?}"
        );
    }

    #[test]
    fn registration_and_timer_messages_tolerate_reentrant_state_borrows() -> Result<()> {
        let instance = unsafe { GetModuleHandleW(None)? }.into();
        let tray = Tray::with_cache(instance, HWND::default(), None)?;
        let held = tray.state();
        // Native SendMessage can reenter our window procedure while the UI owns
        // a read view. It must defer mutations rather than unwind across Win32.
        unsafe {
            SendMessageW(tray.hwnd, WM_REANNOUNCE, None, None);
            SendMessageW(
                tray.hwnd,
                RegisterWindowMessageW(w!("TaskbarCreated")),
                None,
                None,
            );
            SendMessageW(tray.hwnd, WM_TIMER, Some(WPARAM(1)), None);
        }
        drop(held);
        Ok(())
    }

    thread_local! {
        static REGISTRATIONS: Cell<u32> = const { Cell::new(0) };
    }

    unsafe extern "system" fn registering_app(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message >= 0xc000 && message == unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) } {
            REGISTRATIONS.set(REGISTRATIONS.get() + 1);
            let data = test_icon(hwnd);
            let _ = unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn test_icon(hwnd: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 701,
            uFlags: NIF_ICON | NIF_MESSAGE,
            uCallbackMessage: WM_APP + 101,
            hIcon: unsafe { LoadIconW(None, IDI_APPLICATION).unwrap() },
            ..Default::default()
        }
    }

    #[test]
    fn registration_request_does_not_signal_a_shell_restart_to_our_dock() -> Result<()> {
        unsafe extern "system" fn counter(
            hwnd: HWND,
            message: u32,
            w: WPARAM,
            l: LPARAM,
        ) -> LRESULT {
            if message >= 0xc000
                && message == unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }
            {
                unsafe {
                    SetWindowLongPtrW(
                        hwnd,
                        GWLP_USERDATA,
                        GetWindowLongPtrW(hwnd, GWLP_USERDATA) + 1,
                    )
                };
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, message, w, l) }
        }
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            let mut windows = Vec::new();
            for name in [
                w!("StyledDockWindow"),
                w!("StyledDockRegistrationRecipientTest"),
            ] {
                let class = WNDCLASSW {
                    hInstance: instance,
                    lpszClassName: name,
                    lpfnWndProc: Some(counter),
                    ..Default::default()
                };
                assert_ne!(RegisterClassW(&class), 0);
                windows.push(CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    name,
                    w!("Registration recipient test"),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    None,
                    None,
                    Some(instance),
                    None,
                )?);
            }
            request_registrations(HWND::default());
            let dock_messages = GetWindowLongPtrW(windows[0], GWLP_USERDATA);
            let client_messages = GetWindowLongPtrW(windows[1], GWLP_USERDATA);
            for hwnd in windows {
                let _ = DestroyWindow(hwnd);
            }
            let _ = UnregisterClassW(w!("StyledDockWindow"), Some(instance));
            let _ = UnregisterClassW(w!("StyledDockRegistrationRecipientTest"), Some(instance));
            assert_eq!(
                dock_messages, 0,
                "our icon-discovery request must not make the dock release its AppBar"
            );
            assert_eq!(
                client_messages, 1,
                "icon owners must still receive the discovery request"
            );
        }
        Ok(())
    }

    #[test]
    fn appbar_reserves_work_area_with_tray_bridge_active() -> Result<()> {
        use crate::placement::{AppBar, monitor_info};
        use windows::Win32::Foundation::RECT;
        use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
        use windows::Win32::Graphics::Gdi::*;
        struct Window(HWND);
        unsafe extern "system" fn app_proc(
            hwnd: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        impl Drop for Window {
            fn drop(&mut self) {
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            let class = WNDCLASSW {
                hInstance: instance,
                lpszClassName: w!("StyledDockMaximizedWorkAreaTest"),
                lpfnWndProc: Some(app_proc),
                ..Default::default()
            };
            RegisterClassW(&class);
            let window = Window(CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!("AppBar with tray bridge test"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance),
                None,
            )?);
            {
                let tray = Tray::with_cache(instance, window.0, None)?;
                assert_eq!(FindWindowW(w!("Shell_TrayWnd"), None)?, tray.hwnd);
                let monitor = MonitorFromWindow(window.0, MONITOR_DEFAULTTOPRIMARY);
                let screen = monitor_info(monitor)?.monitorInfo.rcMonitor;
                let mut bar = AppBar::new(window.0, ABE_TOP)?;
                let _ = ShowWindow(window.0, SW_SHOWNOACTIVATE);
                let reserved = bar.reserve(screen, 32)?;
                assert!(monitor_info(monitor)?.monitorInfo.rcWork.top >= reserved.bottom);
                SetWindowPos(
                    window.0,
                    None,
                    reserved.left,
                    reserved.top,
                    reserved.right - reserved.left,
                    reserved.bottom - reserved.top,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )?;

                for _ in 0..3 {
                    let _ = ShowWindow(window.0, SW_HIDE);
                    let _ = ShowWindow(window.0, SW_SHOWNOACTIVATE);
                    assert_eq!(bar.reserve(screen, 32)?, reserved);
                    bar.reregister()?;
                    assert_eq!(bar.reserve(screen, 32)?, reserved);
                    assert!(monitor_info(monitor)?.monitorInfo.rcWork.top >= reserved.bottom);
                }

                // Check the user-visible result, not only ABM_SETPOS's return:
                // a newly maximized application must start below the top bar.
                let app = Window(CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    class.lpszClassName,
                    w!("Maximized work area test"),
                    WS_OVERLAPPEDWINDOW,
                    screen.left + 100,
                    screen.top + 100,
                    320,
                    240,
                    None,
                    None,
                    Some(instance),
                    None,
                )?);
                let _ = ShowWindow(app.0, SW_SHOWMAXIMIZED);
                windows::Win32::Graphics::Dwm::DwmFlush()?;
                assert!(IsZoomed(app.0).as_bool());
                let mut frame = RECT::default();
                DwmGetWindowAttribute(
                    app.0,
                    DWMWA_EXTENDED_FRAME_BOUNDS,
                    (&raw mut frame).cast(),
                    size_of::<RECT>() as u32,
                )?;
                assert!(
                    frame.top >= reserved.bottom,
                    "maximized frame {frame:?} overlaps reserved bar {reserved:?}"
                );
                drop(app);
                drop(bar);
                drop(tray);
            }
            Ok(())
        }
    }

    #[test]
    fn existing_apps_register_after_startup_and_lookup_priority_loss() -> Result<()> {
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            let class = WNDCLASSW {
                hInstance: instance,
                lpszClassName: w!("StyledDockRegisteringAppTest"),
                lpfnWndProc: Some(registering_app),
                ..Default::default()
            };
            RegisterClassW(&class);
            let app = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class.lpszClassName,
                w!("Tray startup test"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )?;
            let result = (|| {
                // This app exists before the bridge, with no saved snapshot.
                let _ = Shell_NotifyIconW(NIM_ADD, &test_icon(app));
                let tray = Tray::with_cache(instance, app, None)?;
                SendMessageW(tray.hwnd, WM_REANNOUNCE, None, None);
                assert!(tray.state().icons().any(|icon| icon.window == app));
                let competitor = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                    w!("Shell_TrayWnd"),
                    w!("Lookup priority test"),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(instance),
                    None,
                )?;
                prioritize(competitor);
                assert_eq!(FindWindowW(w!("Shell_TrayWnd"), None)?, competitor);
                tray.state.borrow_mut().items.clear();
                SendMessageW(tray.hwnd, WM_TIMER, Some(WPARAM(1)), None);
                let recovered = tray.state().icons().any(|icon| icon.window == app);
                assert!(
                    recovered,
                    "priority recovery must also request missed registrations"
                );
                tray.state.borrow_mut().startup_ticks = 4;
                let registrations = REGISTRATIONS.get();
                for _ in 0..5 {
                    prioritize(competitor);
                    SendMessageW(tray.hwnd, WM_TIMER, Some(WPARAM(1)), None);
                }
                assert_eq!(
                    REGISTRATIONS.get(),
                    registrations,
                    "persistent priority loss must not repeatedly rebuild app icons"
                );
                let _ = DestroyWindow(competitor);
                let _ = Shell_NotifyIconW(NIM_DELETE, &test_icon(app));
                drop(tray);
                Ok(())
            })();
            let _ = Shell_NotifyIconW(NIM_DELETE, &test_icon(app));
            let _ = DestroyWindow(app);
            result
        }
    }

    #[test]
    fn rejects_truncated_or_unrecognized_tray_packets() {
        for length in 0..32 {
            assert!(Packet::parse(&vec![0; length]).is_none());
        }
        let mut packet = vec![0; 964];
        packet[0..4].copy_from_slice(&0x34753423u32.to_le_bytes());
        packet[8..12].copy_from_slice(&956u32.to_le_bytes());
        assert!(Packet::parse(&packet).is_some());
        assert!(Packet::parse(&packet[..963]).is_none());
        packet[4..8].copy_from_slice(&5u32.to_le_bytes());
        assert!(Packet::parse(&packet).is_none());
    }

    #[test]
    fn native_shell_icon_add_update_click_and_delete() -> Result<()> {
        unsafe {
            let cache_path = std::env::current_dir()
                .unwrap()
                .join(".test-tmp")
                .join(format!("tray-restart-{}.bin", std::process::id()));
            let _ = std::fs::remove_file(&cache_path);
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            let owner = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Styled Dock tray test"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance),
                None,
            )?;
            let result = (|| {
                let mut tray = Tray::with_cache(instance, owner, Some(cache_path.clone()))?;
                let mut data = NOTIFYICONDATAW {
                    cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: owner,
                    uID: 42,
                    uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                    uCallbackMessage: WM_APP + 100,
                    hIcon: LoadIconW(None, IDI_APPLICATION)?,
                    ..Default::default()
                };
                let title: Vec<_> = "Styled Dock test icon".encode_utf16().collect();
                data.szTip[..title.len()].copy_from_slice(&title);
                assert!(Shell_NotifyIconW(NIM_ADD, &data).as_bool());
                let check: Result<()> = {
                    let state = tray.state();
                    let icon = state
                        .icons()
                        .find(|icon| icon.window == owner && icon.id == 42)
                        .expect("Shell_NotifyIcon must reach the top-bar bridge");
                    assert_eq!(icon.title, "Styled Dock test icon");
                    assert!(
                        icon.image
                            .pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .any(|pixel| pixel[3] > 0)
                    );
                    drop(state);
                    data.uFlags = NIF_TIP | NIF_STATE;
                    data.dwState = NIS_HIDDEN;
                    data.dwStateMask = NIS_HIDDEN;
                    data.szTip = [0; 128];
                    data.szTip[..7].copy_from_slice(&"Updated".encode_utf16().collect::<Vec<_>>());
                    assert!(Shell_NotifyIconW(NIM_MODIFY, &data).as_bool());
                    assert!(
                        tray.state()
                            .icons()
                            .all(|icon| icon.window != owner || icon.id != 42)
                    );
                    assert_eq!(
                        tray.state()
                            .items
                            .iter()
                            .find(|icon| icon.window == owner)
                            .unwrap()
                            .title,
                        "Updated"
                    );
                    data.uFlags = NIF_STATE;
                    data.dwState = NOTIFY_ICON_STATE(0);
                    assert!(Shell_NotifyIconW(NIM_MODIFY, &data).as_bool());
                    data.Anonymous.uVersion = 4;
                    assert!(Shell_NotifyIconW(NIM_SETVERSION, &data).as_bool());

                    let pixels = tray
                        .state()
                        .icons()
                        .find(|icon| icon.id == 42)
                        .unwrap()
                        .image
                        .pixels
                        .clone();
                    // This owner deliberately ignores TaskbarCreated and sends
                    // no add/modify messages after the bridge restarts.
                    drop(tray);
                    tray = Tray::with_cache(instance, owner, Some(cache_path.clone()))?;
                    let state = tray.state();
                    let icon = state
                        .icons()
                        .find(|icon| icon.window == owner && icon.id == 42)
                        .unwrap();
                    assert_eq!(icon.version, 4);
                    assert_eq!(icon.title, "Updated");
                    assert_eq!(icon.image.pixels, pixels);
                    icon.click(WM_RBUTTONUP, POINT { x: 120, y: 32 });
                    drop(state);
                    let mut message = MSG::default();
                    assert!(
                        PeekMessageW(
                            &mut message,
                            Some(owner),
                            WM_APP + 100,
                            WM_APP + 100,
                            PM_REMOVE
                        )
                        .as_bool()
                    );
                    assert_eq!(message.lParam.0 as u32, 42 << 16 | WM_CONTEXTMENU);
                    assert_eq!(message.wParam.0 as u32, 32 << 16 | 120);
                    Ok(())
                };
                assert!(Shell_NotifyIconW(NIM_DELETE, &data).as_bool());
                assert!(
                    tray.state()
                        .icons()
                        .all(|icon| icon.window != owner || icon.id != 42)
                );
                drop(tray);
                let tray = Tray::with_cache(instance, owner, Some(cache_path.clone()))?;
                assert!(
                    tray.state()
                        .icons()
                        .all(|icon| icon.window != owner || icon.id != 42)
                );
                check
            })();
            let _ = DestroyWindow(owner);
            let _ = std::fs::remove_file(cache_path);
            result
        }
    }
}
