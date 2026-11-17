use crate::config::Pin;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Result, w};

fn text(item: &IShellItem, kind: SIGDN) -> Result<String> {
    unsafe {
        let value = item.GetDisplayName(kind)?;
        let text = value.to_string();
        CoTaskMemFree(Some(value.0.cast()));
        Ok(text?)
    }
}
fn installed() -> Result<Vec<Pin>> {
    unsafe {
        let folder: IShellItem = SHCreateItemFromParsingName(w!("shell:AppsFolder"), None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        let mut apps = Vec::new();
        loop {
            let mut next = [None];
            let mut count = 0;
            items.Next(&mut next, Some(&mut count))?;
            if count == 0 {
                break;
            }
            if let Some(item) = next[0].take() {
                if let (Ok(name), Ok(id)) = (
                    text(&item, SIGDN_NORMALDISPLAY),
                    text(&item, SIGDN_PARENTRELATIVEPARSING),
                ) {
                    apps.push(Pin {
                        name,
                        target: format!("shell:AppsFolder\\{id}"),
                    });
                }
            }
        }
        apps.sort_by_key(|app| app.name.to_lowercase());
        apps.dedup_by(|a, b| a.target == b.target);
        Ok(apps)
    }
}
struct State {
    apps: Vec<Pin>,
    filtered: Vec<usize>,
    choice: Option<Pin>,
}
impl State {
    fn filter(&mut self, hwnd: HWND) {
        unsafe {
            let mut text = [0u16; 512];
            let count = GetDlgItemTextW(hwnd, 1001, &mut text);
            let query = String::from_utf16_lossy(&text[..count as usize]).to_lowercase();
            let Ok(list) = GetDlgItem(Some(hwnd), 1002) else {
                return;
            };
            SendMessageW(list, LB_RESETCONTENT, None, None);
            self.filtered = self
                .apps
                .iter()
                .enumerate()
                .filter(|(_, app)| app.name.to_lowercase().contains(&query))
                .map(|(index, _)| index)
                .collect();
            for &index in &self.filtered {
                let label = HSTRING::from(&self.apps[index].name);
                SendMessageW(
                    list,
                    LB_ADDSTRING,
                    None,
                    Some(LPARAM(label.as_ptr() as isize)),
                );
            }
            SendMessageW(list, LB_SETCURSEL, Some(WPARAM(0)), None);
        }
    }
}
unsafe extern "system" fn dialog(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    if message == WM_INITDIALOG {
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, lparam.0);
        }
    }
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
    let Some(state) = (unsafe { state.as_mut() }) else {
        return 0;
    };
    match message {
        WM_INITDIALOG => {
            state.filter(hwnd);
            1
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xffff;
            let event = wparam.0 >> 16;
            if id == 1001 && event == EN_CHANGE as usize {
                state.filter(hwnd);
            } else if id == IDCANCEL.0 as usize {
                let _ = unsafe { EndDialog(hwnd, 0) };
            } else if id == IDOK.0 as usize || (id == 1002 && event == LBN_DBLCLK as usize) {
                let index =
                    unsafe { SendDlgItemMessageW(hwnd, 1002, LB_GETCURSEL, WPARAM(0), LPARAM(0)) }
                        .0;
                if let Some(&index) = usize::try_from(index)
                    .ok()
                    .and_then(|index| state.filtered.get(index))
                {
                    state.choice = Some(state.apps[index].clone());
                    let _ = unsafe { EndDialog(hwnd, 1) };
                }
            }
            1
        }
        WM_CLOSE => {
            let _ = unsafe { EndDialog(hwnd, 0) };
            1
        }
        _ => 0,
    }
}
pub fn show(owner: HWND) -> Result<Option<Pin>> {
    let _ole = crate::files::Ole::new()?;
    let mut state = State {
        apps: installed()?,
        filtered: Vec::new(),
        choice: None,
    };
    let result = unsafe {
        DialogBoxParamW(
            Some(GetModuleHandleW(None)?.into()),
            windows::core::PCWSTR(102usize as *const u16),
            Some(owner),
            Some(dialog),
            LPARAM((&raw mut state) as isize),
        )
    };
    if result == -1 {
        return Err(windows::core::Error::from_thread());
    }
    Ok(state.choice)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_apps_have_launchable_shell_targets() -> Result<()> {
        let _ole = crate::files::Ole::new()?;
        let apps = installed()?;
        assert!(
            !apps.is_empty(),
            "Windows AppsFolder should expose installed apps"
        );
        for app in apps {
            assert!(!app.name.is_empty());
            let _: IShellItem =
                unsafe { SHCreateItemFromParsingName(&HSTRING::from(app.target), None)? };
        }
        Ok(())
    }
}
