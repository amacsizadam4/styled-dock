//! Window and shortcut identities must survive hosted apps sharing an executable.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
thread_local! {
    static RELAUNCH: RefCell<HashMap<String, (String, String, String)>> = RefCell::new(HashMap::new());
}
use windows::Win32::Foundation::{E_FAIL, HWND, PROPERTYKEY};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::PropertiesSystem::*;
use windows::Win32::UI::Shell::*;
use windows::core::{GUID, HSTRING, Interface, Result};

pub fn property(pid: u32) -> PROPERTYKEY {
    PROPERTYKEY {
        fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid,
    }
}

pub fn string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let mut value = store.GetValue(key).ok()?;
        let result = PropVariantToStringAlloc(&value);
        let _ = PropVariantClear(&mut value);
        let text = result.ok()?;
        let result = text.to_string().ok().filter(|text| !text.is_empty());
        CoTaskMemFree(Some(text.0.cast()));
        result
    }
}

pub fn load_link(path: &str) -> Result<IShellLinkW> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.cast::<IPersistFile>()?
            .Load(&HSTRING::from(path), STGM_READ)?;
        Ok(link)
    }
}

pub fn link_id(link: &IShellLinkW) -> Option<String> {
    string(&link.cast::<IPropertyStore>().ok()?, &property(5))
}

pub fn buffer_text(buffer: &[u16]) -> String {
    String::from_utf16_lossy(
        &buffer[..buffer
            .iter()
            .position(|&ch| ch == 0)
            .unwrap_or(buffer.len())],
    )
}

pub fn shortcut_identity(path: &str) -> Result<String> {
    let link = load_link(path)?;
    if let Some(id) = link_id(&link) {
        return Ok(format!("shell:AppsFolder\\{id}"));
    }
    unsafe {
        let mut target = [0; 32768];
        link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)?;
        let target = buffer_text(&target);
        let mut arguments = [0; 32768];
        link.GetArguments(&mut arguments)?;
        // A shortcut with arguments is not interchangeable with its host executable.
        if !buffer_text(&arguments).is_empty() {
            return Ok(path.to_owned());
        }
        if target.is_empty() {
            return Err(windows::core::Error::from_hresult(E_FAIL));
        }
        Ok(target)
    }
}

pub fn window_id(hwnd: HWND) -> Option<String> {
    let store: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(hwnd).ok()? };
    string(&store, &property(5))
}

// Keep the original command tail byte-for-byte; CommandLineToArgvW would lose quoting.
fn split_command(command: &str) -> Option<(&str, &str)> {
    let command = command.trim();
    let (file, tail) = if let Some(rest) = command.strip_prefix('"') {
        let end = rest.find('"')?;
        (&rest[..end], &rest[end + 1..])
    } else {
        command
            .split_once(char::is_whitespace)
            .unwrap_or((command, ""))
    };
    (!file.is_empty()).then_some((file, tail.trim_start()))
}

