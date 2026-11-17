//! Shell-backed file interactions. Windows owns drag semantics and file operations.
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, Result, w};

unsafe extern "system" fn context_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    if matches!(
        message,
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
    ) {
        let context = unsafe { &*(data as *const IContextMenu) };
        if let Ok(extended) = context.cast::<IContextMenu3>() {
            let mut result = LRESULT(0);
            if unsafe { extended.HandleMenuMsg2(message, wparam, lparam, Some(&mut result)) }
                .is_ok()
            {
                return result;
            }
        } else if let Ok(extended) = context.cast::<IContextMenu2>() {
            if unsafe { extended.HandleMenuMsg(message, wparam, lparam) }.is_ok() {
                return LRESULT(0);
            }
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

pub struct Ole;
impl Ole {
    pub fn new() -> Result<Self> {
        unsafe { OleInitialize(None)? };
        Ok(Self)
    }
}
impl Drop for Ole {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

pub fn reveal(path: &str) -> Result<()> {
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None)?;
        let pidl = SHGetIDListFromObject(&item)?;
        let result = SHOpenFolderAndSelectItems(pidl, None, 0);
        CoTaskMemFree(Some(pidl.cast()));
        result
    }
}

pub fn drag(owner: HWND, path: &str) -> Result<()> {
    let _ole = Ole::new()?;
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None)?;
        let data: IDataObject = item.BindToHandler(None, &BHID_DataObject)?;
        SHDoDragDrop(
            Some(owner),
            &data,
            None,
            DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK,
        )?;
    }
    Ok(())
}

pub fn menu(owner: HWND, path: &str, point: POINT) -> Result<()> {
    let _ole = Ole::new()?;
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None)?;
        let context: IContextMenu = item.BindToHandler(None, &BHID_SFUIObject)?;
        let menu = CreatePopupMenu()?;
        if !SetWindowSubclass(
            owner,
            Some(context_proc),
            0xf11e,
            (&raw const context) as usize,
        )
        .as_bool()
        {
            let _ = DestroyMenu(menu);
            return Err(windows::core::Error::from_thread());
        }
        let result = (|| {
            context
                .QueryContextMenu(menu, 0, 1, 0x6fff, CMF_NORMAL)
                .ok()?;
            AppendMenuW(menu, MF_SEPARATOR, 0, None)?;
            AppendMenuW(menu, MF_STRING, 0x7000, w!("Show in folder"))?;
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                owner,
                None,
            )
            .0 as u32;
            if command == 0x7000 {
                return reveal(path);
            }
            if command != 0 {
                let info = CMINVOKECOMMANDINFO {
                    cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
                    hwnd: owner,
                    lpVerb: windows::core::PCSTR((command - 1) as usize as *const u8),
                    nShow: SW_SHOWNORMAL.0,
                    ..Default::default()
                };
                context.InvokeCommand(&info)?;
            }
            Ok(())
        })();
        let _ = DestroyMenu(menu);
        let _ = RemoveWindowSubclass(owner, Some(context_proc), 0xf11e);
        result
    }
}

pub fn error(owner: HWND, error: &windows::core::Error) {
    unsafe {
        MessageBoxW(
            Some(owner),
            &HSTRING::from(format!("Could not complete the file action.\n\n{error}")),
            w!("Styled Dock"),
            MB_ICONERROR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    #[test]
    fn downloaded_file_exposes_native_drag_data_and_context_actions() -> Result<()> {
        let _ole = Ole::new()?;
        let path =
            std::env::temp_dir().join(format!("styled-dock-shell-{}.txt", std::process::id()));
        std::fs::write(&path, "file transfer regression").unwrap();
        let result = (|| unsafe {
            let item: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)?;
            let data: IDataObject = item.BindToHandler(None, &BHID_DataObject)?;
            data.QueryGetData(&FORMATETC {
                cfFormat: 15,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            })
            .ok()?;
            let context: IContextMenu = item.BindToHandler(None, &BHID_SFUIObject)?;
            let menu = CreatePopupMenu()?;
            let populated = context
                .QueryContextMenu(menu, 0, 1, 0x6fff, CMF_NORMAL)
                .ok();
            let count = GetMenuItemCount(Some(menu));
            let _ = DestroyMenu(menu);
            populated?;
            assert!(count > 0);
            Ok(())
        })();
        let _ = std::fs::remove_file(path);
        result
    }
}
