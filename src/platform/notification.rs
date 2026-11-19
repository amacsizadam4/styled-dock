//! Styled Dock's notification icon, independent of optional tray mirroring.
use std::mem::size_of;
use windows::Win32::{
    Foundation::{HWND, POINT},
    UI::{Shell::*, WindowsAndMessaging::*},
};
use windows::core::Result;

pub const WM_NOTIFY_ICON: u32 = WM_APP + 35;
pub enum Action {
    Settings,
    Restore,
    Quit,
}
pub struct Notification(NOTIFYICONDATAW);
impl Notification {
    pub fn new(hwnd: HWND) -> Result<Self> {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 0x5344,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_NOTIFY_ICON,
            hIcon: unsafe { LoadIconW(None, IDI_APPLICATION)? },
            ..Default::default()
        };
        let title: Vec<_> = "Styled Dock".encode_utf16().collect();
        data.szTip[..title.len()].copy_from_slice(&title);
        let icon = Self(data);
        icon.register();
        Ok(icon)
    }
    pub fn register(&self) {
        unsafe {
            if Shell_NotifyIconW(NIM_ADD, &self.0).as_bool() {
                let mut version = self.0;
                version.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &version);
            }
        }
    }
    pub fn handle(&self, event: u32, mut point: POINT) -> Result<Option<Action>> {
        if matches!(event, NIN_SELECT | WM_LBUTTONUP | WM_LBUTTONDBLCLK)
            || event == (NIN_SELECT | 1)
        {
            return Ok(Some(Action::Settings));
        }
        if event != WM_CONTEXTMENU && event != WM_RBUTTONUP {
            return Ok(None);
        }
        if event == WM_RBUTTONUP || point.x == -1 && point.y == -1 {
            unsafe {
                GetCursorPos(&mut point)?;
            }
        }
        let mut menu = crate::popup::Popup::new(self.0.hWnd, 300)?;
        menu.heading("Styled Dock")?;
        menu.action(1, crate::i18n::text("Dock Settings…"))?;
        menu.action(2, crate::i18n::text("Restore Windows taskbar and quit"))?;
        menu.action(3, crate::i18n::text("Quit Styled Dock"))?;
        Ok(
            match menu.track(self.0.hWnd, point, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN) {
                1 => Some(Action::Settings),
                2 => Some(Action::Restore),
                3 => Some(Action::Quit),
                _ => None,
            },
        )
    }
}
impl Drop for Notification {
    fn drop(&mut self) {
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::core::w;
    #[test]
    fn own_icon_registers_reannounces_selects_and_deletes_through_the_mirror() -> Result<()> {
        let _ole = crate::files::Ole::new()?;
        let instance = unsafe { GetModuleHandleW(None)? }.into();
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Own notification icon test"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance),
                None,
            )?
        };
        let tray = crate::tray::Tray::with_cache(instance, hwnd, None)?;
        let icon = Notification::new(hwnd)?;
        assert!(
            tray.state()
                .icons()
                .any(|entry| entry.menu_key().0 == hwnd.0 as isize && entry.title == "Styled Dock")
        );
        icon.register();
        assert_eq!(
            tray.state()
                .icons()
                .filter(|entry| entry.menu_key().0 == hwnd.0 as isize)
                .count(),
            1
        );
        assert!(matches!(
            icon.handle(NIN_SELECT, POINT::default())?,
            Some(Action::Settings)
        ));
        assert!(matches!(
            icon.handle(NIN_SELECT | 1, POINT::default())?,
            Some(Action::Settings)
        ));
        drop(icon);
        assert!(
            !tray
                .state()
                .icons()
                .any(|entry| entry.menu_key().0 == hwnd.0 as isize)
        );
        drop(tray);
        let _ = unsafe { DestroyWindow(hwnd) };
        Ok(())
    }
}
