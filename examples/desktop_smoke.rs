//! End-to-end desktop checks using an isolated config and only test-owned dock
//! processes. Run on an idle Windows desktop with the installed dock stopped.
//! Does not restart Explorer, suspend Windows, or change display configuration.
use std::{
    fs,
    mem::size_of,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::{
        Foundation::{FILETIME, HANDLE, HWND, LPARAM, RECT, WPARAM},
        Graphics::Gdi::*,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::GetProcessTimes,
        },
        UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn wait(label: &str, mut ready: impl FnMut() -> bool) -> Result<()> {
    let started = Instant::now();
    while !ready() {
        if started.elapsed() > Duration::from_secs(5) {
            return Err(format!("Timed out: {label}").into());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

fn find(class: PCWSTR, pid: u32) -> Option<HWND> {
    let hwnd = unsafe { FindWindowW(class, None) }.ok()?;
    let mut owner = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
    (owner == pid).then_some(hwnd)
}

fn send(hwnd: HWND, message: u32, param: u32) -> Result<()> {
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            message,
            WPARAM(param as usize),
            LPARAM(0),
            SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
            1000,
            None,
        )
    };
    if sent.0 == 0 {
        return Err(format!("Window stopped responding to {message:#x}").into());
    }
    Ok(())
}

fn monitor(hwnd: HWND) -> Result<MONITORINFO> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY), &mut info).ok()?;
    }
    Ok(info)
}

fn rect(hwnd: HWND) -> Result<RECT> {
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut rect)?;
    }
    Ok(rect)
}

struct ShellState {
    taskbar: HWND,
    pid: u32,
    state: usize,
    visible: bool,
    work: RECT,
}
impl ShellState {
    fn capture() -> Result<Self> {
        let taskbar = unsafe { FindWindowW(w!("Shell_TrayWnd"), None)? };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(taskbar, Some(&mut pid));
        }
        let mut shell_pid = 0;
        unsafe {
            GetWindowThreadProcessId(GetShellWindow(), Some(&mut shell_pid));
        }
        if pid == 0 || pid != shell_pid {
            return Err("Taskbar is not owned by Explorer; stop other shell bridges first".into());
        }
        let mut data = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: taskbar,
            ..Default::default()
        };
        Ok(Self {
            taskbar,
            pid,
            state: unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) },
            visible: unsafe { IsWindowVisible(taskbar).as_bool() },
            work: monitor(taskbar)?.rcWork,
        })
    }
    fn restored(&self) -> bool {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(self.taskbar, Some(&mut pid));
        }
        if pid != self.pid {
            return false;
        }
        let mut data = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: self.taskbar,
            ..Default::default()
        };
        (unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) == self.state })
            && monitor(self.taskbar).is_ok_and(|info| info.rcWork == self.work)
            && (!self.visible || unsafe { IsWindowVisible(self.taskbar).as_bool() })
    }
}
impl Drop for ShellState {
    fn drop(&mut self) {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(self.taskbar, Some(&mut pid));
        }
        if pid == self.pid {
            let mut data = APPBARDATA {
                cbSize: size_of::<APPBARDATA>() as u32,
                hWnd: self.taskbar,
                lParam: LPARAM(self.state as isize),
                ..Default::default()
            };
            unsafe {
                SHAppBarMessage(ABM_SETSTATE, &mut data);
                if self.visible {
                    let _ = ShowWindowAsync(self.taskbar, SW_SHOWNA);
                }
            }
        }
    }
}

struct Running(Child);
impl Running {
    fn command(exe: &Path, cwd: &Path, args: &[&str]) -> Result<Self> {
        Ok(Self(
            Command::new(exe)
                .args(args)
                .current_dir(cwd)
                .env("LOCALAPPDATA", cwd.join("isolated-appdata"))
                .spawn()?,
        ))
    }
    fn exited(&mut self) -> bool {
        self.0.try_wait().ok().flatten().is_some()
    }
    fn successful_exit(&mut self) -> Result<()> {
        wait("process exit", || self.exited())?;
        let status = self.0.wait()?;
        if !status.success() {
            return Err(format!("Process returned {status}").into());
        }
        Ok(())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        if !self.exited() {
            if let Some(hwnd) = find(w!("StyledDockWindow"), self.0.id()) {
                let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            }
            if wait("cleanup", || self.exited()).is_err() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
            // Allow the independent watchdog to restore the shell after a kill.
            thread::sleep(Duration::from_millis(500));
        }
    }
}

fn cpu_time(child: &Child) -> Result<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        GetProcessTimes(
            HANDLE(child.as_raw_handle()),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )?;
    }
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Ok(ticks(kernel) + ticks(user))
}

fn working_set(child: &Child) -> Result<f64> {
    let mut memory = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe {
        GetProcessMemoryInfo(
            HANDLE(child.as_raw_handle()),
            &mut memory,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )?;
    }
    Ok(memory.WorkingSetSize as f64 / 1_048_576.0)
}

