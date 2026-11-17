use windows::Win32::Foundation::{CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
    SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Power::{
    GetPwrCapabilities, SYSTEM_POWER_CAPABILITIES, SetSuspendState,
};
use windows::Win32::System::Shutdown::*;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::core::{Error, Result};

#[derive(Clone, Copy, Debug)]
pub enum SystemAction {
    About,
    Notifications,
    Start,
    Search,
    TaskView,
    ShowDesktop,
    InputLanguage,
    SoundOutput,
    ToggleWifi,
    ToggleBluetooth,
    TaskManager,
    Store,
    Sleep,
    Restart,
    Shutdown,
    Lock,
    LogOut,
}

impl SystemAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::About => "About this PC",
            Self::Notifications => "Notification center",
            Self::Start => "Start menu",
            Self::Search => "Windows Search",
            Self::TaskView => "Task View",
            Self::ShowDesktop => "Show desktop",
            Self::InputLanguage => "Switch input language",
            Self::SoundOutput => "Sound output",
            Self::ToggleWifi => "Change Wi-Fi",
            Self::ToggleBluetooth => "Change Bluetooth",
            Self::TaskManager => "Task Manager",
            Self::Store => "Microsoft Store",
            Self::Sleep => "Sleep",
            Self::Restart => "Restart",
            Self::Shutdown => "Shut down",
            Self::Lock => "Lock screen",
            Self::LogOut => "Log out",
        }
    }

    pub fn execute(self) -> Result<()> {
        match self {
            Self::About => crate::shell::launch("ms-settings:about"),
            Self::Notifications
            | Self::Start
            | Self::Search
            | Self::TaskView
            | Self::ShowDesktop
            | Self::InputLanguage
            | Self::SoundOutput => {
                let keys = match self {
                    Self::Start => vec![(VK_LWIN, false), (VK_LWIN, true)],
                    Self::Search => shortcut_keys(VK_S),
                    Self::TaskView => shortcut_keys(VK_TAB),
                    Self::ShowDesktop => shortcut_keys(VK_D),
                    Self::InputLanguage => shortcut_keys(VK_SPACE),
                    Self::SoundOutput => vec![
                        (VK_LWIN, false),
                        (VK_LCONTROL, false),
                        (VK_V, false),
                        (VK_V, true),
                        (VK_LCONTROL, true),
                        (VK_LWIN, true),
                    ],
                    _ => shortcut_keys(VK_N),
                };
                let inputs = keyboard_inputs(&keys);
                if unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) } != inputs.len() as u32 {
                    // Release every injected key after a partially accepted shortcut.
                    let releases = keyboard_inputs(
                        &keys.iter().map(|&(key, _)| (key, true)).collect::<Vec<_>>(),
                    );
                    let _ = unsafe { SendInput(&releases, size_of::<INPUT>() as i32) };
                    return Err(Error::new(
                        windows::Win32::Foundation::E_FAIL,
                        format!("Could not open {}", self.label()),
                    ));
                }
                Ok(())
            }
            Self::ToggleWifi => crate::panels::toggle_radio(false),
            Self::ToggleBluetooth => crate::panels::toggle_radio(true),
            Self::Store => crate::shell::launch("ms-windows-store:"),
            Self::TaskManager => {
                let mut path = [0; 32768];
                let length = unsafe { GetSystemDirectoryW(Some(&mut path)) } as usize;
                if length == 0 || length >= path.len() {
                    return Err(Error::from_thread());
                }
                crate::shell::launch(&format!(
                    "{}\\Taskmgr.exe",
                    String::from_utf16_lossy(&path[..length]),
                ))
            }
            Self::Lock => unsafe { LockWorkStation() },
            Self::LogOut => unsafe { ExitWindowsEx(EWX_LOGOFF, shutdown_reason()) },
            Self::Sleep | Self::Restart | Self::Shutdown => {
                let _privilege = ShutdownPrivilege::enable()?;
                unsafe {
                    match self {
                        Self::Sleep => {
                            if SetSuspendState(false, false, false) {
                                Ok(())
                            } else {
                                Err(Error::from_thread())
                            }
                        }
                        Self::Restart => ExitWindowsEx(EWX_REBOOT, shutdown_reason()),
                        Self::Shutdown => ExitWindowsEx(EWX_POWEROFF, shutdown_reason()),
                        _ => unreachable!(),
                    }
                }
            }
        }
    }
}

fn shortcut_keys(key: VIRTUAL_KEY) -> Vec<(VIRTUAL_KEY, bool)> {
    vec![(VK_LWIN, false), (key, false), (key, true), (VK_LWIN, true)]
}

fn keyboard_inputs(keys: &[(VIRTUAL_KEY, bool)]) -> Vec<INPUT> {
    keys.iter()
        .map(|&(key, released)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    dwFlags: if released {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    ..Default::default()
                },
            },
        })
        .collect()
}

fn shutdown_reason() -> SHUTDOWN_REASON {
    SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_MINOR_OTHER | SHTDN_REASON_FLAG_PLANNED
}

pub fn sleep_available() -> bool {
    let mut capabilities = SYSTEM_POWER_CAPABILITIES::default();
    let available = unsafe { GetPwrCapabilities(&mut capabilities) };
    available
        && (capabilities.SystemS1
            || capabilities.SystemS2
            || capabilities.SystemS3
            || capabilities.AoAc)
}

struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

// Enable the privilege only around a power request and restore its previous state.
struct ShutdownPrivilege {
    token: Token,
    previous: TOKEN_PRIVILEGES,
}

impl ShutdownPrivilege {
    fn enable() -> Result<Self> {
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )?;
            let token = Token(token);
            let mut luid = LUID::default();
            LookupPrivilegeValueW(None, SE_SHUTDOWN_NAME, &mut luid)?;
            let requested = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let mut guard = Self {
                token,
                previous: TOKEN_PRIVILEGES::default(),
            };
            let mut size = 0;
            AdjustTokenPrivileges(
                guard.token.0,
                false,
                Some(&requested),
                size_of::<TOKEN_PRIVILEGES>() as u32,
                Some(&mut guard.previous),
                Some(&mut size),
            )?;
            if GetLastError() == ERROR_NOT_ALL_ASSIGNED {
                return Err(Error::from_thread());
            }
            Ok(guard)
        }
    }
}

impl Drop for ShutdownPrivilege {
    fn drop(&mut self) {
        let _ = unsafe {
            AdjustTokenPrivileges(self.token.0, false, Some(&self.previous), 0, None, None)
        };
    }
}

// Bind termination to the captured window and process, not whichever app gains focus later.
pub fn end_task(hwnd: windows::Win32::Foundation::HWND, pid: u32) -> Result<()> {
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let process = Token(OpenProcess(PROCESS_TERMINATE, false, pid)?);
        let mut current_pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut current_pid));
        if current_pid != pid || pid == std::process::id() {
            return Ok(());
        }
        TerminateProcess(process.0, 1)
    }
}
