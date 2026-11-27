//! Read destinations on the shell worker; never change an application's Jump List.
use super::app_identity;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::Common::IObjectArray;
use windows::Win32::UI::Shell::*;
use windows::core::{HSTRING, Interface, Result};

#[derive(Clone)]
pub enum Destination {
    Item(String),
    Link(Vec<u8>),
}
#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub section: String,
    pub translate_section: bool,
    pub destination: Destination,
}

fn item_name(item: &IShellItem, kind: SIGDN) -> Result<String> {
    unsafe {
        let value = item.GetDisplayName(kind)?;
        let result = value.to_string();
        CoTaskMemFree(Some(value.0.cast()));
        Ok(result?)
    }
}

pub fn app_id(target: &str) -> Option<String> {
    if target.to_lowercase().starts_with("shell:appsfolder\\") {
        return Some(target[17..].to_owned());
    }
    if target.to_lowercase().ends_with(".lnk") {
        let link = app_identity::load_link(target).ok()?;
        if let Some(id) = app_identity::link_id(&link) {
            return Some(id);
        }
        let resolved = app_identity::shortcut_identity(target).ok()?;
        return (resolved != target).then(|| app_id(&resolved)).flatten();
    }
    if std::path::Path::new(target).is_absolute() && target.to_lowercase().ends_with(".exe") {
        // Desktop apps without an explicit AppID use their executable identity.
        return Some(target.to_owned());
    }
    None
}