fn settings(pid: u32) -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
        let (pid, windows) = unsafe { &mut *(data.0 as *mut (u32, Vec<HWND>)) };
        let mut owner = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut owner));
        }
        if owner == *pid {
            let mut class = [0; 32];
            let length = unsafe { GetClassNameW(hwnd, &mut class) };
            if String::from_utf16_lossy(&class[..length as usize]) == "#32770" {
                windows.push(hwnd);
            }
        }
        true.into()
    }
    let mut state = (pid, Vec::new());
    let _ = unsafe { EnumWindows(Some(collect), LPARAM((&raw mut state) as isize)) };
    state.1
}

struct ForegroundWindow {
    hwnd: HWND,
    previous: HWND,
}
impl ForegroundWindow {
    fn new(bounds: RECT, style: WINDOW_EX_STYLE, title: PCWSTR) -> Result<Self> {
        let previous = unsafe { GetForegroundWindow() };
        let hwnd = unsafe {
            CreateWindowExW(
                style,
                w!("STATIC"),
                title,
                WS_POPUP,
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                None,
                None,
                None,
                None,
            )?
        };
        let window = Self { hwnd, previous };
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        wait("test window takes foreground", || unsafe {
            GetForegroundWindow() == hwnd
        })?;
        Ok(window)
    }
}
impl Drop for ForegroundWindow {
    fn drop(&mut self) {
        let restore = unsafe { GetForegroundWindow() == self.hwnd };
        let _ = unsafe { DestroyWindow(self.hwnd) };
        if restore && unsafe { IsWindow(Some(self.previous)).as_bool() } {
            let _ = unsafe { SetForegroundWindow(self.previous) };
        }
    }
}

fn check_placement(dock: HWND, pid: u32, mode: &str, topbar: bool) -> Result<()> {
    let info = monitor(dock)?;
    let bounds = rect(dock)?;
    if bounds.left < info.rcMonitor.left
        || bounds.right > info.rcMonitor.right
        || bounds.bottom > info.rcMonitor.bottom
    {
        return Err("Dock left the selected display".into());
    }
    if topbar {
        let bar = find(w!("StyledDockTopBar"), pid).ok_or("Missing status bar")?;
        if info.rcWork.top < rect(bar)?.bottom {
            return Err("Status bar reservation was lost".into());
        }
    }
    if mode == "reserve" && info.rcWork.bottom >= bounds.bottom {
        return Err("Dock reservation was lost".into());
    }
    if mode == "auto-hide" && bounds.bottom != info.rcMonitor.bottom {
        return Err("Reveal strip left the display edge".into());
    }
    Ok(())
}

