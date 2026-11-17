//! A bounded snapshot of copied pixels and callback metadata, never raw HICONs.
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, FILETIME, HWND};
use windows::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};
use windows::core::HSTRING;

use super::{Icon, TrayIcon, TrayState};

const MAGIC: &[u8; 8] = b"SDTRAY01";
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_ICONS: usize = 256;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct ProcessIdentity {
    pid: u32,
    created: u64,
}

impl ProcessIdentity {
    pub(super) fn for_window(window: HWND) -> Option<Self> {
        unsafe {
            if !IsWindow(Some(window)).as_bool() {
                return None;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(window, Some(&mut pid));
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let (mut created, mut exited, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            let result =
                GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
            let _ = CloseHandle(process);
            result.ok()?;
            Some(Self {
                pid,
                created: (created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64,
            })
        }
    }

    pub(super) fn matches(&self, window: HWND) -> bool {
        Self::for_window(window).as_ref() == Some(self)
    }
}

pub(super) fn path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|root| {
        PathBuf::from(root)
            .join("StyledDock")
            .join("tray-cache.bin")
    })
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid tray snapshot")
}

fn number(bytes: &mut &[u8]) -> io::Result<u32> {
    let mut value = [0; 4];
    bytes.read_exact(&mut value)?;
    Ok(u32::from_le_bytes(value))
}

fn decode(mut bytes: &[u8], owner: HWND) -> io::Result<TrayState> {
    if bytes.len() > MAX_BYTES || !bytes.starts_with(MAGIC) {
        return Err(invalid());
    }
    bytes = &bytes[MAGIC.len()..];
    let count = number(&mut bytes)? as usize;
    if count > MAX_ICONS {
        return Err(invalid());
    }
    let mut state = TrayState {
        owner,
        ..Default::default()
    };
    for _ in 0..count {
        let window = HWND(number(&mut bytes)? as usize as *mut _);
        let identity = ProcessIdentity {
            pid: number(&mut bytes)?,
            created: number(&mut bytes)? as u64 | (number(&mut bytes)? as u64) << 32,
        };
        let id = number(&mut bytes)?;
        let callback = number(&mut bytes)?;
        let version = number(&mut bytes)?;
        let hidden = number(&mut bytes)? != 0;
        let mut guid = [0; 16];
        bytes.read_exact(&mut guid)?;
        let width = number(&mut bytes)?;
        let height = number(&mut bytes)?;
        if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
            return Err(invalid());
        }
        let title_len = number(&mut bytes)? as usize;
        if title_len > 4096 || title_len > bytes.len() {
            return Err(invalid());
        }
        let title = std::str::from_utf8(&bytes[..title_len])
            .map_err(|_| invalid())?
            .to_owned();
        bytes = &bytes[title_len..];
        let pixel_len = (width * height * 4) as usize;
        if pixel_len > bytes.len() {
            return Err(invalid());
        }
        let pixels = bytes[..pixel_len].to_vec();
        bytes = &bytes[pixel_len..];
        // A recycled HWND/PID must never inherit another app's click target.
        if identity.matches(window) {
            state.next_serial += 1;
            state.items.push(TrayIcon {
                serial: state.next_serial,
                image: Icon {
                    width,
                    height,
                    pixels,
                },
                title,
                window,
                id,
                callback,
                version,
                guid,
                hidden,
                identity: Some(identity),
            });
        }
    }
    if !bytes.is_empty() {
        return Err(invalid());
    }
    Ok(state)
}

pub(super) fn load(path: &Path, owner: HWND) -> TrayState {
    let read = || -> io::Result<TrayState> {
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        decode(&bytes, owner)
    };
    read().unwrap_or_else(|_| TrayState {
        owner,
        ..Default::default()
    })
}

fn encode(state: &TrayState) -> io::Result<Vec<u8>> {
    let icons: Vec<_> = state
        .items
        .iter()
        .filter_map(|icon| {
            icon.identity
                .filter(|identity| identity.matches(icon.window))
                .map(|identity| (icon, identity))
        })
        .take(MAX_ICONS)
        .collect();
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&(icons.len() as u32).to_le_bytes());
    for (icon, identity) in icons {
        if icon.title.len() > 4096
            || !(1..=256).contains(&icon.image.width)
            || !(1..=256).contains(&icon.image.height)
            || icon.image.pixels.len() != (icon.image.width * icon.image.height * 4) as usize
        {
            return Err(invalid());
        }
        for value in [
            icon.window.0 as usize as u32,
            identity.pid,
            identity.created as u32,
            (identity.created >> 32) as u32,
            icon.id,
            icon.callback,
            icon.version,
            u32::from(icon.hidden),
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&icon.guid);
        for value in [icon.image.width, icon.image.height, icon.title.len() as u32] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(icon.title.as_bytes());
        bytes.extend_from_slice(&icon.image.pixels);
        if bytes.len() > MAX_BYTES {
            return Err(invalid());
        }
    }
    Ok(bytes)
}

pub(super) fn save(path: &Path, state: &TrayState) -> io::Result<()> {
    let bytes = encode(state)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&temporary, bytes)?;
    // Keep the previous complete snapshot if a write or replacement fails.
    let result = unsafe {
        MoveFileExW(
            &HSTRING::from(temporary.as_os_str()),
            &HSTRING::from(path.as_os_str()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if let Err(error) = result {
        let _ = fs::remove_file(temporary);
        return Err(io::Error::other(error.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP,
    };
    use windows::core::w;

    #[test]
    fn snapshot_keeps_hidden_icons_and_rejects_dead_or_reused_owners() -> windows::core::Result<()>
    {
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Tray cache owner"),
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
        };
        let mut state = TrayState::default();
        state.items.push(TrayIcon {
            serial: 1,
            image: Icon {
                width: 1,
                height: 1,
                pixels: vec![0, 0, 0, 255],
            },
            title: "Hidden app".into(),
            window,
            id: 7,
            callback: 0x8001,
            version: 4,
            guid: [3; 16],
            hidden: true,
            identity: ProcessIdentity::for_window(window),
        });
        let bytes = encode(&state).unwrap();
        let restored = decode(&bytes, window).unwrap();
        assert_eq!(restored.items.len(), 1);
        assert!(restored.items[0].hidden);
        assert_eq!(restored.items[0].guid, [3; 16]);
        assert_eq!(restored.icons().count(), 0);
        for length in 0..bytes.len() {
            assert!(decode(&bytes[..length], window).is_err());
        }
        let mut reused = bytes.clone();
        // Keep a valid HWND, but substitute another process creation time.
        reused[20] ^= 1;
        assert!(decode(&reused, window).unwrap().items.is_empty());
        unsafe { DestroyWindow(window)? };
        assert!(decode(&bytes, HWND::default()).unwrap().items.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_truncated_oversized_and_invalid_snapshots() {
        let empty = encode(&TrayState::default()).unwrap();
        for length in 0..empty.len() {
            assert!(decode(&empty[..length], HWND::default()).is_err());
        }
        assert!(decode(&empty, HWND::default()).unwrap().items.is_empty());
        let mut oversized = MAGIC.to_vec();
        oversized.extend_from_slice(&(MAX_ICONS as u32 + 1).to_le_bytes());
        assert!(decode(&oversized, HWND::default()).is_err());
        assert!(decode(b"not a snapshot", HWND::default()).is_err());
    }
}
