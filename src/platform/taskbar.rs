use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::{
    ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Error, HSTRING, Result, w};

const LEASE_NAME: windows::core::PCWSTR = w!("Local\\StyledDock.TaskbarLease");
const WATCH_INTERVAL_MS: u32 = 200;

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub struct TaskbarLease {
    _lease: Handle,
    refresh: Handle,
    stop: Handle,
    worker: Child,
}

impl TaskbarLease {
    pub fn for_monitor(monitor: &str) -> Result<Self> {
        let monitor = crate::placement::resolve_monitor_name(monitor)?;
        let lease = Handle(unsafe { CreateMutexW(None, false, LEASE_NAME)? });
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Err(Error::new(
                E_FAIL,
                "Another Styled Dock instance is managing the taskbar",
            ));
        }
        let pid = std::process::id();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let ready = create_event(pid, nonce, "Ready")?;
        let refresh = create_event(pid, nonce, "Refresh")?;
        let stop = create_event(pid, nonce, "Stop")?;
        let worker = Command::new(std::env::current_exe().map_err(io_error)?)
            .args(["--taskbar-watch", &pid.to_string(), &nonce.to_string()])
            .arg(&monitor)
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .map_err(io_error)?;
        let guard = Self {
            _lease: lease,
            refresh,
            stop,
            worker,
        };
        let worker = HANDLE(guard.worker.as_raw_handle());
        if unsafe { WaitForMultipleObjects(&[ready.0, worker], false, 3000) } != WAIT_OBJECT_0 {
            return Err(Error::new(E_FAIL, "The taskbar watchdog did not start"));
        }
        Ok(guard)
    }

    pub fn refresh(&self) {
        let _ = unsafe { SetEvent(self.refresh.0) };
    }
}

impl Drop for TaskbarLease {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.stop.0);
            let _ = WaitForSingleObject(HANDLE(self.worker.as_raw_handle()), 1500);
        }
        let _ = self.worker.try_wait();
    }
}

fn event_name(pid: u32, nonce: u128, kind: &str) -> HSTRING {
    HSTRING::from(format!("Local\\StyledDock.Taskbar.{pid}.{nonce}.{kind}"))
}

fn create_event(pid: u32, nonce: u128, kind: &str) -> Result<Handle> {
    unsafe { CreateEventW(None, kind == "Stop", false, &event_name(pid, nonce, kind)).map(Handle) }
}

pub fn watch(mut args: impl Iterator<Item = String>) -> Result<()> {
    let pid = args
        .next()
        .and_then(|arg| arg.parse().ok())
        .ok_or_else(Error::empty)?;
    let nonce = args
        .next()
        .and_then(|arg| arg.parse().ok())
        .ok_or_else(Error::empty)?;
    let monitor = args.next().unwrap_or_else(|| "primary".into());
    unsafe {
        let _lease = Handle(OpenMutexW(MUTEX_ALL_ACCESS, false, LEASE_NAME)?);
        let dock = Handle(OpenProcess(PROCESS_SYNCHRONIZE, false, pid)?);
        let open =
            |kind| OpenEventW(EVENT_ALL_ACCESS, false, &event_name(pid, nonce, kind)).map(Handle);
        let (ready, refresh, stop) = (open("Ready")?, open("Refresh")?, open("Stop")?);
        let mut hidden = HiddenTaskbars::default();
        hidden.monitor = Some(monitor);
        let handles = [dock.0, stop.0, refresh.0];
        if WaitForMultipleObjects(&handles[..2], false, 0) != WAIT_TIMEOUT {
            return Ok(());
        }
        hidden.hide_visible();
        SetEvent(ready.0)?;
        maintain_hidden_taskbars(&handles, WATCH_INTERVAL_MS, || hidden.hide_visible())
    }
}

fn maintain_hidden_taskbars(
    handles: &[HANDLE; 3],
    interval_ms: u32,
    mut hide: impl FnMut(),
) -> Result<()> {
    loop {
        let reason = unsafe { WaitForMultipleObjects(handles, false, interval_ms) };
        if reason == WAIT_FAILED {
            return Err(Error::from_thread());
        }
        if reason != WAIT_TIMEOUT && reason.0 != WAIT_OBJECT_0.0 + 2 {
            return Ok(());
        }
        // Explorer can show its taskbar again after fullscreen without a
        // TaskbarCreated or work-area notification. Keep the lease enforced
        // even while the dock itself is hidden or busy in a menu.
        hide();
    }
}

#[derive(Default)]
struct HiddenTaskbars {
    windows: Vec<(HWND, u32)>,
    original_state: Option<usize>,
    monitor: Option<String>,
}

impl HiddenTaskbars {
    fn hide_visible(&mut self) {
        // Hiding the window alone leaves Explorer's AppBar reservation in
        // rcWork. Let Explorer release it through its auto-hide setting while
        // the watchdog continues suppressing the actual taskbar windows.
        if let Some(hwnd) = primary_taskbar() {
            let state = taskbar_state(hwnd);
            self.original_state.get_or_insert(state);
            if state & ABS_AUTOHIDE as usize == 0 {
                set_taskbar_state(hwnd, state | ABS_AUTOHIDE as usize);
            }
        }
        self.windows
            .retain(|&(hwnd, pid)| taskbar_pid(hwnd) == Some(pid));
        for (hwnd, pid) in taskbars() {
            if self.monitor.as_ref().is_some_and(|name| unsafe {
                windows::Win32::Graphics::Gdi::MonitorFromWindow(hwnd, windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST)
            } != crate::placement::select_monitor(name)) { continue; }
            if unsafe {
                IsWindowVisible(hwnd).as_bool() && ShowWindowAsync(hwnd, SW_HIDE).as_bool()
            } && !self.windows.contains(&(hwnd, pid))
            {
                self.windows.push((hwnd, pid));
            }
        }
    }
}