/// A durable shell shortcut preserves PWA/profile arguments and icon when pinned.
pub fn relaunch_link(hwnd: HWND, id: &str, title: &str) -> Option<(String, String)> {
    let store: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(hwnd).ok()? };
    let command = string(&store, &property(2))?;
    let (file, arguments) = split_command(&command)?;
    if arguments.is_empty() {
        return Some((file.to_owned(), title.to_owned()));
    }
    let display = string(&store, &property(4)).unwrap_or_else(|| title.to_owned());
    let icon = string(&store, &property(3));
    let signature = format!("{command}\0{display}\0{}", icon.as_deref().unwrap_or(""));
    if let Some(cached) = RELAUNCH.with_borrow(|cache| {
        cache
            .get(id)
            .filter(|(saved, _, _)| saved == &signature)
            .map(|(_, path, name)| (path.clone(), name.clone()))
    }) {
        return Some(cached);
    }
    let mut name = [0u16; 1024];
    let name = if display.starts_with('@') {
        unsafe {
            SHLoadIndirectString(&HSTRING::from(&display), &mut name, None).ok()?;
        }
        buffer_text(&name)
    } else {
        display
    };
    let root = std::env::var_os("LOCALAPPDATA")?;
    let folder = Path::new(&root).join("StyledDock").join("AppLinks");
    std::fs::create_dir_all(&folder).ok()?;
    // A fixed-size name supports Chromium IDs longer than a Windows component.
    // Verify the stored ID before reusing it rather than trusting a hash collision.
    let hash = id
        .to_lowercase()
        .bytes()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    let path = folder.join(format!("{hash:016x}.lnk"));
    if path.exists()
        && let Ok(existing) = load_link(&path.to_string_lossy())
        && let Some(existing_id) = link_id(&existing)
        && !existing_id.eq_ignore_ascii_case(id)
    {
        return None;
    }
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        link.SetPath(&HSTRING::from(file)).ok()?;
        link.SetArguments(&HSTRING::from(arguments)).ok()?;
        link.SetDescription(&HSTRING::from(&name)).ok()?;
        if let Some(icon) = icon {
            let (file, index) = icon
                .rsplit_once(',')
                .map_or((icon.as_str(), 0), |(file, index)| {
                    (file, index.trim().parse().unwrap_or(0))
                });
            let _ = link.SetIconLocation(&HSTRING::from(file.trim_matches('"')), index);
        }
        let link_store = link.cast::<IPropertyStore>().ok()?;
        let value = windows::Win32::System::Com::StructuredStorage::PROPVARIANT::from(id);
        link_store.SetValue(&property(5), &value).ok()?;
        link_store.Commit().ok()?;
        link.cast::<IPersistFile>()
            .ok()?
            .Save(&HSTRING::from(path.as_os_str()), true)
            .ok()?;
    }
    let target = path.to_string_lossy().into_owned();
    RELAUNCH.with_borrow_mut(|cache| {
        cache.insert(id.to_owned(), (signature, target.clone(), name.clone()));
    });
    Some((target, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn window_relaunch_preserves_hosted_app_identity_and_profile() -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::*;
        let _ole = crate::files::Ole::new()?;
        let id = format!("StyledDock.Test.WhatsApp.Profile2.{}", std::process::id());
        let mut saved = None;
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("STATIC"),
                windows::core::w!("WhatsApp test"),
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
            let result = (|| {
                let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd)?;
                for (pid, text) in [
                    (
                        2,
                        r#""C:\Program Files\Edge\msedge_proxy.exe" --profile-directory="Profile 2" --app-id=whatsapp"#,
                    ),
                    (4, "WhatsApp"),
                    (5, id.as_str()),
                ] {
                    store.SetValue(
                        &property(pid),
                        &windows::Win32::System::Com::StructuredStorage::PROPVARIANT::from(text),
                    )?;
                }
                assert_eq!(window_id(hwnd).as_deref(), Some(id.as_str()));
                let (target, name) =
                    relaunch_link(hwnd, &id, "Changing chat title").expect("relaunch shortcut");
                saved = Some(target.clone());
                assert_eq!(name, "WhatsApp");
                let link = load_link(&target)?;
                assert_eq!(link_id(&link).as_deref(), Some(id.as_str()));
                let mut args = [0; 1024];
                link.GetArguments(&mut args)?;
                assert_eq!(
                    buffer_text(&args),
                    r#"--profile-directory="Profile 2" --app-id=whatsapp"#
                );
                assert_eq!(
                    relaunch_link(hwnd, &id, "Another chat"),
                    Some((target, name))
                );
                Ok(())
            })();
            let _ = DestroyWindow(hwnd);
            if let Some(target) = saved {
                let _ = std::fs::remove_file(target);
            }
            RELAUNCH.with_borrow_mut(|cache| {
                cache.remove(&id);
            });
            result
        }
    }
    #[test]
    fn shortcut_identity_preserves_app_id_and_argument_only_links() -> Result<()> {
        let _ole = crate::files::Ole::new()?;
        let path =
            std::env::temp_dir().join(format!("styled-dock-identity-{}.lnk", std::process::id()));
        let result = (|| unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(r"C:\Edge\msedge_proxy.exe"))?;
            link.SetArguments(&HSTRING::from(
                r#"--app-id=whatsapp --profile-directory="Profile 2""#,
            ))?;
            link.cast::<IPersistFile>()?
                .Save(&HSTRING::from(path.as_os_str()), true)?;
            assert_eq!(
                shortcut_identity(&path.to_string_lossy())?,
                path.to_string_lossy()
            );
            let store = link.cast::<IPropertyStore>()?;
            store.SetValue(
                &property(5),
                &windows::Win32::System::Com::StructuredStorage::PROPVARIANT::from(
                    "MSEdge._crx_whatsapp.Profile2",
                ),
            )?;
            store.Commit()?;
            link.cast::<IPersistFile>()?
                .Save(&HSTRING::from(path.as_os_str()), true)?;
            assert_eq!(
                shortcut_identity(&path.to_string_lossy())?,
                r"shell:AppsFolder\MSEdge._crx_whatsapp.Profile2"
            );
            let restored = load_link(&path.to_string_lossy())?;
            let mut args = [0; 1024];
            restored.GetArguments(&mut args)?;
            assert_eq!(
                buffer_text(&args),
                r#"--app-id=whatsapp --profile-directory="Profile 2""#
            );
            Ok(())
        })();
        let _ = std::fs::remove_file(path);
        result
    }
    #[test]
    fn relaunch_command_preserves_browser_profile_and_app_arguments() {
        assert_eq!(
            split_command(
                r#""C:\Program Files\Microsoft\Edge\Application\msedge_proxy.exe" --profile-directory="Profile 2" --app-id=whatsapp"#
            ),
            Some((
                r"C:\Program Files\Microsoft\Edge\Application\msedge_proxy.exe",
                r#"--profile-directory="Profile 2" --app-id=whatsapp"#
            ))
        );
        assert_eq!(
            split_command("  notepad.exe  file.txt "),
            Some(("notepad.exe", "file.txt"))
        );
        assert!(split_command("\"unterminated").is_none());
    }
}
