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
    set_enabled(!enabled())
}

pub fn set_enabled(enable: bool) -> Result<()> {
    let _ole = crate::files::Ole::new()?;
    let path = shortcut()?;
    let exe = std::env::current_exe().map_err(io_error)?;
    let source = crate::config::Source::open()?;
    let cwd = source.path.parent().ok_or_else(|| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            crate::i18n::text("Configuration directory is unavailable."),
        )
    })?;
    update_shortcut(&path, enable, &exe, cwd)
}

fn io_error(error: std::io::Error) -> windows::core::Error {
    windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error.to_string())
}

fn update_shortcut(
    path: &std::path::Path,
    enable: bool,
    exe: &std::path::Path,
    cwd: &std::path::Path,
) -> Result<()> {
    if !enable {
        if !path.exists() {
            return Ok(());
        }
        std::fs::remove_file(path).map_err(io_error)?;
    } else {
        std::fs::create_dir_all(cwd).map_err(io_error)?;
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(exe.as_os_str()))?;
            link.SetWorkingDirectory(&HSTRING::from(cwd.as_os_str()))?;
            link.SetDescription(&HSTRING::from(crate::i18n::text(
                "Start Styled Dock when signing in",
            )))?;
            link.cast::<IPersistFile>()?
                .Save(&HSTRING::from(path.as_os_str()), true)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_shortcut_round_trips_without_changing_windows_startup() -> Result<()> {
        let _ole = crate::files::Ole::new()?;
        let path =
            std::env::temp_dir().join(format!("styled-dock-startup-{}.lnk", std::process::id()));
        let exe = std::env::current_exe().map_err(io_error)?;
        let cwd = exe.parent().unwrap();
        let result = (|| {
            update_shortcut(&path, false, &exe, cwd)?;
            update_shortcut(&path, true, &exe, cwd)?;
            update_shortcut(&path, true, &exe, cwd)?;
            unsafe {
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
                link.cast::<IPersistFile>()?
                    .Load(&HSTRING::from(path.as_os_str()), STGM_READ)?;
                let mut target = [0; 32768];
                link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)?;
                let length = target.iter().position(|&ch| ch == 0).unwrap();
                assert_eq!(
                    PathBuf::from(String::from_utf16_lossy(&target[..length])),
                    exe
                );
                let mut working = [0; 32768];
                link.GetWorkingDirectory(&mut working)?;
                let length = working.iter().position(|&ch| ch == 0).unwrap();
                assert_eq!(
                    PathBuf::from(String::from_utf16_lossy(&working[..length])),
                    cwd
                );
            }
            update_shortcut(&path, false, &exe, cwd)?;
            update_shortcut(&path, false, &exe, cwd)?;
            assert!(!path.exists());
            Ok(())
        })();
        let _ = std::fs::remove_file(path);
        result
    }
}