fn scenario(
    exe: &Path,
    root: &Path,
    mode: &str,
    crash: bool,
    hide_taskbar: bool,
    topbar: bool,
    fullscreen: bool,
) -> Result<()> {
    let name = format!(
        "{mode}-{}-topbar-{topbar}",
        if crash { "crash" } else { "quit" }
    );
    let cwd = root.join(&name);
    fs::create_dir_all(&cwd)?;
    fs::write(
        cwd.join("config.toml"),
        format!(
            "pinned = []\n[general]\nsetup_completed = true\nlanguage = 'en'\n[placement]\nreserve_space = {}\nauto_hide = {}\nhide_on_fullscreen = true\nhide_taskbar = {hide_taskbar}\n[top_bar]\nenabled = {topbar}\nshow_tray = true\n",
            mode == "reserve",
            mode == "auto-hide"
        ),
    )?;
    let shell = ShellState::capture()?;
    let mut app = Running::command(exe, &cwd, &[])?;
    let pid = app.0.id();
    wait("dock startup", || {
        find(w!("StyledDockWindow"), pid)
            .is_some_and(|hwnd| unsafe { IsWindowVisible(hwnd).as_bool() })
    })?;
    let dock = find(w!("StyledDockWindow"), pid).unwrap();
    thread::sleep(Duration::from_millis(500));
    check_placement(dock, pid, mode, topbar)?;
    if hide_taskbar {
        wait("taskbar hiding", || !unsafe {
            IsWindowVisible(shell.taskbar).as_bool()
        })?;
    }
    // Exercise recovery handlers without restarting the user's Explorer or sleep.
    for event in [WM_DISPLAYCHANGE, WM_SETTINGCHANGE] {
        send(dock, event, 0)?;
    }
    send(dock, WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC)?;
    send(
        dock,
        unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
        0,
    )?;
    thread::sleep(Duration::from_millis(500));
    check_placement(dock, pid, mode, topbar)?;
    let mut duplicate = Running::command(exe, &cwd, &[])?;
    duplicate.successful_exit()?;
    if find(w!("StyledDockWindow"), pid) != Some(dock) {
        return Err("Duplicate launch replaced the original instance".into());
    }
    wait("duplicate launch opens settings", || {
        !settings(pid).is_empty()
    })?;
    for dialog in settings(pid) {
        unsafe {
            PostMessageW(Some(dialog), WM_CLOSE, WPARAM(0), LPARAM(0))?;
        }
    }
    wait("settings close", || settings(pid).is_empty())?;
    if fullscreen {
        let screen = monitor(dock)?.rcMonitor;
        let window = ForegroundWindow::new(
            screen,
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("Styled Dock fullscreen smoke test"),
        )?;
        let bar = if topbar {
            Some(find(w!("StyledDockTopBar"), pid).ok_or("Status bar is missing")?)
        } else {
            None
        };
        let bars_hidden = || unsafe {
            !IsWindowVisible(dock).as_bool()
                && bar.is_none_or(|bar| !IsWindowVisible(bar).as_bool())
        };
        let bars_visible = || unsafe {
            IsWindowVisible(dock).as_bool() && bar.is_none_or(|bar| IsWindowVisible(bar).as_bool())
        };
        wait("fullscreen hides dock and status bar", bars_hidden)?;
        let popup = RECT {
            left: screen.right - 340,
            top: screen.bottom - 170,
            right: screen.right - 20,
            bottom: screen.bottom - 50,
        };
        let notification = ForegroundWindow::new(
            popup,
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("Styled Dock notification overlay smoke test"),
        )?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(600) {
            if !bars_hidden() {
                return Err(
                    "Dock or status bar appeared during a fullscreen notification overlay".into(),
                );
            }
            if unsafe { GetForegroundWindow() } != notification.hwnd {
                return Err(
                    "Foreground changed unexpectedly during the notification overlay".into(),
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
        drop(notification);
        wait(
            "fullscreen regains foreground after notification",
            || unsafe { GetForegroundWindow() == window.hwnd },
        )?;
        let app_window = ForegroundWindow::new(
            popup,
            WS_EX_APPWINDOW,
            w!("Styled Dock ordinary app smoke test"),
        )?;
        wait("switching apps restores dock and status bar", bars_visible)?;
        if unsafe { GetForegroundWindow() } != app_window.hwnd {
            return Err("Restoring the dock or status bar changed the foreground window".into());
        }
        drop(app_window);
        wait("switching back hides dock and status bar", bars_hidden)?;
        unsafe {
            SetWindowPos(
                window.hwnd,
                None,
                screen.left + 100,
                screen.top + 100,
                320,
                240,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )?;
        }
        wait("fullscreen exit restores dock and status bar", bars_visible)?;
        drop(window);
        check_placement(dock, pid, mode, topbar)?;
    }
    thread::sleep(Duration::from_millis(500));
    let before = cpu_time(&app.0)?;
    let idle = Instant::now();
    thread::sleep(Duration::from_secs(5));
    let cpu = (cpu_time(&app.0)? - before) as f64 / 10_000_000.0;
    let idle_elapsed = idle.elapsed().as_secs_f64();
    let memory = working_set(&app.0)?;
    let dpi = unsafe { GetDpiForWindow(dock) };
    // Exercise quit/crash while Settings is open as well, after measuring the
    // dock alone so the sample does not include the Settings window's paint.
    Running::command(exe, &cwd, &[])?.successful_exit()?;
    wait("settings reopens before shutdown", || {
        !settings(pid).is_empty()
    })?;
    if crash {
        app.0.kill()?;
        app.0.wait()?;
    } else {
        Running::command(exe, &cwd, &["--quit"])?.successful_exit()?;
        app.successful_exit()?;
    }
    wait("shell state restoration", || shell.restored())?;
    // The recovery command must also be safe after the dock is already gone.
    Running::command(exe, &cwd, &["--restore-taskbar"])?.successful_exit()?;
    wait("idempotent taskbar recovery", || shell.restored())?;
    println!(
        "PASS {name}: dpi={dpi}, fullscreen={fullscreen}, idle CPU={:.2}% of one core (5s sample), working set={memory:.1} MiB, shell restored",
        100.0 * cpu / idle_elapsed
    );
    Ok(())
}

fn main() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    if unsafe { FindWindowW(w!("StyledDockWindow"), None).is_ok() } {
        return Err("Stop the running dock before this diagnostic".into());
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    let exe = args
        .windows(2)
        .find(|pair| pair[0] == "--exe")
        .map(|pair| PathBuf::from(&pair[1]))
        .ok_or("Usage: desktop_smoke --exe <styled-dock.exe> [--taskbar] [--fullscreen]")?
        .canonicalize()?;
    let hide = args.iter().any(|arg| arg == "--taskbar");
    let fullscreen = args.iter().any(|arg| arg == "--fullscreen");
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let root = std::env::current_dir()?
        .join(".test-tmp")
        .join(format!("desktop-smoke-{nonce}"));
    fs::create_dir_all(&root)?;
    println!(
        "Desktop smoke: displays={}, taskbar hiding={hide}, configs={}",
        unsafe { GetSystemMetrics(SM_CMONITORS) },
        root.display()
    );
    for mode in ["floating", "reserve", "auto-hide"] {
        for crash in [false, true] {
            scenario(&exe, &root, mode, crash, hide, true, fullscreen)?;
        }
    }
    scenario(&exe, &root, "floating", false, false, false, fullscreen)?;
    println!(
        "PASS all 7 scenarios. Real sleep, Explorer restart, display reconnect, and mouse gestures remain manual checks."
    );
    Ok(())
}
