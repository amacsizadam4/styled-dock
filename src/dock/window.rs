use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::mem::{size_of, size_of_val};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    E_FAIL, HWND, LPARAM, LRESULT, POINT, RECT, WAIT_FAILED, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CombineRgn, CreateRectRgnIndirect, CreateRoundRectRgn,
    DeleteObject, EndPaint, InvalidateRect, PAINTSTRUCT, RGN_AND, RGN_ERROR, RGN_OR, SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::MK_LBUTTON;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{ABN_FULLSCREENAPP, ABN_POSCHANGED};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, HSTRING, Result, w};

use crate::animation::{Activation, Animation};
use crate::config::{self, Pin, Source};
use crate::layout::{Bounds, DockSize, Layout, Scene};
use crate::placement::{
    Placement, WM_APPBAR, WM_FULLSCREEN_CHECK, WM_REPOSITION, WM_SHELL_RESTART,
};
use crate::popup::Popup;
use crate::render::Renderer;
use crate::settings::{self, SettingsWindow, WM_SETTINGS_OPEN, WM_SETTINGS_SAVED};
use crate::shell::{self, Event, Request, WM_SHELL_RESULT};
use crate::taskbar::TaskbarLease;
use crate::topbar::{Action, TopBar};
use crate::watcher::{self, WM_APPS_CHANGED, Watcher};

const WM_RENDER_DOCK: u32 = WM_APP + 1;
const HIDE_TIMER: usize = 1;
const LABEL_TIMER: usize = 2;
const RECOVERY_TIMER: usize = 3;
const RECOVERY_INTERVAL_MS: u32 = 200;

struct DockHotkeys {
    hwnd: HWND,
    registered: Vec<i32>,
}
impl DockHotkeys {
    fn new(hwnd: HWND) -> Self {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        let registered = [(1, b'D'), (2, b'S')]
            .into_iter()
            .filter_map(|(id, key)| unsafe {
                RegisterHotKey(
                    Some(hwnd),
                    id,
                    MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                    key as u32,
                )
                .is_ok()
                .then_some(id)
            })
            .collect();
        Self { hwnd, registered }
    }
}
impl Drop for DockHotkeys {
    fn drop(&mut self) {
        for &id in &self.registered {
            let _ = unsafe {
                windows::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey(Some(self.hwnd), id)
            };
        }
    }
}

#[derive(Default)]
struct HoverLabel {
    pending: Option<(usize, Instant)>,
}

impl HoverLabel {
    fn reset(&mut self, hwnd: HWND) {
        self.pending = None;
        let _ = unsafe { KillTimer(Some(hwnd), LABEL_TIMER) };
    }

    fn visible(&mut self, hwnd: HWND, index: Option<usize>, delay_ms: u32) -> Option<usize> {
        let (visible, remaining) = self.update(index, delay_ms, Instant::now());
        if let Some(remaining) = remaining {
            // Wake once when the delay expires; no continuous rendering while waiting.
            unsafe { SetTimer(Some(hwnd), LABEL_TIMER, remaining, None) };
        } else {
            let _ = unsafe { KillTimer(Some(hwnd), LABEL_TIMER) };
        }
        visible
    }