pub fn read(target: &str, id: Option<String>) -> Vec<Entry> {
    let Some(id) = id.or_else(|| app_id(target)) else {
        return Vec::new();
    };
    let mut entries = custom(&id);
    unsafe {
        if let Ok(lists) = CoCreateInstance::<_, IApplicationDocumentLists>(
            &ApplicationDocumentLists,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            if lists.SetAppID(&HSTRING::from(&id)).is_ok() {
                if let Ok(items) = lists.GetList::<IObjectArray>(ADLT_RECENT, 10) {
                    for index in 0..items.GetCount().unwrap_or(0).min(10) {
                        if let Ok(item) = items.GetAt::<IShellItem>(index) {
                            if let (Ok(name), Ok(path)) = (
                                item_name(&item, SIGDN_NORMALDISPLAY),
                                item_name(&item, SIGDN_DESKTOPABSOLUTEPARSING),
                            ) {
                                entries.push(Entry {
                                    name,
                                    section: "Recent".into(),
                                    translate_section: true,
                                    destination: Destination::Item(path),
                                });
                            }
                        } else if let Ok(link) = items.GetAt::<IShellLinkW>(index) {
                            if let Some(name) = link_name(&link)
                                && let Some(data) = save_link(&link)
                            {
                                entries.push(Entry {
                                    name,
                                    section: "Recent".into(),
                                    translate_section: true,
                                    destination: Destination::Link(data),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    entries
}

// Explorer's custom-destination cache is not a public API. Keep this optional,
// bounded, read-only, and let Windows deserialize each embedded Shell Link.
fn custom(id: &str) -> Vec<Entry> {
    use std::io::Read;
    let Some(root) = std::env::var_os("APPDATA") else {
        return vec![];
    };
    let path = std::path::Path::new(&root)
        .join("Microsoft/Windows/Recent/CustomDestinations")
        .join(format!("{:016x}.customDestinations-ms", app_hash(id)));
    let Ok(file) = std::fs::File::open(path) else {
        return vec![];
    };
    let mut bytes = Vec::new();
    if file
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > 4 * 1024 * 1024
    {
        return vec![];
    }
    parse_custom(&bytes)
}

fn app_hash(id: &str) -> u64 {
    let mut crc = u64::MAX;
    for byte in id.to_uppercase().encode_utf16().flat_map(u16::to_le_bytes) {
        crc ^= u64::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0x92c64265d32139a4 } else { 0 };
        }
    }
    crc
}

fn parse_custom(bytes: &[u8]) -> Vec<Entry> {
    // Version 2 category container; unknown versions are deliberately ignored.
    if bytes.get(..4) != Some(&2u32.to_le_bytes()) {
        return vec![];
    }
    let mut offset = 4;
    let Some(categories) = take_u32(bytes, &mut offset).filter(|&count| count <= 64) else {
        return vec![];
    };
    offset += 4;
    let mut entries = Vec::new();
    for _ in 0..categories {
        let Some(kind) = take_u32(bytes, &mut offset) else {
            break;
        };
        let section = match kind {
            0 => {
                let Some(length) = bytes.get(offset..offset + 2) else {
                    break;
                };
                let length = u16::from_le_bytes([length[0], length[1]]) as usize;
                offset += 2;
                let Some(text) = bytes.get(offset..offset + length * 2) else {
                    break;
                };
                offset += length * 2;
                String::from_utf16_lossy(
                    &text
                        .chunks_exact(2)
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        .collect::<Vec<_>>(),
                )
            }
            1 => {
                if take_u32(bytes, &mut offset).is_none() {
                    break;
                }
                if take_u32(bytes, &mut offset) != Some(0xbabffbab) {
                    break;
                }
                continue;
            }
            2 => "Tasks".into(),
            _ => break,
        };
        let Some(count) = take_u32(bytes, &mut offset).filter(|&count| count <= 128) else {
            break;
        };
        for _ in 0..count {
            // OleSaveToStream prefixes the persisted object with its CLSID.
            const CLSID: [u8; 16] = [1, 20, 2, 0, 0, 0, 0, 0, 192, 0, 0, 0, 0, 0, 0, 70];
            if bytes.get(offset..offset + 16) != Some(&CLSID) {
                return entries;
            }
            offset += 16;
            let Some((link, size)) = load_link(&bytes[offset..]) else {
                return entries;
            };
            let data = bytes[offset..offset + size].to_vec();
            offset += size;
            if let Some(name) = link_name(&link) {
                entries.push(Entry {
                    name,
                    section: section.clone(),
                    translate_section: kind == 2,
                    destination: Destination::Link(data),
                });
                if entries.len() >= 30 {
                    return entries;
                }
            }
        }
        if take_u32(bytes, &mut offset) != Some(0xbabffbab) {
            break;
        }
    }
    entries
}
fn take_u32(bytes: &[u8], offset: &mut usize) -> Option<u32> {
    let value = u32::from_le_bytes(bytes.get(*offset..*offset + 4)?.try_into().ok()?);
    *offset += 4;
    Some(value)
}
fn load_link(bytes: &[u8]) -> Option<(IShellLinkW, usize)> {
    unsafe {
        let stream = SHCreateMemStream(Some(bytes))?;
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        link.cast::<IPersistStream>().ok()?.Load(&stream).ok()?;
        let mut position = 0;
        stream.Seek(0, STREAM_SEEK_CUR, Some(&mut position)).ok()?;
        (position > 0 && position <= bytes.len() as u64).then_some((link, position as usize))
    }
}
fn save_link(link: &IShellLinkW) -> Option<Vec<u8>> {
    unsafe {
        let stream = SHCreateMemStream(None)?;
        link.cast::<IPersistStream>()
            .ok()?
            .Save(&stream, true)
            .ok()?;
        let mut size = 0;
        stream.Seek(0, STREAM_SEEK_CUR, Some(&mut size)).ok()?;
        if size == 0 || size > 1024 * 1024 {
            return None;
        }
        let mut bytes = vec![0; size as usize];
        stream.Seek(0, STREAM_SEEK_SET, None).ok()?;
        let mut read = 0;
        stream
            .Read(
                bytes.as_mut_ptr().cast(),
                bytes.len() as u32,
                Some(&mut read),
            )
            .ok()
            .ok()?;
        (read as usize == bytes.len()).then_some(bytes)
    }
}
fn link_name(link: &IShellLinkW) -> Option<String> {
    // PKEY_Title; empty/separator tasks are intentionally omitted.
    let store = link.cast().ok()?;
    app_identity::string(
        &store,
        &windows::Win32::Foundation::PROPERTYKEY {
            fmtid: windows::core::GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9),
            pid: 2,
        },
    )
    .or_else(|| unsafe {
        let mut description = [0; 1024];
        link.GetDescription(&mut description).ok()?;
        let name = app_identity::buffer_text(&description);
        (!name.is_empty()).then_some(name)
    })
}

pub fn invoke(destination: &Destination) -> Result<()> {
    match destination {
        Destination::Item(target) => super::shell::launch(target),
        Destination::Link(bytes) => unsafe {
            let (link, _) = load_link(bytes).ok_or_else(windows::core::Error::empty)?;
            let mut file = [0; 32768];
            link.GetPath(&mut file, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)?;
            let mut args = [0; 32768];
            link.GetArguments(&mut args)?;
            let mut directory = [0; 32768];
            link.GetWorkingDirectory(&mut directory)?;
            let pidl = link.GetIDList()?;
            let mut info = SHELLEXECUTEINFOW {
                cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
                lpFile: windows::core::PCWSTR(file.as_ptr()),
                lpParameters: windows::core::PCWSTR(args.as_ptr()),
                lpDirectory: windows::core::PCWSTR(directory.as_ptr()),
                nShow: link.GetShowCmd()?.0,
                ..Default::default()
            };
            if !pidl.is_null() {
                info.fMask |= SEE_MASK_INVOKEIDLIST;
                info.lpIDList = pidl.cast();
            }
            let result = ShellExecuteExW(&mut info);
            CoTaskMemFree(Some(pidl.cast()));
            result
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_tasks_round_trip_native_shell_links() -> Result<()> {
        let _ole = crate::files::Ole::new()?;
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(r"C:\Windows\notepad.exe"))?;
            link.SetArguments(&HSTRING::from(r#""C:\Documents\a b.txt""#))?;
            link.SetDescription(&HSTRING::from("Open test document"))?;
            let stream = SHCreateMemStream(None).unwrap();
            link.cast::<IPersistStream>()?.Save(&stream, true)?;
            let mut size = 0;
            stream.Seek(0, STREAM_SEEK_CUR, Some(&mut size))?;
            stream.Seek(0, STREAM_SEEK_SET, None)?;
            let mut data = vec![0; size as usize];
            let mut read = 0;
            stream
                .Read(data.as_mut_ptr().cast(), data.len() as u32, Some(&mut read))
                .ok()?;
            assert_eq!(read as usize, data.len());
            let mut bytes = Vec::new();
            for value in [2u32, 1, 0, 2, 1] {
                bytes.extend(value.to_le_bytes());
            }
            bytes.extend([1, 20, 2, 0, 0, 0, 0, 0, 192, 0, 0, 0, 0, 0, 0, 70]);
            bytes.extend(&data);
            bytes.extend(0xbabffbabu32.to_le_bytes());
            let entries = parse_custom(&bytes);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].name, "Open test document");
            assert_eq!(entries[0].section, "Tasks");
            let Destination::Link(restored) = &entries[0].destination else {
                panic!("expected link");
            };
            let (restored, _) = load_link(restored).unwrap();
            let mut arguments = [0; 1024];
            restored.GetArguments(&mut arguments)?;
            assert_eq!(
                app_identity::buffer_text(&arguments),
                r#""C:\Documents\a b.txt""#
            );
            // Truncation must not create a partially deserialized executable task.
            assert!(parse_custom(&bytes[..30]).is_empty());
        }
        Ok(())
    }
    #[test]
    fn malformed_custom_destinations_are_ignored() {
        for bytes in [
            vec![],
            vec![2, 0, 0, 0],
            vec![0; 100],
            vec![255; 100],
            vec![2, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 0],
        ] {
            assert!(parse_custom(&bytes).is_empty());
        }
        assert_eq!(app_hash("Chrome"), 0x5d696d521de238c3);
        assert_eq!(
            app_id(r"shell:AppsFolder\MSEdge.Profile2"),
            Some("MSEdge.Profile2".into())
        );
    }
}
