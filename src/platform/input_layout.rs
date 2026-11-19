use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, WPARAM};
use windows::Win32::Globalization::{
    GetLocaleInfoW, LOCALE_SISO639LANGNAME, LOCALE_SLOCALIZEDDISPLAYNAME,
};
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, GetKeyboardLayoutList, HKL};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, HSTRING, PCWSTR, PWSTR, Result, w};

const LAYOUTS: PCWSTR = w!("SYSTEM\\CurrentControlSet\\Control\\Keyboard Layouts");

#[derive(Clone, Copy, Default)]
pub struct Target {
    window: HWND,
    thread: u32,
}

impl Target {
    pub fn foreground() -> Option<Self> {
        unsafe {
            let foreground = GetForegroundWindow();
            let mut pid = 0;
            let thread = GetWindowThreadProcessId(foreground, Some(&mut pid));
            if thread == 0 || pid == std::process::id() {
                return None;
            }
            let mut info = GUITHREADINFO {
                cbSize: size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            if GetGUIThreadInfo(thread, &mut info).is_ok() && !info.hwndFocus.is_invalid() {
                let focus_thread = GetWindowThreadProcessId(info.hwndFocus, None);
                if focus_thread != 0 {
                    return Some(Self {
                        window: info.hwndFocus,
                        thread: focus_thread,
                    });
                }
            }
            Some(Self {
                window: foreground,
                thread,
            })
        }
    }

    pub fn active(self) -> HKL {
        // Thread zero reads the local UI thread when there is no external target yet.
        unsafe { GetKeyboardLayout(self.thread) }
    }

    pub fn select(self, layout: HKL) -> Result<()> {
        unsafe {
            if self.thread == 0 || GetWindowThreadProcessId(self.window, None) != self.thread {
                return Err(Error::new(
                    E_FAIL,
                    crate::i18n::text("Focus an app to change its keyboard layout."),
                ));
            }
            PostMessageW(
                Some(self.window),
                WM_INPUTLANGCHANGEREQUEST,
                WPARAM(0),
                LPARAM(layout.0 as isize),
            )
            .map_err(|error| {
                Error::new(
                    error.code(),
                    crate::i18n::text(
                        "Windows could not request this layout. Use Win+Space in the app.",
                    ),
                )
            })
        }
    }
}

pub fn available(active: HKL) -> Vec<HKL> {
    let count = unsafe { GetKeyboardLayoutList(None) }.max(0);
    let mut layouts = vec![HKL::default(); count as usize];
    let written = unsafe { GetKeyboardLayoutList(Some(&mut layouts)) }.max(0);
    layouts.truncate(written as usize);
    layouts.retain(|layout| !layout.is_invalid());
    if !active.is_invalid() && !layouts.contains(&active) {
        layouts.insert(0, active);
    }
    layouts
}

pub fn language_code(layout: HKL) -> String {
    locale_info(layout, LOCALE_SISO639LANGNAME)
        .map(|code| code.to_ascii_uppercase())
        .unwrap_or_else(|| "—".into())
}

pub fn label(layout: HKL, name: Option<&str>) -> String {
    if layout.is_invalid() {
        return crate::i18n::text("Keyboard layout").into();
    }
    let language = locale_info(layout, LOCALE_SLOCALIZEDDISPLAYNAME)
        .unwrap_or_else(|| crate::i18n::text("Keyboard layout").into());
    match name {
        Some(name) if name != language => format!("{language} · {name}"),
        Some(_) => language,
        None => format!("{language} · {:08X}", layout.0 as u32),
    }
}

pub fn layout_name(layout: HKL) -> Option<String> {
    if layout.is_invalid() {
        return None;
    }
    let device = (layout.0 as usize >> 16) as u16;
    let mut root = HKEY::default();
    unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, LAYOUTS, None, KEY_READ, &mut root) }
        .ok()
        .ok()?;
    let root = LayoutKey(root);
    let key = match device & 0xf000 {
        // Extended layouts (such as Dvorak and Turkish F) use a Layout Id.
        0xf000 => variant_key(&root, device & 0x0fff)?,
        // Legacy IMEs store their full identifier in the input locale handle.
        0xe000 => format!("{:08X}", layout.0 as u32),
        _ => format!("{device:08X}"),
    };
    let key = HSTRING::from(key);
    if let Some(indirect) = registry_string(&root, &key, w!("Layout Display Name")) {
        let mut name = [0u16; 256];
        if unsafe {
            windows::Win32::UI::Shell::SHLoadIndirectString(
                &HSTRING::from(indirect),
                &mut name,
                None,
            )
        }
        .is_ok()
        {
            return Some(wide_string(&name));
        }
    }
    registry_string(&root, &key, w!("Layout Text"))
}

fn locale_info(layout: HKL, kind: u32) -> Option<String> {
    let mut text = [0u16; 128];
    let len = unsafe { GetLocaleInfoW(layout.0 as u16 as u32, kind, Some(&mut text)) };
    (len > 1).then(|| String::from_utf16_lossy(&text[..len as usize - 1]))
}

struct LayoutKey(HKEY);

impl Drop for LayoutKey {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn variant_key(root: &LayoutKey, id: u16) -> Option<String> {
    for index in 0.. {
        let mut key = [0u16; 16];
        let mut length = key.len() as u32;
        unsafe {
            RegEnumKeyExW(
                root.0,
                index,
                Some(PWSTR(key.as_mut_ptr())),
                &mut length,
                None,
                None,
                None,
                None,
            )
        }
        .ok()
        .ok()?;
        let name = String::from_utf16_lossy(&key[..length as usize]);
        if registry_string(root, &HSTRING::from(&name), w!("Layout Id"))
            .and_then(|value| u16::from_str_radix(&value, 16).ok())
            == Some(id)
        {
            return Some(name);
        }
    }
    None
}

fn registry_string(root: &LayoutKey, key: &HSTRING, value: PCWSTR) -> Option<String> {
    let mut text = [0u16; 256];
    let mut bytes = size_of_val(&text) as u32;
    unsafe {
        RegGetValueW(
            root.0,
            key,
            value,
            RRF_RT_REG_SZ,
            None,
            Some(text.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    }
    .ok()
    .ok()?;
    Some(wide_string(&text))
}

fn wide_string(text: &[u16]) -> String {
    let end = text
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use windows::Win32::System::Threading::GetCurrentThreadId;

    #[test]
    fn layout_request_switches_a_native_window_when_two_layouts_are_loaded() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || unsafe {
            let active = GetKeyboardLayout(0);
            let Some(layout) = available(active)
                .into_iter()
                .find(|&layout| layout != active)
            else {
                eprintln!("Only one keyboard layout is loaded in this Windows session");
                sender.send(None).unwrap();
                return;
            };
            let window = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
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
            let target = Target {
                window,
                thread: GetCurrentThreadId(),
            };
            sender
                .send(Some((
                    target.window.0 as isize,
                    target.thread,
                    layout.0 as isize,
                )))
                .unwrap();
            let start = Instant::now();
            let mut message = MSG::default();
            while GetKeyboardLayout(0) != layout && start.elapsed() < Duration::from_secs(2) {
                if PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&message);
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            let changed = GetKeyboardLayout(0) == layout;
            DestroyWindow(window).unwrap();
            assert!(changed, "focused thread did not accept the layout request");
        });
        if let Some((window, thread, layout)) =
            receiver.recv_timeout(Duration::from_secs(2)).unwrap()
        {
            Target {
                window: HWND(window as *mut _),
                thread,
            }
            .select(HKL(layout as *mut _))
            .unwrap();
        }
        worker.join().unwrap();
    }
}