impl Drop for HiddenTaskbars {
    fn drop(&mut self) {
        // This guard lives in the independent watchdog, so both a normal exit
        // and a dock crash restore the user's original auto-hide preference.
        if let (Some(state), Some(hwnd)) = (self.original_state, primary_taskbar()) {
            set_taskbar_state(hwnd, state);
        }
        for &(hwnd, pid) in &self.windows {
            if taskbar_pid(hwnd) == Some(pid) {
                let _ = unsafe { ShowWindowAsync(hwnd, SW_SHOWNA) };
            }
        }
    }
}

fn primary_taskbar() -> Option<HWND> {
    taskbars().into_iter().find_map(|(hwnd, _)| {
        let mut class = [0u16; 32];
        let len = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
        (String::from_utf16_lossy(&class[..len]) == "Shell_TrayWnd").then_some(hwnd)
    })
}

fn taskbar_state(hwnd: HWND) -> usize {
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut taskbar_data(hwnd)) }
}

fn set_taskbar_state(hwnd: HWND, state: usize) {
    let mut data = taskbar_data(hwnd);
    data.lParam = LPARAM(state as isize);
    unsafe { SHAppBarMessage(ABM_SETSTATE, &mut data) };
}

fn taskbar_data(hwnd: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        ..Default::default()
    }
}

pub fn restore() -> Result<()> {
    for (hwnd, _) in taskbars() {
        let _ = unsafe { ShowWindowAsync(hwnd, SW_SHOWNA) };
    }
    Ok(())
}

fn taskbars() -> Vec<(HWND, u32)> {
    let mut bars: Vec<(HWND, u32)> = Vec::new();
    let _ = unsafe { EnumWindows(Some(collect_taskbars), LPARAM((&raw mut bars) as isize)) };
    bars
}

unsafe extern "system" fn collect_taskbars(hwnd: HWND, data: LPARAM) -> BOOL {
    if let Some(pid) = taskbar_pid(hwnd) {
        unsafe { (&mut *(data.0 as *mut Vec<(HWND, u32)>)).push((hwnd, pid)) };
    }
    BOOL(1)
}

fn taskbar_pid(hwnd: HWND) -> Option<u32> {
    let mut class = [0u16; 32];
    let length = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
    if !matches!(
        String::from_utf16_lossy(&class[..length]).as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    ) {
        return None;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    // Scope visibility changes to the actual Explorer shell process.
    let mut shell_pid = 0;
    unsafe { GetWindowThreadProcessId(GetShellWindow(), Some(&mut shell_pid)) };
    (pid != 0 && pid == shell_pid).then_some(pid)
}

fn io_error(error: std::io::Error) -> Error {
    Error::new(E_FAIL, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromWindow};

    #[test]
    fn hiding_taskbar_releases_work_area_and_restores_shell_state() -> Result<()> {
        let hwnd = primary_taskbar().expect("Explorer taskbar must be running");
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY) };
        let state = || taskbar_state(hwnd);
        let original = state();
        let before = crate::placement::monitor_info(monitor)?.monitorInfo;
        {
            let mut hidden = HiddenTaskbars::default();
            hidden.hide_visible();
            assert_ne!(state() & ABS_AUTOHIDE as usize, 0);
            let work = crate::placement::monitor_info(monitor)?.monitorInfo.rcWork;
            assert!(
                work.bottom >= before.rcMonitor.bottom - 2,
                "reserved taskbar band remains: {work:?}"
            );
            assert_eq!(
                work.top, before.rcWork.top,
                "topbar reservation must survive"
            );
            // A user who already enabled auto-hide must keep that preference.
            {
                let mut already_auto_hidden = HiddenTaskbars::default();
                already_auto_hidden.hide_visible();
            }
            assert_ne!(state() & ABS_AUTOHIDE as usize, 0);
            // Explorer can lose the state while the lease is active. Repair it
            // without replacing the original preference saved on first hide.
            set_taskbar_state(hwnd, original & !(ABS_AUTOHIDE as usize));
            hidden.hide_visible();
            assert_ne!(state() & ABS_AUTOHIDE as usize, 0);
        }
        assert_eq!(
            state(),
            original,
            "restore the setting from before the first hide"
        );
        Ok(())
    }

    #[test]
    fn watchdog_rehides_without_dock_notifications_and_stops_on_exit() -> Result<()> {
        let dock = Handle(unsafe { CreateEventW(None, true, false, None)? });
        let stop = Handle(unsafe { CreateEventW(None, true, false, None)? });
        let refresh = Handle(unsafe { CreateEventW(None, false, true, None)? });
        let handles = [dock.0, stop.0, refresh.0];
        let mut calls = 0;
        maintain_hidden_taskbars(&handles, 10, || {
            calls += 1;
            if calls == 3 {
                unsafe { SetEvent(stop.0).unwrap() };
            }
        })?;
        // One explicit refresh followed by two timeouts, without any more
        // messages from the dock. Stop must win before another hide attempt.
        assert_eq!(calls, 3);
        unsafe {
            ResetEvent(stop.0)?;
            SetEvent(dock.0)?;
            SetEvent(refresh.0)?;
        }
        maintain_hidden_taskbars(&handles, 10, || panic!("parent exit must stop the worker"))?;
        Ok(())
    }
}
