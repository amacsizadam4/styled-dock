use std::collections::VecDeque;
use std::mem::size_of;
use std::sync::mpsc::{self, Receiver, Sender};

use windows::Win32::Foundation::{HWND, LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_SHOWNORMAL, WM_APP};
use windows::core::{Error, HSTRING, Result};

use crate::config::{Pin, is_packaged};
use crate::system::SystemAction;

pub const WM_SHELL_RESULT: u32 = WM_APP + 2;

pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub enum Event {
    Icon(String, Icon),
    LaunchFailed(String, Error),
    SettingsFailed(Error),
    SystemFailed(SystemAction, Error),
    Shortcut(String, String),
    WorkerFailed(Error),
}

pub enum Request {
    Launch(Pin),
    LaunchFiles(Pin, Vec<String>),
    Settings(&'static str),
    System(SystemAction),
    Icon(String),
}

pub fn start(hwnd: HWND, pins: Vec<Pin>) -> std::io::Result<(Sender<Request>, Receiver<Event>)> {
    let (launches, requests) = mpsc::channel();
    let (results, events) = mpsc::channel();
    let handle = hwnd.0 as isize;
    std::thread::Builder::new()
        .name("dock-shell".into())
        .spawn(move || {
            let hwnd = HWND(handle as *mut _);
            let notify = |event| {
                results.send(event).is_ok()
                    && unsafe {
                        PostMessageW(Some(hwnd), WM_SHELL_RESULT, WPARAM(0), LPARAM(0)).is_ok()
                    }
            };
            let apartment = unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()
            };
            if let Err(error) = apartment {
                notify(Event::WorkerFailed(error));
                return;
            }

            {
                let imaging: Result<IWICImagingFactory> = unsafe {
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                };
                for pin in pins {
                    if pin.target.to_ascii_lowercase().ends_with(".lnk")
                        && let Ok(target) = shortcut_target(&pin.target)
                    {
                        notify(Event::Shortcut(pin.target, target));
                    }
                }
                let mut pending_icons = VecDeque::new();
                loop {
                    let first = if pending_icons.is_empty() {
                        let Ok(request) = requests.recv() else { break };
                        Some(request)
                    } else {
                        None
                    };
                    for request in first.into_iter().chain(requests.try_iter()) {
                        match request {
                            Request::Launch(pin) => {
                                if let Err(error) = launch(&pin.target) {
                                    notify(Event::LaunchFailed(pin.name, error));
                                }
                            }
                            Request::LaunchFiles(pin, paths) => {
                                if let Err(error) = launch_files(&pin.target, &paths) {
                                    notify(Event::LaunchFailed(pin.name, error));
                                }
                            }
                            Request::Settings(target) => {
                                if let Err(error) = launch(target) {
                                    notify(Event::SettingsFailed(error));
                                }
                            }
                            Request::System(action) => {
                                if let Err(error) = action.execute() {
                                    notify(Event::SystemFailed(action, error));
                                }
                            }
                            Request::Icon(target) => pending_icons.push_back(target),
                        }
                    }
                    if let Some(target) = pending_icons.pop_front()
                        && let Ok(imaging) = &imaging
                        && let Ok(icon) = extract(imaging, &target)
                    {
                        notify(Event::Icon(target, icon));
                    }
                }
            }
            unsafe { CoUninitialize() };
        })?;
    Ok((launches, events))
}

fn shortcut_target(path: &str) -> Result<String> {
    use windows::core::Interface;
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.cast::<IPersistFile>()?
            .Load(&HSTRING::from(path), STGM_READ)?;
        let mut target = [0; 4096];
        link.GetPath(&mut target, std::ptr::null_mut(), 0)?;
        let length = target
            .iter()
            .position(|&ch| ch == 0)
            .unwrap_or(target.len());
        if length == 0 {
            return Err(Error::empty());
        }
        Ok(String::from_utf16_lossy(&target[..length]))
    }
}
struct ShellBitmap(HBITMAP);

impl Drop for ShellBitmap {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(self.0.into()) };
    }
}

fn extract(imaging: &IWICImagingFactory, target: &str) -> Result<Icon> {
    let target = if target == "shell:RecycleBinFolder" {
        "::{645FF040-5081-101B-9F08-00AA002F954E}".to_string()
    } else {
        folder_target(target)?
    };
    let target = target.as_str();
    unsafe {
        let item: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(target), None)?;
        let size = SIZE { cx: 128, cy: 128 };
        let bitmap = ShellBitmap(item.GetImage(size, SIIGBF_ICONONLY | SIIGBF_SCALEUP)?);
        let image = imaging.CreateBitmapFromHBITMAP(
            bitmap.0,
            Default::default(),
            WICBitmapUsePremultipliedAlpha,
        )?;
        let image = WICConvertBitmapSource(&GUID_WICPixelFormat32bppPBGRA, &image)?;
        let (mut width, mut height) = (0, 0);
        image.GetSize(&mut width, &mut height)?;
        if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
            return Err(Error::empty());
        }
        let mut pixels = vec![0; (width * height * 4) as usize];
        image.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
        if !pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0) {
            return Err(Error::empty());
        }
        Ok(Icon {
            width,
            height,
            pixels,
        })
    }
}

pub(crate) fn launch(target: &str) -> Result<()> {
    launch_files(target, &[])
}

fn launch_files(target: &str, paths: &[String]) -> Result<()> {
    let resolved = folder_target(target)?;
    let target = resolved.as_str();
    let path = HSTRING::from(target);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    let arguments = HSTRING::from(
        paths
            .iter()
            .map(|path| format!("\"{path}\""))
            .collect::<Vec<_>>()
            .join(" "),
    );
    if !paths.is_empty() {
        info.lpParameters = windows::core::PCWSTR(arguments.as_ptr());
    }
    unsafe {
        if is_packaged(target) {
            let item: IShellItem = SHCreateItemFromParsingName(&path, None)?;
            let id = SHGetIDListFromObject(&item)?;
            info.fMask |= SEE_MASK_INVOKEIDLIST;
            info.lpIDList = id.cast();
            let result = ShellExecuteExW(&mut info);
            CoTaskMemFree(Some(id.cast()));
            result
        } else {
            info.lpFile = windows::core::PCWSTR(path.as_ptr());
            ShellExecuteExW(&mut info)
        }
    }
}

pub(crate) fn folder_target(target: &str) -> Result<String> {
    if target != "shell:Downloads" {
        return Ok(target.to_owned());
    }
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None)?;
        let result = path.to_string();
        CoTaskMemFree(Some(path.0.cast()));
        Ok(result?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_folders_have_windows_shell_icons() -> Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            let result = (|| {
                let imaging: IWICImagingFactory =
                    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
                for target in ["shell:Downloads", "shell:RecycleBinFolder"] {
                    let icon = extract(&imaging, target)?;
                    assert!(icon.width > 0 && icon.height > 0);
                    assert!(icon.pixels.chunks_exact(4).any(|pixel| pixel[3] > 0));
                }
                Ok(())
            })();
            CoUninitialize();
            result
        }
    }
}
