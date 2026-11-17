use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Result, w};

pub struct Instance(HANDLE);
impl Instance {
    pub fn acquire() -> Result<Option<Self>> {
        let handle = unsafe { CreateMutexW(None, false, w!("Local\\StyledDock.Instance"))? };
        let duplicate = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let guard = Self(handle);
        if duplicate {
            unsafe {
                if let Ok(window) = FindWindowW(w!("StyledDockWindow"), None) {
                    let _ = PostMessageW(
                        Some(window),
                        crate::settings::WM_SETTINGS_OPEN,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            Ok(None)
        } else {
            Ok(Some(guard))
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub fn stop() -> Result<()> {
    unsafe {
        let windows: Vec<_> = [
            FindWindowW(w!("StyledDockWindow"), None),
            FindWindowW(None, w!("Styled Dock Settings")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut processes = std::collections::HashMap::new();
        for &window in &windows {
            let mut pid = 0;
            GetWindowThreadProcessId(window, Some(&mut pid));
            if pid != 0 && !processes.contains_key(&pid) {
                if let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
                    processes.insert(pid, process);
                }
            }
        }
        for window in windows {
            if IsWindow(Some(window)).as_bool() {
                let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        let mut timeout = false;
        for process in processes.into_values() {
            timeout |= WaitForSingleObject(process, 3000) != WAIT_OBJECT_0;
            let _ = CloseHandle(process);
        }
        if timeout {
            return Err(windows::core::Error::new(
                E_FAIL,
                "Styled Dock did not exit within three seconds.",
            ));
        }
    }
    Ok(())
}
