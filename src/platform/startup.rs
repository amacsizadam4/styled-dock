use std::path::PathBuf;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;
use windows::core::{HSTRING, Interface, Result};

fn shortcut() -> Result<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Startup, KF_FLAG_DEFAULT, None)?;
        let result = path.to_string();
        CoTaskMemFree(Some(path.0.cast()));
        Ok(PathBuf::from(result?).join("Styled Dock.lnk"))
    }
}
pub fn enabled() -> bool {
    shortcut().is_ok_and(|path| path.is_file())
}
pub fn toggle() -> Result<()> {
    let path = shortcut()?;
    if path.exists() {
        std::fs::remove_file(path).map_err(|error| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error.to_string())
        })?;
    } else {
        let exe = std::env::current_exe().map_err(|error| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error.to_string())
        })?;
        let cwd = std::env::current_dir().map_err(|error| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error.to_string())
        })?;
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(exe.as_os_str()))?;
            link.SetWorkingDirectory(&HSTRING::from(cwd.as_os_str()))?;
            link.SetDescription(windows::core::w!("Start Styled Dock when signing in"))?;
            link.cast::<IPersistFile>()?
                .Save(&HSTRING::from(path.as_os_str()), true)?;
        }
    }
    Ok(())
}