    fn update(
        &mut self,
        index: Option<usize>,
        delay_ms: u32,
        now: Instant,
    ) -> (Option<usize>, Option<u32>) {
        let Some(index) = index else {
            self.pending = None;
            return (None, None);
        };
        if self.pending.is_none_or(|(previous, _)| previous != index) {
            self.pending = Some((index, now));
        }
        let elapsed = now.duration_since(self.pending.unwrap().1);
        let delay = Duration::from_millis(u64::from(delay_ms));
        if elapsed >= delay {
            (Some(index), None)
        } else {
            (None, Some((delay - elapsed).as_millis() as u32 + 1))
        }
    }
}
thread_local! {
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

pub(crate) struct DockItem {
    pub pin: Pin,
    pub windows: Vec<HWND>,
    pub(crate) pinned: bool,
}

impl DockItem {
    fn movable(&self) -> bool {
        !self.utility() || dock_system_action(&self.pin.target).is_some()
    }
    pub(crate) fn utility(&self) -> bool {
        matches!(
            self.pin.target.as_str(),
            "dock:Start" | "dock:Search" | "shell:Downloads" | "shell:RecycleBinFolder"
        )
    }
}
struct Press {
    index: usize,
    origin: POINT,
    dragging: bool,
}

impl Press {
    fn plan(&self, layout: Layout, items: &[DockItem], point: POINT) -> Option<(usize, bool, f32)> {
        // Use stable slots so moving neighbours cannot move the drop target.
        let slots = Scene {
            panel: Bounds {
                x: (layout.width - layout.base_width()) / 2.0,
                y: layout.panel_top(),
                width: layout.base_width(),
                height: layout.panel_height(),
            },
            icons: (0..items.len())
                .map(|index| {
                    let (x, y, size) = layout.icon(index);
                    Bounds {
                        x,
                        y,
                        width: size,
                        height: size,
                    }
                })
                .collect(),
        };
        let (slot, marker) = slots.insertion(point, layout.scale)?;
        let to = slot - usize::from(slot > self.index);
        if dock_system_action(&items[self.index].pin.target).is_some() {
            return Some((to, false, marker));
        }
        let pinned = items.iter().filter(|item| item.pinned).count();
        let apps = items.iter().filter(|item| !item.utility()).count();
        let logical_slot = items[..slot]
            .iter()
            .filter(|item| dock_system_action(&item.pin.target).is_none())
            .count();
        if logical_slot > apps || (items[self.index].pinned && logical_slot > pinned) {
            return None;
        }
        let boundary = if pinned == 0 {
            let first_app = items.iter().position(|item| !item.utility()).unwrap_or(0);
            let (x, _, size) = layout.icon(first_app);
            x + size / 2.0
        } else {
            let first_unpinned = items
                .iter()
                .position(|item| !item.utility() && !item.pinned)
                .unwrap_or_else(|| items.iter().rposition(|item| item.pinned).unwrap_or(0) + 1);
            layout.icon(first_unpinned).0 - layout.gap() / 2.0
        };
        let pin = items[self.index].pinned || (point.x as f32 / layout.scale) < boundary;
        if pin && !items[self.index].pinned && pinned >= config::MAX_PINS {
            return None;
        }
        Some((to, pin, marker))
    }

    fn moved(&self, point: POINT, scale: f32) -> bool {
        let threshold = (5.0 * scale).round() as i32;
        (point.x - self.origin.x).abs() >= threshold || (point.y - self.origin.y).abs() >= threshold
    }

    #[cfg(test)]
    fn drop_target(&self, scene: &Scene, point: POINT, scale: f32) -> Option<(usize, f32)> {
        let (slot, marker) = scene.insertion(point, scale)?;
        let destination = slot - usize::from(slot > self.index);
        (destination != self.index).then_some((destination, marker))
    }
}

fn dock_system_action(target: &str) -> Option<crate::system::SystemAction> {
    match target {
        "dock:Start" => Some(crate::system::SystemAction::Start),
        "dock:Search" => Some(crate::system::SystemAction::Search),
        _ => None,
    }
}

fn dock_items(
    pins: &[Pin],
    shortcuts: &HashMap<String, String>,
    apps: Vec<watcher::App>,
    previous: &[DockItem],
    dock: config::Dock,
) -> Vec<DockItem> {
    let mut items: Vec<_> = pins
        .iter()
        .cloned()
        .map(|pin| DockItem {
            pin,
            windows: Vec::new(),
            pinned: true,
        })
        .collect();
    for app in apps {
        let index = pins
            .iter()
            .position(|pin| watcher::matches(&pin.target, &app))
            .or_else(|| {
                pins.iter().position(|pin| {
                    shortcuts
                        .get(&pin.target)
                        .is_some_and(|target| watcher::matches(target, &app))
                })
            });
        if let Some(index) = index {
            items[index].windows.extend(app.windows);
        } else {
            items.push(DockItem {
                pin: Pin {
                    name: app.name,
                    target: app.target,
                },
                windows: app.windows,
                pinned: false,
            });
        }
    }
    // Keep the user's order through focus changes and append newly opened apps.
    items.sort_by_key(|item| {
        (
            !item.pinned,
            previous
                .iter()
                .position(|old| old.pin.target == item.pin.target)
                .unwrap_or(usize::MAX),
        )
    });
    for item in items.iter_mut().filter(|item| !item.pinned) {
        if let Some(old) = previous
            .iter()
            .find(|old| old.pin.target == item.pin.target)
        {
            item.pin.name.clone_from(&old.pin.name);
        }
    }
    for (name, target, enabled) in [
        ("Downloads", "shell:Downloads", dock.show_downloads),
        ("Recycle Bin", "shell:RecycleBinFolder", true),
    ] {
        if !enabled {
            continue;
        }
        items.push(DockItem {
            pin: Pin {
                name: name.into(),
                target: target.into(),
            },
            windows: Vec::new(),
            pinned: false,
        });
    }
    let mut shortcuts = vec![
        (
            "Start menu",
            "dock:Start",
            dock.show_start,
            dock.start_position,
            usize::from(dock.search_first),
        ),
        (
            "Windows Search",
            "dock:Search",
            dock.show_search,
            dock.search_position,
            usize::from(!dock.search_first),
        ),
    ];
    shortcuts.sort_by_key(|item| (item.3, item.4));
    for (inserted, (name, target, _, position, _)) in
        shortcuts.into_iter().filter(|item| item.2).enumerate()
    {
        let index = position.saturating_add(inserted).min(items.len());
        items.insert(
            index,
            DockItem {
                pin: Pin {
                    name: name.into(),
                    target: target.into(),
                },
                windows: Vec::new(),
                pinned: false,
            },
        );
    }
    items
}

pub fn run() -> Result<()> {
    if run_dock()? {
        settings::restart()?;
    }
    Ok(())
}

fn run_dock() -> Result<bool> {
    let Some(_instance) = crate::lifecycle::Instance::acquire()? else {
        return Ok(false);
    };
    let _ole = crate::files::Ole::new()?;
    let config = config::load()?;
    let appearance = config.appearance;
    crate::theme::configure(appearance.theme);
    let dock = DockSize::new(appearance);
    let mut dock_shortcuts = config.dock;
    let mut pins = config.pinned;
    let mut settings = config.placement;
    settings.monitor = crate::placement::resolve_monitor_name(&settings.monitor)?;
    // The watchdog now calls Explorer's AppBar API. Start it before our tray
    // bridge takes Shell_TrayWnd lookup priority: waiting for ready while that
    // bridge belongs to this blocked UI thread would deadlock its shell calls.
    // This also releases Explorer's band before calculating the dock placement.
    let taskbar = settings
        .hide_taskbar
        .then(|| TaskbarLease::for_monitor(&settings.monitor))
        .transpose()?;
    // All window operations and the message loop run on the creating thread.
    unsafe {
        let instance = GetModuleHandleW(None)?.into();
        let class = WNDCLASSW {
            hInstance: instance,
            lpszClassName: w!("StyledDockWindow"),
            lpfnWndProc: Some(window_proc),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(Error::from_thread());
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP,
            class.lpszClassName,
            w!("Styled Dock"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance),
            None,
        )?;
        let _hotkeys = DockHotkeys::new(hwnd);
        windows::Win32::UI::Shell::DragAcceptFiles(hwnd, true);
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        if taskbar_created == 0 {
            return Err(Error::from_thread());
        }
        TASKBAR_CREATED.set(taskbar_created);
        let mut placement = Placement::new(
            hwnd,
            &settings,
            dock,
            pins.len()
                + 1
                + usize::from(dock_shortcuts.show_start)
                + usize::from(dock_shortcuts.show_search)
                + usize::from(dock_shortcuts.show_downloads),
        )?;
        let mut topbar = config
            .top_bar
            .enabled
            .then(|| {
                TopBar::new(
                    instance,
                    placement.monitor(),
                    appearance.top_bar_opacity_percent as f32 / 100.0,
                    config.top_bar.show_tray,
                )
            })
            .transpose()?;
        let mut layout = Layout::for_window(
            hwnd,
            pins.len()
                + 1
                + usize::from(dock_shortcuts.show_start)
                + usize::from(dock_shortcuts.show_search)
                + usize::from(dock_shortcuts.show_downloads),
            dock,
        )?;
        suppress_window_border(hwnd);
        let mut renderer =
            Renderer::new(hwnd, layout, appearance.dock_opacity_percent as f32 / 100.0)?;
        let mut shortcuts = HashMap::new();
        let mut items = dock_items(&pins, &shortcuts, Vec::new(), &[], dock_shortcuts);
        let mut hidden = settings.auto_hide;
        let mut animation = Animation::new(hidden, appearance);
        let (mut scene, _) = animation.frame(layout, &items, None, None, hidden, None);
        let mut shape = WindowShape::new(layout, &scene, None);
        shape.apply(hwnd)?;
        let (requests, events) = shell::start(hwnd, pins.clone())
            .map_err(|error| Error::new(E_FAIL, error.to_string()))?;
        let send = |request| {
            requests
                .send(request)
                .map_err(|_| Error::new(E_FAIL, "The shell worker stopped"))
        };
        let mut requested: HashSet<_> = items.iter().map(|item| item.pin.target.clone()).collect();
        for item in items
            .iter()
            .filter(|item| dock_system_action(&item.pin.target).is_none())
        {
            send(Request::Icon(item.pin.target.clone()))?;
        }
        let watcher = Watcher::new(hwnd, settings.hide_on_fullscreen)?;
        let mut visible = !settings.hide_on_fullscreen || !placement.fullscreen();
        if visible {
            placement.show()?;
            if let Some(bar) = &mut topbar {
                bar.set_visible(true)?;
            }
        }
        if (settings.auto_hide || settings.hide_on_fullscreen)
            && SetTimer(Some(hwnd), RECOVERY_TIMER, RECOVERY_INTERVAL_MS, None) == 0
        {
            return Err(Error::from_thread());
        }

        let mut message = MSG::default();
        let mut pressed: Option<Press> = None;
        let mut cursor: Option<POINT> = None;
        let mut label = HoverLabel::default();
        let mut frame_pending = true;
        let mut settings_window: Option<SettingsWindow> = None;
        loop {
            let status = if frame_pending && visible {
                if !PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let wait = MsgWaitForMultipleObjectsEx(
                        Some(&[renderer.frame_ready()]),
                        u32::MAX,
                        QS_ALLINPUT,
                        MWMO_INPUTAVAILABLE,
                    );
                    if wait == WAIT_FAILED {
                        return Err(Error::from_thread());
                    }
                    if wait == WAIT_OBJECT_0 {
                        layout = Layout::for_window(hwnd, items.len(), dock)?;
                        let dragging = pressed.as_ref().filter(|press| press.dragging);
                        let hover = cursor.filter(|_| dragging.is_none());
                        let drop = dragging
                            .zip(cursor)
                            .and_then(|(press, point)| press.plan(layout, &items, point));
                        let drag = dragging.zip(cursor).map(|(press, point)| {
                            (
                                press.index,
                                drop.map_or(press.index, |(to, _, _)| to),
                                point.x as f32 / layout.scale,
                            )
                        });
                        let x = hover.map(|point| point.x as f32 / layout.scale);
                        let held = pressed
                            .as_ref()
                            .filter(|press| !press.dragging)
                            .map(|press| press.index);
                        (scene, frame_pending) =
                            animation.frame(layout, &items, x, held, hidden, drag);
                        let hovered = cursor
                            .filter(|_| !hidden && pressed.is_none())
                            .and_then(|point| scene.hit(point, layout.scale));
                        let hovered = label.visible(hwnd, hovered, appearance.label_delay_ms);
                        let overlay = hovered.map(|index| scene.overlay(layout, index));
                        let updated = WindowShape::new(layout, &scene, overlay);
                        if shape != updated {
                            updated.apply(hwnd)?;
                            shape = updated;
                        }
                        let marker = drop.map(|(_, _, marker)| marker);
                        let summary = hovered.map(|index| window_summary(&items[index]));
                        renderer.paint(
                            layout,
                            &scene,
                            &items,
                            marker,
                            hovered.zip(summary.as_deref()),
                            dragging.map(|press| press.index),
                        )?;
                    }
                    continue;
                }
                i32::from(message.message != WM_QUIT)
            } else {
                GetMessageW(&mut message, None, 0, 0).0
            };
            match status {
                -1 => return Err(Error::from_thread()),
                0 => return Ok(false),
                _ => {
                    if message.message == WM_HOTKEY {
                        if message.wParam.0 == 1 {
                            let mut menu = Popup::new(hwnd, 320)?;
                            menu.heading("Dock apps · Ctrl+Alt+D")?;
                            for (index, item) in items.iter().enumerate() {
                                menu.action(index as u32 + 1, &item.pin.name)?;
                            }
                            let mut anchor = POINT::default();
                            let _ = GetCursorPos(&mut anchor);
                            let choice = menu.track(hwnd, anchor, TPM_RIGHTBUTTON);
                            if let Some(item) = choice
                                .checked_sub(1)
                                .and_then(|index| items.get(index as usize))
                            {
                                if let Some(action) = dock_system_action(&item.pin.target) {
                                    send(Request::System(action))?;
                                } else if matches!(
                                    watcher::activate(&item.windows),
                                    watcher::Activation::Missing
                                ) {
                                    send(Request::Launch(item.pin.clone()))?;
                                }
                            }
                        } else if let Some(bar) = &mut topbar {
                            match bar.keyboard_menu()? {
                                Some(Action::System(action)) => send(Request::System(action))?,
                                Some(Action::WindowsSettings(target)) => {
                                    send(Request::Settings(target))?
                                }
                                Some(Action::DockSettings) => {
                                    post_dock_message(hwnd, WM_SETTINGS_OPEN)
                                }
                                None => {}
                            }
                        }
                        continue;
                    }
                    if let Some(dialog) = &settings_window
                        && dialog.route(&message)
                    {
                        continue;
                    }
                    if let Some(bar) = &mut topbar
                        && message.hwnd == bar.hwnd()
                    {
                        let opens_menu = message.message == WM_LBUTTONUP;
                        let result = bar.handle(&message, placement.monitor());
                        if opens_menu {
                            watcher.refresh();
                        }
                        match result {
                            Ok(Some(Action::System(action))) => send(Request::System(action))?,
                            Ok(Some(Action::WindowsSettings(settings))) => {
                                send(Request::Settings(settings))?
                            }
                            Ok(Some(Action::DockSettings)) => {
                                post_dock_message(hwnd, WM_SETTINGS_OPEN)
                            }
                            Ok(None) => {}
                            Err(error) if message.message == WM_LBUTTONUP => {
                                show_settings_error(bar.hwnd(), &error);
                            }
                            Err(error) => return Err(error),
                        }
                        continue;
                    }
                    if message.hwnd == hwnd {
                        let recovery_tick =
                            message.message == WM_TIMER && message.wParam.0 == RECOVERY_TIMER;
                        if matches!(message.message, WM_REPOSITION | WM_SHELL_RESTART) {
                            crate::theme::refresh();
                            if let Some(taskbar) = &taskbar {
                                taskbar.refresh();
                            }
                            if message.message == WM_SHELL_RESTART {
                                placement.explorer_restarted()?;
                                if let Some(bar) = &mut topbar {
                                    bar.explorer_restarted()?;
                                }
                            }
                            placement.update(items.len())?;
                            if let Some(bar) = &mut topbar {
                                bar.update(placement.monitor())?;
                            }
                            frame_pending = true;
                        }
                        if settings.hide_on_fullscreen
                            && (recovery_tick
                                || matches!(
                                    message.message,
                                    WM_APPS_CHANGED
                                        | WM_FULLSCREEN_CHECK
                                        | WM_REPOSITION
                                        | WM_SHELL_RESTART
                                ))
                        {
                            let show = !placement.fullscreen();
                            if visible != show {
                                visible = show;
                                if show {
                                    if let Some(taskbar) = &taskbar {
                                        taskbar.refresh();
                                    }
                                    placement.update(items.len())?;
                                    placement.show()?;
                                } else {
                                    let _ = ShowWindow(hwnd, SW_HIDE);
                                }
                                if let Some(bar) = &mut topbar {
                                    bar.set_visible(show)?;
                                }
                                cursor = None;
                                pressed = None;
                                hidden = settings.auto_hide;
                                label.reset(hwnd);
                                let _ = KillTimer(Some(hwnd), HIDE_TIMER);
                                frame_pending = true;
                            }
                        }
                        if recovery_tick {
                            let mut point = POINT::default();
                            if visible
                                && settings.auto_hide
                                && hidden
                                && GetCursorPos(&mut point).is_ok()
                                && placement.reveal_at(point)
                            {
                                // Do not depend on WM_MOUSEMOVE reaching the
                                // three-pixel strip after shell z-order changes.
                                placement.show()?;
                                let _ = KillTimer(Some(hwnd), HIDE_TIMER);
                                hidden = false;
                                frame_pending = true;
                                // The pointer may already be stationary here;
                                // initialize hover/leave tracking without
                                // waiting for another mouse-move message.
                                windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point)
                                    .ok()?;
                                cursor = Some(point);
                                let _ = TrackMouseEvent(&mut TRACKMOUSEEVENT {
                                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                                    dwFlags: TME_LEAVE,
                                    hwndTrack: hwnd,
                                    dwHoverTime: 0,
                                });
                            }
                            continue;
                        }
                        match message.message {
                            WM_DROPFILES => {
                                use windows::Win32::UI::Shell::{
                                    DragFinish, DragQueryFileW, DragQueryPoint, HDROP,
                                };
                                let drop = HDROP(message.wParam.0 as *mut _);
                                let mut point = POINT::default();
                                let _ = DragQueryPoint(drop, &mut point);
                                let count = DragQueryFileW(drop, u32::MAX, None);
                                let mut paths = Vec::new();
                                for index in 0..count.min(256) {
                                    let length = DragQueryFileW(drop, index, None);
                                    let mut name = vec![0u16; length as usize + 1];
                                    let read = DragQueryFileW(drop, index, Some(&mut name));
                                    paths.push(String::from_utf16_lossy(&name[..read as usize]));
                                }
                                DragFinish(drop);
                                if let Some(index) = scene
                                    .hit(point, layout.scale)
                                    .filter(|&index| !items[index].utility())
                                {
                                    send(Request::LaunchFiles(items[index].pin.clone(), paths))?;
                                } else {
                                    let previous = pins.clone();
                                    for target in paths {
                                        let path = std::path::Path::new(&target);
                                        if pins.len() < config::MAX_PINS
                                            && path
                                                .extension()
                                                .and_then(|ext| ext.to_str())
                                                .is_some_and(|ext| {
                                                    ext.eq_ignore_ascii_case("exe")
                                                        || ext.eq_ignore_ascii_case("lnk")
                                                })
                                            && !pins.iter().any(|pin| {
                                                watcher::key(&pin.target) == watcher::key(&target)
                                            })
                                        {
                                            pins.push(Pin {
                                                name: path
                                                    .file_stem()
                                                    .unwrap_or_default()
                                                    .to_string_lossy()
                                                    .into_owned(),
                                                target,
                                            });
                                        }
                                    }
                                    if let Err(error) = Source::open()
                                        .and_then(|source| source.save_pins(&previous, &pins))
                                    {
                                        pins = previous;
                                        show_dock_error(
                                            hwnd,
                                            "Could not pin the dropped apps.",
                                            &error,
                                        );
                                    }
                                }
                                watcher.refresh();
                                continue;
                            }
                            WM_SETTINGS_OPEN => {
                                if let Some(dialog) =
                                    settings_window.as_ref().filter(|dialog| dialog.is_open())
                                {
                                    dialog.focus();
                                } else {
                                    match SettingsWindow::open(instance, Some(hwnd)) {
                                        Ok(dialog) => settings_window = Some(dialog),
                                        Err(error) => {
                                            show_settings_error(hwnd, &error);
                                            watcher.refresh();
                                        }
                                    }
                                }
                                continue;
                            }
                            WM_RBUTTONUP => {
                                label.reset(hwnd);
                                let point = client_point(message.lParam);
                                let hovered = scene.hit(point, layout.scale);
                                let item = hovered.map(|index| (index, &items[index]));
                                if cursor.is_some() {
                                    let updated = WindowShape::new(layout, &scene, None);
                                    if shape != updated {
                                        updated.apply(hwnd)?;
                                        shape = updated;
                                        renderer.paint(layout, &scene, &items, None, None, None)?;
                                    }
                                }
                                let choice =
                                    dock_menu(hwnd, point, item, pins.len() < config::MAX_PINS);
                                cursor = None;
                                pressed = None;
                                frame_pending = true;
                                if settings.auto_hide
                                    && hide_now(hwnd, appearance.auto_hide_delay_ms)
                                {
                                    hidden = true;
                                }
                                match choice {
                                    Ok(Some(MenuAction::Settings)) => {
                                        post_dock_message(hwnd, WM_SETTINGS_OPEN)
                                    }
                                    Ok(Some(MenuAction::Quit)) => return Ok(false),
                                    Ok(Some(MenuAction::Startup)) => {
                                        if let Err(error) = crate::startup::toggle() {
                                            show_dock_error(
                                                hwnd,
                                                "Could not change launch at sign-in.",
                                                &error,
                                            );
                                        }
                                    }
                                    Ok(Some(MenuAction::Close(index))) => {
                                        for &window in &items[index].windows {
                                            let _ = PostMessageW(
                                                Some(window),
                                                WM_CLOSE,
                                                WPARAM(0),
                                                LPARAM(0),
                                            );
                                        }
                                    }
                                    Ok(Some(MenuAction::System(action))) => {
                                        send(Request::System(action))?
                                    }
                                    Ok(Some(MenuAction::Launch(index))) => {
                                        if let Some(action) =
                                            dock_system_action(&items[index].pin.target)
                                        {
                                            send(Request::System(action))?;
                                        } else {
                                            send(Request::Launch(items[index].pin.clone()))?;
                                        }
                                        animation.launch(&items[index].pin.target);
                                    }
                                    Ok(Some(MenuAction::Focus(index, window))) => {
                                        watcher::focus(window);
                                        animation
                                            .activate(&items[index].pin.target, Activation::Focus);
                                    }
                                    Ok(Some(MenuAction::TogglePin(index))) => {
                                        items[index].pinned = !items[index].pinned;
                                        if let Err(error) =
                                            save_dock_pins(&mut pins, &items, &mut dock_shortcuts)
                                        {
                                            items[index].pinned = !items[index].pinned;
                                            show_dock_error(
                                                hwnd,
                                                "Could not update pinned apps.",
                                                &error,
                                            );
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(error) => show_dock_error(
                                        hwnd,
                                        "Could not open the dock menu.",
                                        &error,
                                    ),
                                }
                                watcher.refresh();
                                continue;
                            }
                            WM_SETTINGS_SAVED => {
                                let _ = ShowWindow(hwnd, SW_HIDE);
                                return Ok(true);
                            }
                            WM_REPOSITION | WM_FULLSCREEN_CHECK | WM_SHELL_RESTART => continue,
                            WM_RENDER_DOCK => {
                                frame_pending = true;
                                continue;
                            }
                            WM_APPS_CHANGED => {
                                let updated = dock_items(
                                    &pins,
                                    &shortcuts,
                                    watcher.snapshot(),
                                    &items,
                                    dock_shortcuts,
                                );
                                let list_changed = !items
                                    .iter()
                                    .map(|item| &item.pin.target)
                                    .eq(updated.iter().map(|item| &item.pin.target));
                                let indicators_changed = !items
                                    .iter()
                                    .map(|item| item.windows.len())
                                    .eq(updated.iter().map(|item| item.windows.len()));
                                items = updated;
                                if let Some(bar) = &mut topbar {
                                    bar.foreground_changed(&items);
                                }
                                if list_changed {
                                    let targets: HashSet<_> =
                                        items.iter().map(|item| item.pin.target.clone()).collect();
                                    renderer.retain_icons(&targets);
                                    animation.retain(&targets);
                                    requested.retain(|target| targets.contains(target));
                                    for item in &items {
                                        if dock_system_action(&item.pin.target).is_none()
                                            && requested.insert(item.pin.target.clone())
                                        {
                                            send(Request::Icon(item.pin.target.clone()))?;
                                        }
                                    }
                                    placement.update(items.len())?;
                                    scene.icons.clear();
                                    cursor = None;
                                    pressed = None;
                                    let _ =
                                        windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture(
                                        );
                                    label.reset(hwnd);
                                }
                                if list_changed || indicators_changed || !items.is_empty() {
                                    frame_pending = true;
                                }
                                continue;
                            }
                            WM_SHELL_RESULT => {
                                let mut repaint = false;
                                for event in events.try_iter() {
                                    match event {
                                        Event::Icon(target, icon) => {
                                            if requested.contains(&target) {
                                                renderer.set_icon(target, icon);
                                                repaint = true;
                                            }
                                        }
                                        Event::Shortcut(shortcut, target) => {
                                            shortcuts.insert(shortcut, target);
                                            watcher.refresh();
                                        }
                                        Event::LaunchFailed(name, error) => {
                                            show_dock_error(
                                                hwnd,
                                                &format!("Could not open {name}."),
                                                &error,
                                            );
                                            watcher.refresh();
                                            repaint = true;
                                        }
                                        Event::WorkerFailed(error) => return Err(error),
                                        Event::SystemFailed(action, error) => {
                                            show_dock_error(
                                                hwnd,
                                                &format!(
                                                    "Could not complete ‘{}’.",
                                                    action.label()
                                                ),
                                                &error,
                                            );
                                            watcher.refresh();
                                            repaint = true;
                                        }
                                        Event::SettingsFailed(error) => {
                                            show_settings_error(hwnd, &error);
                                            watcher.refresh();
                                            repaint = true;
                                        }
                                    }
                                }
                                if repaint {
                                    frame_pending = true;
                                }
                                continue;
                            }
                            WM_MOUSEMOVE => {
                                if settings.auto_hide {
                                    let _ = KillTimer(Some(hwnd), HIDE_TIMER);
                                    if hidden {
                                        hidden = false;
                                        frame_pending = true;
                                    }
                                }
                                let point = client_point(message.lParam);
                                if message.wParam.0 & MK_LBUTTON.0 as usize == 0 {
                                    if pressed.take().is_some() {
                                        frame_pending = true;
                                        let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                                    }
                                } else if let Some(press) = &mut pressed {
                                    press.dragging |= items[press.index].movable()
                                        && press.moved(point, layout.scale);
                                    frame_pending |= press.dragging;
                                }
                                if cursor.is_none() {
                                    let _ = TrackMouseEvent(&mut TRACKMOUSEEVENT {
                                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                                        dwFlags: TME_LEAVE,
                                        hwndTrack: hwnd,
                                        dwHoverTime: 0,
                                    });
                                }
                                if cursor != Some(point) || pressed.is_some() {
                                    frame_pending = true;
                                }
                                cursor = Some(point);
                            }
                            WM_MOUSELEAVE => {
                                label.reset(hwnd);
                                if pressed.is_none() {
                                    cursor = None;
                                }
                                frame_pending = true;
                                if settings.auto_hide
                                    && hide_now(hwnd, appearance.auto_hide_delay_ms)
                                {
                                    hidden = true;
                                }
                            }
                            WM_TIMER if message.wParam.0 == HIDE_TIMER => {
                                let _ = KillTimer(Some(hwnd), HIDE_TIMER);
                                if cursor.is_none() {
                                    hidden = true;
                                    frame_pending = true;
                                }
                                continue;
                            }
                            WM_TIMER if message.wParam.0 == LABEL_TIMER => {
                                let _ = KillTimer(Some(hwnd), LABEL_TIMER);
                                frame_pending = true;
                                continue;
                            }
                            WM_LBUTTONDOWN => {
                                label.reset(hwnd);
                                let point = client_point(message.lParam);
                                pressed = scene.hit(point, layout.scale).map(|index| Press {
                                    index,
                                    origin: point,
                                    dragging: false,
                                });
                                if pressed.is_some() {
                                    windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd);
                                }
                                frame_pending |= pressed.is_some();
                            }
                            WM_CAPTURECHANGED => {
                                pressed = None;
                                frame_pending = true;
                            }
                            WM_LBUTTONUP => {
                                if let Some(press) = pressed.take() {
                                    let _ =
                                        windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture(
                                        );
                                    frame_pending = true;
                                    let point = client_point(message.lParam);
                                    let index = press.index;
                                    if items[index].movable()
                                        && (press.dragging || press.moved(point, layout.scale))
                                    {
                                        if let Some((destination, pin, _)) =
                                            press.plan(layout, &items, point)
                                        {
                                            let previous_pin = items[index].pinned;
                                            items[index].pinned = pin;
                                            move_item(&mut items, index, destination);
                                            if let Err(error) = save_dock_pins(
                                                &mut pins,
                                                &items,
                                                &mut dock_shortcuts,
                                            ) {
                                                move_item(&mut items, destination, index);
                                                items[index].pinned = previous_pin;
                                                show_dock_error(
                                                    hwnd,
                                                    "Could not save the new dock order.",
                                                    &error,
                                                );
                                                watcher.refresh();
                                            }
                                        }
                                    } else if !press.moved(point, layout.scale)
                                        && scene.hit(point, layout.scale) == Some(index)
                                    {
                                        let item = &items[index];
                                        if let Some(action) = dock_system_action(&item.pin.target) {
                                            send(Request::System(action))?;
                                            continue;
                                        }
                                        if item.pin.target == "shell:Downloads" {
                                            let icon = scene.icons[index];
                                            let mut anchor = POINT {
                                                x: ((icon.x + icon.width / 2.0) * layout.scale)
                                                    as i32,
                                                y: (icon.y * layout.scale) as i32,
                                            };
                                            let _ = ClientToScreen(hwnd, &mut anchor);
                                            match crate::downloads::show(hwnd, anchor, appearance) {
                                                Ok(Some(pin)) => {
                                                    send(Request::Launch(pin))?;
                                                }
                                                Ok(None) => {}
                                                Err(error) => show_dock_error(
                                                    hwnd,
                                                    "Could not read Downloads.",
                                                    &error,
                                                ),
                                            }
                                            cursor = None;
                                            if settings.auto_hide
                                                && hide_now(hwnd, appearance.auto_hide_delay_ms)
                                            {
                                                hidden = true;
                                            }
                                            continue;
                                        }
                                        match watcher::activate(&item.windows) {
                                            watcher::Activation::Minimized => animation
                                                .activate(&item.pin.target, Activation::Minimize),
                                            watcher::Activation::Focused => animation
                                                .activate(&item.pin.target, Activation::Focus),
                                            watcher::Activation::Missing
                                                if item.pinned || item.utility() =>
                                            {
                                                send(Request::Launch(item.pin.clone()))?;
                                                animation.launch(&item.pin.target);
                                            }
                                            watcher::Activation::Missing
                                            | watcher::Activation::Unchanged => {}
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }
}

fn hide_now(hwnd: HWND, delay_ms: u32) -> bool {
    unsafe { SetTimer(Some(hwnd), HIDE_TIMER, delay_ms, None) == 0 }
}

fn move_item(items: &mut [DockItem], from: usize, to: usize) {
    if from < to {
        items[from..=to].rotate_left(1);
    } else {
        items[to..=from].rotate_right(1);
    }
}

fn save_dock_pins(pins: &mut Vec<Pin>, items: &[DockItem], dock: &mut config::Dock) -> Result<()> {
    let updated: Vec<_> = items
        .iter()
        .filter(|item| item.pinned)
        .map(|item| item.pin.clone())
        .collect();
    let placement = shortcut_placement(items, *dock);
    if *pins != updated || placement != *dock {
        Source::open()?.save_order(pins, &updated, Some(placement))?;
        *pins = updated;
        *dock = placement;
    }
    Ok(())
}

fn shortcut_placement(items: &[DockItem], dock: config::Dock) -> config::Dock {
    let mut placement = dock;
    let position = |target: &str| {
        items
            .iter()
            .position(|item| item.pin.target == target)
            .map(|index| {
                items[..index]
                    .iter()
                    .filter(|item| dock_system_action(&item.pin.target).is_none())
                    .count()
            })
    };
    if let Some(position) = position("dock:Start") {
        placement.start_position = position;
    }
    if let Some(position) = position("dock:Search") {
        placement.search_position = position;
    }
    if let (Some(start), Some(search)) = (
        items
            .iter()
            .position(|item| item.pin.target == "dock:Start"),
        items
            .iter()
            .position(|item| item.pin.target == "dock:Search"),
    ) {
        placement.search_first = search < start;
    }
    placement
}

enum MenuAction {
    Launch(usize),
    Focus(usize, HWND),
    TogglePin(usize),
    Settings,
    Quit,
    Startup,
    Close(usize),
    System(crate::system::SystemAction),
}

fn window_summary(item: &DockItem) -> String {
    let count = item.windows.len();
    if count == 0 {
        return "Click to open".into();
    }
    let minimized = item
        .windows
        .iter()
        .filter(|&&window| unsafe { IsIconic(window).as_bool() })
        .count();
    let label = if count == 1 {
        "1 window".into()
    } else {
        format!("{count} windows")
    };
    if minimized > 0 {
        format!("{label} · {minimized} minimized")
    } else {
        label
    }
}

fn dock_menu(
    hwnd: HWND,
    mut point: POINT,
    item: Option<(usize, &DockItem)>,
    can_pin: bool,
) -> Result<Option<MenuAction>> {
    unsafe { ClientToScreen(hwnd, &mut point).ok()? };
    let mut menu = Popup::new(hwnd, 264)?;
    if let Some((_, item)) = item {
        menu.heading(&item.pin.name)?;
        menu.detail(&window_summary(item))?;
        menu.divider()?;
        for (index, &window) in item.windows.iter().enumerate() {
            let mut title = watcher::title(window);
            if unsafe { IsIconic(window).as_bool() } {
                title.push_str(" (minimized)");
            }
            menu.action(index as u32 + 4, &title)?;
        }
        if !item.windows.is_empty() {
            menu.action(
                0x7001,
                if item.windows.len() == 1 {
                    "Close window"
                } else {
                    "Close all windows"
                },
            )?;
            menu.divider()?;
        }
        menu.action(
            1,
            if item.windows.is_empty() {
                "Open"
            } else {
                "Launch Again"
            },
        )?;
        let label = if item.pinned {
            "Unpin from Dock"
        } else {
            "Pin to Dock"
        };
        if !item.utility() {
            menu.option(2, label, item.pinned || can_pin, false)?;
        }
        menu.divider()?;
    }
    menu.action(3, "Dock Settings…")?;
    menu.option(0x7007, "Launch at sign-in", true, crate::startup::enabled())?;
    menu.action(0x7002, "Task View")?;
    menu.action(0x7003, "Show desktop")?;
    menu.action(0x7004, "Switch input language")?;
    menu.divider()?;
    menu.action(0x7005, "Restore Windows taskbar and quit")?;
    menu.action(0x7006, "Quit Styled Dock")?;
    Ok(
        match (
            menu.track(hwnd, point, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN),
            item,
        ) {
            (1, Some((index, _))) => Some(MenuAction::Launch(index)),
            (2, Some((index, _))) => Some(MenuAction::TogglePin(index)),
            (3, _) => Some(MenuAction::Settings),
            (0x7007, _) => Some(MenuAction::Startup),
            (0x7001, Some((index, _))) => Some(MenuAction::Close(index)),
            (0x7002, _) => Some(MenuAction::System(crate::system::SystemAction::TaskView)),
            (0x7003, _) => Some(MenuAction::System(crate::system::SystemAction::ShowDesktop)),
            (0x7004, _) => Some(MenuAction::System(
                crate::system::SystemAction::InputLanguage,
            )),
            (0x7005 | 0x7006, _) => Some(MenuAction::Quit),
            (choice, Some((index, item))) if choice >= 4 => item
                .windows
                .get(choice as usize - 4)
                .map(|&window| MenuAction::Focus(index, window)),
            _ => None,
        },
    )
}

fn show_settings_error(hwnd: HWND, error: &Error) {
    show_dock_error(hwnd, "Could not apply the quick setting.", error);
}

fn show_dock_error(hwnd: HWND, message: &str, error: &Error) {
    let text = HSTRING::from(format!("{message}\n\n{error}"));
    unsafe { MessageBoxW(Some(hwnd), &text, w!("Styled Dock"), MB_ICONERROR) };
}

fn client_point(position: LPARAM) -> POINT {
    POINT {
        x: position.0 as i16 as i32,
        y: (position.0 >> 16) as i16 as i32,
    }
}

#[derive(PartialEq)]
struct WindowShape {
    panel: RECT,
    diameter: i32,
    raised: Vec<RECT>,
    clip: Option<RECT>,
}

impl WindowShape {
    fn new(layout: Layout, scene: &Scene, overlay: Option<Bounds>) -> Self {
        let panel = scene.panel.pixels(layout.scale);
        let diameter = (2.0 * (layout.dock.radius() * layout.scale - 2.0).max(0.5)).round() as i32;
        let raised = scene
            .icons
            .iter()
            .map(|icon| icon.pixels(layout.scale))
            .filter(|icon| icon.top < panel.top)
            .map(|icon| RECT {
                left: icon.left - 1,
                top: icon.top - 1,
                right: icon.right + 1,
                bottom: icon.bottom + 1,
            })
            .chain(overlay.map(|bounds| bounds.pixels(layout.scale)))
            .collect();
        Self {
            panel,
            diameter,
            raised,
            clip: (panel.bottom > layout.pixels.1 as i32).then_some(RECT {
                left: 0,
                top: 0,
                right: layout.pixels.0 as i32,
                bottom: layout.pixels.1 as i32,
            }),
        }
    }

    fn apply(&self, hwnd: HWND) -> Result<()> {
        let p = self.panel;
        unsafe {
            let region = CreateRoundRectRgn(
                p.left,
                p.top,
                p.right + 1,
                p.bottom + 1,
                self.diameter,
                self.diameter,
            );
            if region.is_invalid() {
                return Err(Error::from_thread());
            }
            for (bounds, mode) in self
                .raised
                .iter()
                .map(|bounds| (bounds, RGN_OR))
                .chain(self.clip.iter().map(|bounds| (bounds, RGN_AND)))
            {
                let icon = CreateRectRgnIndirect(bounds);
                if icon.is_invalid() {
                    let _ = DeleteObject(region.into());
                    return Err(Error::empty());
                }
                let combined = CombineRgn(Some(region), Some(region), Some(icon), mode);
                let _ = DeleteObject(icon.into());
                if combined == RGN_ERROR {
                    let _ = DeleteObject(region.into());
                    return Err(Error::empty());
                }
            }
            // Windows owns the region after SetWindowRgn succeeds.
            if SetWindowRgn(hwnd, Some(region), false) == 0 {
                let error = Error::from_thread();
                let _ = DeleteObject(region.into());
                return Err(error);
            }
        }
        Ok(())
    }
}
fn suppress_window_border(hwnd: HWND) {
    unsafe {
        let border = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&raw const border).cast(),
            size_of_val(&border) as u32,
        );
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if watcher::shell_message(message, wparam, lparam) {
        return LRESULT(0);
    }
    if message >= 0xc000 && message == TASKBAR_CREATED.get() {
        post_dock_message(hwnd, WM_SHELL_RESTART);
        return LRESULT(0);
    }
    match message {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            // Validate the update region here and keep rendering outside the window callback.
            let mut paint = PAINTSTRUCT::default();
            unsafe {
                BeginPaint(hwnd, &mut paint);
                let _ = EndPaint(hwnd, &paint);
                let _ = PostMessageW(Some(hwnd), WM_RENDER_DOCK, WPARAM(0), LPARAM(0));
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            LRESULT(0)
        }
        WM_DPICHANGED | WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
            post_dock_message(hwnd, WM_REPOSITION);
            LRESULT(0)
        }
        WM_APPBAR => {
            match wparam.0 as u32 {
                ABN_POSCHANGED => post_dock_message(hwnd, WM_REPOSITION),
                ABN_FULLSCREENAPP => post_dock_message(hwnd, WM_FULLSCREEN_CHECK),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE | WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn post_dock_message(hwnd: HWND, message: u32) {
    let _ = unsafe { PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(0)) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Bounds;

    #[test]
    fn app_labels_wait_and_restart_for_each_hover() {
        let start = Instant::now();
        let mut label = HoverLabel::default();
        assert_eq!(label.update(Some(0), 1000, start).0, None);
        assert_eq!(
            label
                .update(Some(0), 1000, start + Duration::from_millis(999))
                .0,
            None
        );
        assert_eq!(
            label.update(Some(0), 1000, start + Duration::from_millis(1000)),
            (Some(0), None)
        );
        assert_eq!(
            label
                .update(Some(1), 1000, start + Duration::from_millis(1001))
                .0,
            None
        );
        assert_eq!(
            label.update(None, 1000, start + Duration::from_millis(2002)),
            (None, None)
        );
        assert_eq!(
            label
                .update(Some(1), 1000, start + Duration::from_millis(2003))
                .0,
            None
        );
        assert_eq!(
            label.update(Some(1), 0, start + Duration::from_millis(2004)),
            (Some(1), None)
        );
    }

    #[test]
    fn drag_plans_pin_running_apps_and_protect_fixed_shortcuts() {
        let mut items = dock_items(
            &[Pin {
                name: "Pinned".into(),
                target: "pin.exe".into(),
            }],
            &HashMap::new(),
            vec![watcher::App {
                key: "run.exe".into(),
                target: "run.exe".into(),
                path: "run.exe".into(),
                name: "Running".into(),
                windows: vec![],
            }],
            &[],
            config::Dock {
                show_start: false,
                show_search: false,
                ..Default::default()
            },
        );
        let layout = Layout {
            width: 600.0,
            height: 160.0,
            scale: 1.0,
            count: items.len(),
            pixels: (600, 160),
            dock: DockSize::new(Default::default()),
        };
        let point = |index: usize, fraction: f32| {
            let (x, y, size) = layout.icon(index);
            POINT {
                x: (x + size * fraction) as i32,
                y: (y + size / 2.0) as i32,
            }
        };
        let press = Press {
            index: 1,
            origin: point(1, 0.5),
            dragging: true,
        };
        let (to, pin, _) = press.plan(layout, &items, point(0, 0.1)).unwrap();
        assert_eq!((to, pin), (0, true));
        assert_eq!(press.plan(layout, &items, point(1, 0.25)).unwrap().1, false);
        assert!(press.plan(layout, &items, point(3, 0.75)).is_none());
        assert!(press.plan(layout, &items, POINT { x: 0, y: 0 }).is_none());
        items[1].pinned = pin;
        move_item(&mut items, 1, to);
        assert_eq!(items[0].pin.target, "run.exe");
        assert!(items[0].pinned && items[1].pinned);
        // An empty pinned section can be populated at the leading edge.
        items[0].pinned = false;
        items[1].pinned = false;
        let press = Press {
            index: 1,
            origin: point(1, 0.5),
            dragging: true,
        };
        assert_eq!(press.plan(layout, &items, point(0, 0.1)).unwrap().1, true);
    }

    #[test]
    fn shortcut_visibility_preserves_apps_and_fixed_order() {
        for bits in 0..8 {
            let dock = config::Dock {
                show_start: bits & 1 != 0,
                show_search: bits & 2 != 0,
                show_downloads: bits & 4 != 0,
                ..Default::default()
            };
            let pins = [Pin {
                name: "App".into(),
                target: "app.exe".into(),
            }];
            let items = dock_items(&pins, &HashMap::new(), vec![], &[], dock);
            let mut expected = vec![];
            if dock.show_start {
                expected.push("dock:Start");
            }
            if dock.show_search {
                expected.push("dock:Search");
            }
            expected.push("app.exe");
            if dock.show_downloads {
                expected.push("shell:Downloads");
            }
            expected.push("shell:RecycleBinFolder");
            assert_eq!(
                items
                    .iter()
                    .map(|item| item.pin.target.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(
                items
                    .iter()
                    .find(|item| item.pin.target == "app.exe")
                    .unwrap()
                    .pinned
            );
            assert!(
                items
                    .iter()
                    .filter(|item| item.utility())
                    .all(|item| !item.pinned)
            );
        }
        assert!(matches!(
            dock_system_action("dock:Start"),
            Some(crate::system::SystemAction::Start)
        ));
        assert!(matches!(
            dock_system_action("dock:Search"),
            Some(crate::system::SystemAction::Search)
        ));
        assert!(dock_system_action("shell:Downloads").is_none());
    }

    #[test]
    fn start_and_search_drag_across_apps_and_survive_refresh() {
        let pins = [Pin {
            name: "App".into(),
            target: "app.exe".into(),
        }];
        let shortcuts = HashMap::new();
        let mut items = dock_items(&pins, &shortcuts, vec![], &[], config::Dock::default());
        assert_eq!(items[0].pin.target, "dock:Start");
        assert_eq!(items[1].pin.target, "dock:Search");
        let layout = Layout {
            width: 600.0,
            height: 160.0,
            scale: 1.0,
            count: items.len(),
            pixels: (600, 160),
            dock: DockSize::new(Default::default()),
        };
        let (x, y, size) = layout.icon(items.len() - 1);
        let point = POINT {
            x: (x + size * 0.9) as i32,
            y: (y + size / 2.0) as i32,
        };
        let press = Press {
            index: 0,
            origin: POINT::default(),
            dragging: true,
        };
        let (destination, pin, _) = press.plan(layout, &items, point).unwrap();
        assert!(!pin);
        move_item(&mut items, 0, destination);
        let dock = shortcut_placement(&items, config::Dock::default());
        let refreshed = dock_items(&pins, &shortcuts, vec![], &items, dock);
        assert_eq!(
            refreshed
                .iter()
                .map(|item| &item.pin.target)
                .collect::<Vec<_>>(),
            items
                .iter()
                .map(|item| &item.pin.target)
                .collect::<Vec<_>>()
        );
        assert_eq!(refreshed[0].pin.target, "dock:Search");
        assert_eq!(refreshed.last().unwrap().pin.target, "dock:Start");
        assert!(
            refreshed
                .iter()
                .filter(|item| item.utility())
                .all(|item| !item.pinned)
        );
    }

    #[test]
    fn dragging_running_apps_keeps_order_on_refresh() {
        let pins = [Pin {
            name: "Pinned".into(),
            target: "pinned.exe".into(),
        }];
        let app = |target: &str| watcher::App {
            key: target.into(),
            target: target.into(),
            path: target.into(),
            name: target.into(),
            windows: vec![HWND::default()],
        };
        let shortcuts = HashMap::new();
        let mut items = dock_items(
            &pins,
            &shortcuts,
            vec![app("one.exe"), app("two.exe")],
            &[],
            config::Dock {
                show_start: false,
                show_search: false,
                ..Default::default()
            },
        );
        let bounds = |x, width| Bounds {
            x,
            y: 0.0,
            width,
            height: 40.0,
        };
        let scene = Scene {
            panel: bounds(0.0, 150.0),
            icons: vec![bounds(10.0, 35.0), bounds(55.0, 35.0), bounds(100.0, 35.0)],
        };
        let press = Press {
            index: 2,
            origin: POINT { x: 115, y: 20 },
            dragging: true,
        };
        assert_eq!(
            press.drop_target(&scene, POINT { x: 100, y: 50 }, 1.0),
            None
        );
        let (destination, _) = press
            .drop_target(&scene, POINT { x: 20, y: 20 }, 1.0)
            .unwrap();
        move_item(&mut items, press.index, destination);
        let updated = dock_items(
            &pins,
            &shortcuts,
            vec![app("one.exe"), app("new.exe"), app("two.exe")],
            &items,
            config::Dock {
                show_start: false,
                show_search: false,
                ..Default::default()
            },
        );
        let targets: Vec<_> = updated
            .iter()
            .map(|item| item.pin.target.as_str())
            .collect();
        assert_eq!(
            targets,
            [
                "pinned.exe",
                "two.exe",
                "one.exe",
                "new.exe",
                "shell:Downloads",
                "shell:RecycleBinFolder"
            ]
        );
        assert!(updated[0].pinned);
        assert!(!updated[1].pinned);
        assert!(updated[4].utility());
        assert!(updated[5].utility());
    }
}
