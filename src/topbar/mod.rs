//! Status bar and network, audio, battery, and display controls.
pub(crate) mod bar_render;
pub(crate) mod bar_status;
pub(crate) mod panels;

use std::ops::Range;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Globalization::{
    DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Controls::{
    ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, TOOLTIPS_CLASSW, TTF_SUBCLASS,
    TTM_ADDTOOLW, TTM_NEWTOOLRECTW, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
    WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HKL, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Shell::{ABE_TOP, ABN_POSCHANGED};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, HSTRING, PCWSTR, PWSTR, Result, w};

use crate::bar_render::{BarContents, BarIcon, BarRenderer, HEIGHT};
use crate::bar_status::{
    Audio, Battery, Connection, NetworkChanges, NetworkStatus, Power, PowerPlan, WM_AUDIO,
    WM_AUDIO_DEVICE, WM_NETWORK,
};
use crate::icons::Symbol;
use crate::placement::{AppBar, WM_APPBAR, WM_REPOSITION, monitor_info};
use crate::platform::input_layout::{self, Target as InputTarget};
use crate::popup::{Popup, scale};
use crate::system::{self, SystemAction};
use crate::window::DockItem;

const CLOCK_TIMER: usize = 1;
const HOVER_TIMER: usize = 2;
const INPUT_TIMER: usize = 3;
const WM_POWER_STATUS: u32 = WM_APP + 8;
const WM_CLOCK_CHANGED: u32 = WM_APP + 12;
const BRIGHTNESS_SLIDER: u32 = 30;

pub enum Action {
    System(SystemAction),
    WindowsSettings(&'static str),
    DockSettings,
}

pub struct TopBar {
    accessibility: crate::ui::accessibility::Surface,
    accessible_keys: Vec<(String, BarIcon)>,
    hwnd: HWND,
    appbar: Option<AppBar>,
    power: Option<Power>,
    network_changes: Option<NetworkChanges>,
    audio: Option<Audio>,
    monitor: HMONITOR,
    bounds: RECT,
    renderer: BarRenderer,
    dpi: i32,
    active: String,
    active_window: Option<(HWND, u32)>,
    clock: String,
    battery: Battery,
    network: NetworkStatus,
    input_target: InputTarget,
    input_layout: HKL,
    input_text: String,
    input_name: String,
    input_tooltip: HWND,
    tooltip_text: HSTRING,
    tooltip_bounds: RECT,
    volume: Option<(f32, bool)>,
    hitboxes: Vec<(BarIcon, Range<i32>)>,
    hovered: Option<BarIcon>,
    tracking: bool,
    tray: Option<crate::tray::Tray>,
}

impl TopBar {
    pub fn new(
        instance: HINSTANCE,
        monitor: HMONITOR,
        opacity: f32,
        show_tray: bool,
    ) -> Result<Self> {
        let hwnd = unsafe {
            let class = WNDCLASSW {
                style: CS_DBLCLKS,
                hInstance: instance,
                lpszClassName: w!("StyledDockTopBar"),
                lpfnWndProc: Some(window_proc),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(Error::from_thread());
            }
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP,
                class.lpszClassName,
                &windows::core::HSTRING::from(crate::i18n::text("Styled Dock Top Bar")),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance),
                None,
            )?
        };
        let renderer = match BarRenderer::new(hwnd, opacity) {
            Ok(renderer) => renderer,
            Err(error) => {
                let _ = unsafe { DestroyWindow(hwnd) };
                return Err(error);
            }
        };
        let mut bar = Self {
            accessibility: crate::ui::accessibility::Surface::new(hwnd),
            accessible_keys: Vec::new(),
            hwnd,
            appbar: None,
            power: None,
            network_changes: None,
            audio: None,
            monitor: HMONITOR::default(),
            bounds: RECT::default(),
            renderer,
            dpi: 0,
            active: crate::i18n::text("Desktop").into(),
            active_window: None,
            clock: String::new(),
            battery: Power::status(),
            network: NetworkStatus::read(),
            input_target: InputTarget::default(),
            input_layout: HKL::default(),
            input_text: "—".into(),
            input_name: crate::i18n::text("Keyboard layout").into(),
            input_tooltip: HWND::default(),
            tooltip_text: HSTRING::new(),
            tooltip_bounds: RECT::default(),
            volume: None,
            hitboxes: Vec::new(),
            hovered: None,
            tracking: false,
            tray: None,
        };
        bar.init_input_tooltip(instance);
        bar.update(monitor)?;
        bar.appbar = Some(AppBar::new(hwnd, ABE_TOP)?);
        bar.update(monitor)?;
        bar.power = Some(Power::new(hwnd));
        bar.network_changes = NetworkChanges::new(hwnd);
        bar.audio = Audio::new(hwnd).ok();
        if show_tray {
            bar.tray = Some(crate::tray::Tray::new(instance, hwnd)?);
        }
        panels::refresh_radios(hwnd);
        crate::platform::hotspot::refresh(hwnd);
        bar.refresh_volume();
        bar.refresh_input();
        bar.tick();
        unsafe { SetTimer(Some(hwnd), INPUT_TIMER, 250, None) };
        Ok(bar)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn focus_accessible(&mut self) {
        self.refresh_input();
        self.accessibility.focus_first();
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.renderer.set_opacity(opacity);
        self.invalidate();
    }

    pub fn refresh_theme(&self) {
        self.invalidate();
    }

    pub fn set_visible(&mut self, visible: bool) -> Result<()> {
        if visible {
            crate::placement::show_topmost(self.hwnd)?;
            self.update(self.monitor)?;
        } else {
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }
        Ok(())
    }

    pub fn explorer_restarted(&mut self) -> Result<()> {
        if let Some(bar) = &mut self.appbar {
            bar.reregister()?;
        }
        Ok(())
    }

    pub fn resume(&mut self) {
        crate::platform::hotspot::refresh(self.hwnd);
        if let Some(audio) = &mut self.audio {
            audio.reconnect();
        } else {
            self.audio = Audio::new(self.hwnd).ok();
        }
        // A service can be absent at launch and become available after resume.
        self.network_changes = NetworkChanges::new(self.hwnd);
        panels::refresh_radios(self.hwnd);
        self.refresh_volume();
        self.refresh_input();
        self.tick();
        self.invalidate();
    }

    pub fn update(&mut self, monitor: HMONITOR) -> Result<()> {
        // Display notifications for the two windows can arrive in either order.
        // The dock may still hold a disconnected monitor's handle when this
        // window receives its DPI/work-area notification first.
        let (monitor, info) = match monitor_info(monitor) {
            Ok(info) => (monitor, info),
            Err(_) => {
                let fallback = unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTOPRIMARY) };
                (fallback, monitor_info(fallback)?)
            }
        };
        let screen = info.monitorInfo.rcMonitor;
        unsafe {
            if self.monitor != monitor {
                SetWindowPos(
                    self.hwnd,
                    None,
                    screen.left,
                    screen.top,
                    1,
                    1,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )?;
                self.monitor = monitor;
                self.bounds = RECT::default();
            }
            let dpi = GetDpiForWindow(self.hwnd) as i32;
            if self.dpi != dpi {
                self.dpi = dpi;
                self.invalidate();
            }
            let height = scale(HEIGHT, dpi);
            let bounds = if let Some(bar) = &mut self.appbar {
                bar.reserve(screen, height)?
            } else {
                RECT {
                    bottom: screen.top + height,
                    ..screen
                }
            };
            if self.bounds != bounds {
                SetWindowPos(
                    self.hwnd,
                    None,
                    bounds.left,
                    bounds.top,
                    (bounds.right - bounds.left).max(1),
                    (bounds.bottom - bounds.top).max(1),
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )?;
                self.bounds = bounds;
                if let Some(bar) = &self.appbar {
                    bar.position_changed();
                }
                self.invalidate();
            }
        }
        Ok(())
    }

    pub fn foreground_changed(&mut self, items: &[DockItem]) {
        self.refresh_input();
        unsafe {
            let foreground = GetForegroundWindow();
            let mut pid = 0;
            GetWindowThreadProcessId(foreground, Some(&mut pid));
            if pid == std::process::id() {
                return;
            }
            let owner = GetAncestor(foreground, GA_ROOTOWNER);
            let name = if let Some(item) = items
                .iter()
                .find(|item| item.windows.contains(&foreground) || item.windows.contains(&owner))
            {
                item.pin.name.clone()
            } else {
                let mut title = [0u16; 256];
                let length = GetWindowTextW(foreground, &mut title);
                if length > 0 && foreground != GetShellWindow() {
                    String::from_utf16_lossy(&title[..length as usize])
                } else {
                    crate::i18n::text("Desktop").into()
                }
            };
            self.active_window = if name != crate::i18n::text("Desktop")
                && foreground != GetShellWindow()
                && pid != 0
            {
                Some((foreground, pid))
            } else {
                None
            };
            if self.active != name {
                self.active = name;
                self.invalidate();
            }
        }
    }

    pub fn handle(&mut self, message: &MSG, monitor: HMONITOR) -> Result<Option<Action>> {
        match message.message {
            crate::ui::accessibility::WM_FOCUS => {
                self.hovered = self
                    .accessibility
                    .focused()
                    .and_then(|index| self.accessible_keys.get(index))
                    .map(|(_, icon)| *icon);
                self.invalidate();
            }
            crate::ui::accessibility::WM_ACTION => {
                if let Some((key, context)) = self.accessibility.take_action()
                    && let Some((_, icon)) = self
                        .accessible_keys
                        .iter()
                        .find(|(candidate, _)| *candidate == key)
                    && let Some((_, range)) = self
                        .hitboxes
                        .iter()
                        .find(|(candidate, _)| candidate == icon)
                {
                    let mut synthetic = *message;
                    synthetic.message = if context && matches!(icon, BarIcon::Tray(_)) {
                        WM_RBUTTONUP
                    } else {
                        WM_LBUTTONUP
                    };
                    let x = (range.start + range.end) / 2;
                    synthetic.lParam = LPARAM(
                        (x as u16 as u32 | ((scale(HEIGHT, self.dpi) as u16 as u32 / 2) << 16))
                            as isize,
                    );
                    return self.handle(&synthetic, monitor);
                }
            }
            crate::tray::WM_TRAY_CHANGED => self.invalidate(),
            WM_REPOSITION => self.update(monitor)?,
            WM_PAINT => self.paint()?,
            WM_TIMER if message.wParam.0 == CLOCK_TIMER => self.tick(),
            WM_TIMER if message.wParam.0 == INPUT_TIMER => self.refresh_input(),
            WM_TIMER if message.wParam.0 == HOVER_TIMER => {
                self.paint()?;
                if !self.renderer.animating(self.hovered) {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), HOVER_TIMER);
                    }
                }
            }
            WM_CLOCK_CHANGED => {
                crate::theme::refresh();
                self.invalidate();
                self.tick();
            }
            WM_POWER_STATUS => {
                let battery = Power::status();
                if self.battery != battery {
                    self.battery = battery;
                    self.invalidate();
                }
            }
            panels::WM_RADIOS | crate::platform::hotspot::WM_HOTSPOT => self.invalidate(),
            WM_NETWORK => self.refresh_network(),
            WM_AUDIO_DEVICE | WM_AUDIO => {
                if message.message == WM_AUDIO_DEVICE {
                    if let Some(audio) = &mut self.audio {
                        audio.reconnect();
                    } else {
                        self.audio = Audio::new(self.hwnd).ok();
                    }
                }
                self.refresh_volume();
            }
            WM_MOUSEMOVE => {
                let x = message.lParam.0 as i16 as i32;
                if !self.tracking {
                    self.tracking = unsafe {
                        TrackMouseEvent(&mut TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: self.hwnd,
                            dwHoverTime: 0,
                        })
                        .is_ok()
                    };
                }
                let hovered = self.icon_at(x).map(|(icon, _)| icon);
                if self.hovered != hovered {
                    self.hovered = hovered;
                    self.invalidate();
                }
            }
            WM_MOUSELEAVE => {
                if crate::ui::accessibility::pointer_inside(self.hwnd) {
                    return Ok(None);
                }
                self.tracking = false;
                if self.hovered.take().is_some() {
                    self.invalidate();
                }
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONUP | WM_LBUTTONUP => {
                let x = message.lParam.0 as i16 as i32;
                if let Some((icon, anchor)) = self.icon_at(x) {
                    if let BarIcon::Tray(index) = icon {
                        if let Some(tray) = &self.tray
                            && let Some(item) = tray.state().icons().nth(index)
                        {
                            let mut point = POINT {
                                x: message.lParam.0 as i16 as i32,
                                y: (message.lParam.0 >> 16) as i16 as i32,
                            };
                            let _ = unsafe { ClientToScreen(self.hwnd, &mut point) };
                            item.click(message.message, point);
                        }
                        return Ok(None);
                    }
                    if message.message != WM_LBUTTONUP {
                        return Ok(None);
                    }
                    self.hovered = Some(icon);
                    self.paint()?;
                    let result = match icon {
                        BarIcon::System => self.system_menu(),
                        BarIcon::Active => self.active_menu(),
                        BarIcon::Battery => self.battery_menu(anchor),
                        BarIcon::Network => self.network_menu(anchor),
                        BarIcon::Input => self.input_menu(anchor),
                        BarIcon::Volume => self.volume_menu(anchor),
                        BarIcon::Clock => Ok(Some(Action::System(SystemAction::Notifications))),
                        BarIcon::QuickSettings => self.quick_settings(anchor),
                        BarIcon::Tray(_) => unreachable!(),
                    };
                    self.hovered = None;
                    self.tracking = false;
                    self.invalidate();
                    return result;
                }
            }
            _ => unsafe {
                DispatchMessageW(message);
            },
        }
        Ok(None)
    }

    fn icon_at(&self, x: i32) -> Option<(BarIcon, i32)> {
        self.hitboxes
            .iter()
            .find(|(_, bounds)| bounds.contains(&x))
            .map(|(icon, bounds)| (*icon, bounds.end))
    }

    fn refresh_volume(&mut self) {
        let volume = self.audio.as_ref().and_then(Audio::status);
        if self.volume != volume {
            self.volume = volume;
            self.invalidate();
        }
    }

    fn refresh_input(&mut self) {
        if let Some(target) = InputTarget::foreground() {
            self.input_target = target;
        }
        let active = self.input_target.active();
        if self.input_layout != active {
            self.input_layout = active;
            let layout = input_layout::layout_name(active);
            self.input_name = input_layout::label(active, layout.as_deref());
            self.input_text = input_layout::language_code(active);
            self.invalidate();
        }
    }

    fn init_input_tooltip(&mut self, instance: HINSTANCE) {
        unsafe {
            if !InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_WIN95_CLASSES,
            })
            .as_bool()
            {
                return;
            }
            if let Ok(tooltip) = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                TOOLTIPS_CLASSW,
                None,
                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                0,
                0,
                0,
                0,
                Some(self.hwnd),
                None,
                Some(instance),
                None,
            ) {
                self.input_tooltip = tooltip;
                self.tooltip_text = HSTRING::from(&self.input_name);
                let tool = self.input_tool();
                SendMessageW(
                    tooltip,
                    TTM_ADDTOOLW,
                    None,
                    Some(LPARAM((&raw const tool) as isize)),
                );
            }
        }
    }

    fn input_tool(&self) -> TTTOOLINFOW {
        TTTOOLINFOW {
            cbSize: size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_SUBCLASS,
            hwnd: self.hwnd,
            uId: 1,
            rect: self.tooltip_bounds,
            lpszText: PWSTR(self.tooltip_text.as_ptr() as *mut _),
            ..Default::default()
        }
    }

    fn update_input_tooltip(&mut self, height: i32) {
        if self.input_tooltip.is_invalid() {
            return;
        }
        let bounds = self
            .hitboxes
            .iter()
            .find(|(icon, _)| *icon == BarIcon::Input)
            .map_or(RECT::default(), |(_, range)| RECT {
                left: range.start,
                right: range.end,
                top: 0,
                bottom: height,
            });
        if self.tooltip_bounds != bounds {
            self.tooltip_bounds = bounds;
            let tool = self.input_tool();
            unsafe {
                SendMessageW(
                    self.input_tooltip,
                    TTM_NEWTOOLRECTW,
                    None,
                    Some(LPARAM((&raw const tool) as isize)),
                );
            }
        }
        if self.tooltip_text != self.input_name.as_str() {
            let old = std::mem::replace(&mut self.tooltip_text, HSTRING::from(&self.input_name));
            let tool = self.input_tool();
            unsafe {
                SendMessageW(
                    self.input_tooltip,
                    TTM_UPDATETIPTEXTW,
                    None,
                    Some(LPARAM((&raw const tool) as isize)),
                );
            }
            drop(old);
        }
    }

    fn refresh_network(&mut self) {
        let network = NetworkStatus::read();
        if self.network != network {
            self.network = network;
            self.invalidate();
        }
    }

    fn tick(&mut self) {
        // On clock/status refresh, recover services that were missing at startup or
        // lost an endpoint without delivering a usable device callback.
        if self.audio.as_ref().and_then(Audio::status).is_none() {
            if let Some(audio) = &mut self.audio {
                audio.reconnect();
            } else {
                self.audio = Audio::new(self.hwnd).ok();
            }
        }
        if self.network_changes.is_none() {
            self.network_changes = NetworkChanges::new(self.hwnd);
        }
        self.refresh_network();
        let battery = Power::status();
        if self.battery != battery {
            self.battery = battery;
            self.invalidate();
        }
        self.refresh_volume();
        unsafe {
            let now = GetLocalTime();
            let locale = crate::i18n::locale();
            let mut text = [0; 64];
            let length = GetTimeFormatEx(
                PCWSTR(locale.as_ptr()),
                TIME_NOSECONDS,
                Some(&now),
                PCWSTR::null(),
                Some(&mut text),
            );
            if length > 0 {
                let time = String::from_utf16_lossy(&text[..length as usize - 1]);
                let length = GetDateFormatEx(
                    PCWSTR(locale.as_ptr()),
                    DATE_SHORTDATE,
                    Some(&now),
                    PCWSTR::null(),
                    Some(&mut text),
                    PCWSTR::null(),
                );
                let clock = if length > 0 {
                    let date = String::from_utf16_lossy(&text[..length as usize - 1]);
                    format!("{date}  {time}")
                } else {
                    time
                };
                if self.clock != clock {
                    self.clock = clock;
                    self.invalidate();
                }
            }
            let delay = 60_000 - u32::from(now.wSecond) * 1000 - u32::from(now.wMilliseconds);
            SetTimer(Some(self.hwnd), CLOCK_TIMER, delay.max(10), None);
        }
    }

    fn invalidate(&self) {
        unsafe {
            SetTimer(Some(self.hwnd), HOVER_TIMER, 16, None);
        }
        let _ = unsafe { InvalidateRect(Some(self.hwnd), None, false) };
    }

    fn paint(&mut self) -> Result<()> {
        let mut paint = PAINTSTRUCT::default();
        let mut client = RECT::default();
        unsafe {
            BeginPaint(self.hwnd, &mut paint);
            let _ = EndPaint(self.hwnd, &paint);
            GetClientRect(self.hwnd, &mut client)?;
        }
        if client.right > 0 && client.bottom > 0 {
            let tray = self.tray.as_ref().map(|tray| tray.state());
            let icons: Vec<_> = tray
                .as_ref()
                .map(|state| state.icons().collect())
                .unwrap_or_default();
            self.hitboxes = self.renderer.paint(
                (client.right as u32, client.bottom as u32),
                self.dpi,
                BarContents {
                    active: &self.active,
                    clock: &self.clock,
                    battery: self.battery,
                    connection: self.network.primary(),
                    input: &self.input_text,
                    audio: self.volume,
                    hovered: self.hovered,
                    tray: &icons,
                },
            )?;
            let mut boxes = self.hitboxes.clone();
            boxes.sort_by_key(|(_, range)| range.start);
            self.accessible_keys.clear();
            let entries = boxes
                .into_iter()
                .map(|(icon, range)| {
                    let (key, name) = match icon {
                        BarIcon::System => ("system".into(), "Windows".into()),
                        BarIcon::Active => ("active".into(), self.active.clone()),
                        BarIcon::Network => ("network".into(), self.network.label().into()),
                        BarIcon::Input => (
                            "input".into(),
                            crate::tr!("Keyboard layout: {layout}", layout = self.input_name),
                        ),
                        BarIcon::Battery => ("battery".into(), self.battery.label()),
                        BarIcon::Volume => ("volume".into(), crate::i18n::text("Volume").into()),
                        BarIcon::Clock => ("clock".into(), self.clock.clone()),
                        BarIcon::QuickSettings => {
                            ("quick".into(), crate::i18n::text("Quick Settings").into())
                        }
                        BarIcon::Tray(index) => icons.get(index).map_or_else(
                            || {
                                (
                                    format!("tray:{index}"),
                                    crate::i18n::text("App tray icon").into(),
                                )
                            },
                            |icon| {
                                (
                                    format!("tray:{:?}", icon.menu_key()),
                                    if icon.title.is_empty() {
                                        crate::i18n::text("App tray icon").into()
                                    } else {
                                        icon.title.clone()
                                    },
                                )
                            },
                        ),
                    };
                    self.accessible_keys.push((key.clone(), icon));
                    crate::ui::accessibility::Entry {
                        key,
                        name,
                        bounds: RECT {
                            left: range.start,
                            top: 0,
                            right: range.end,
                            bottom: client.bottom,
                        },
                    }
                })
                .collect();
            self.accessibility.update(entries)?;
            drop(tray);
            self.update_input_tooltip(client.bottom);
        }
        Ok(())
    }

    fn battery_menu(&mut self, anchor: i32) -> Result<Option<Action>> {
        let battery = Power::status();
        let active = PowerPlan::active().ok();
        let plans = PowerPlan::list().unwrap_or_default();
        let mut menu = Popup::new(self.hwnd, 252)?;
        menu.heading(&battery.label())?;
        if let Some(source) = battery.power_source() {
            menu.detail(source)?;
        }
        menu.divider()?;
        menu.detail(if plans.is_empty() {
            crate::i18n::text("Power plans unavailable")
        } else {
            crate::i18n::text("Power plan")
        })?;
        for (index, plan) in plans.iter().enumerate() {
            menu.option(index as u32 + 2, &plan.name, true, plan.is_active(active))?;
        }
        menu.divider()?;
        menu.action(1, crate::i18n::text("Power and Battery Settings…"))?;
        match self.track_menu(&menu, anchor)? {
            0 => {}
            1 => return Ok(Some(Action::WindowsSettings("ms-settings:powersleep"))),
            choice => {
                if let Some(plan) = plans.get(choice as usize - 2)
                    && !plan.is_active(PowerPlan::active().ok())
                {
                    plan.activate().map_err(|error| {
                        Error::new(
                            error.code(),
                            crate::tr!(
                                "Could not switch to {name}.\n\n{error}",
                                name = plan.name,
                                error = error
                            ),
                        )
                    })?;
                }
            }
        }
        Ok(None)
    }

    fn active_menu(&mut self) -> Result<Option<Action>> {
        let target = self.active_window;
        let mut menu = Popup::new(self.hwnd, 238)?;
        menu.heading(&self.active)?;
        menu.option(1, crate::i18n::text("End task"), target.is_some(), false)?;
        let choice = menu.track(
            self.hwnd,
            POINT {
                x: self.bounds.left + scale(43, self.dpi),
                y: self.bounds.bottom + scale(3, self.dpi),
            },
            TPM_LEFTALIGN,
        );
        if choice == 1
            && let Some((hwnd, pid)) = target
        {
            crate::system::end_task(hwnd, pid)?;
        }
        Ok(None)
    }

    fn system_menu(&mut self) -> Result<Option<Action>> {
        let mut menu = Popup::new(self.hwnd, 238)?;
        menu.heading(crate::i18n::text("Windows"))?;
        let actions = [
            SystemAction::About,
            SystemAction::TaskManager,
            SystemAction::Store,
            SystemAction::Sleep,
            SystemAction::Restart,
            SystemAction::Shutdown,
            SystemAction::Lock,
            SystemAction::LogOut,
        ];
        for (index, action) in actions.iter().enumerate() {
            if index == 3 || index == 6 {
                menu.divider()?;
            }
            menu.option(
                index as u32 + 1,
                action.label(),
                !matches!(action, SystemAction::Sleep) || system::sleep_available(),
                false,
            )?;
        }
        let choice = menu.track(
            self.hwnd,
            POINT {
                x: self.bounds.left + scale(7, self.dpi),
                y: self.bounds.bottom + scale(3, self.dpi),
            },
            TPM_LEFTALIGN,
        );
        Ok(choice
            .checked_sub(1)
            .and_then(|index| actions.get(index as usize))
            .copied()
            .map(Action::System))
    }

    fn volume_rows(&self, menu: &mut Popup) -> Result<()> {
        let enabled = self.volume.is_some();
        menu.slider(
            if self.volume.is_some_and(|(_, muted)| muted) {
                crate::i18n::text("Volume muted")
            } else if enabled {
                crate::i18n::text("Volume")
            } else {
                crate::i18n::text("Volume unavailable")
            },
            self.volume.map_or(0.0, |(level, _)| level),
            enabled,
        )?;
        let mute = if self.volume.is_some_and(|(_, muted)| muted) {
            crate::i18n::text("Unmute")
        } else {
            crate::i18n::text("Mute")
        };
        menu.option(3, mute, enabled, false)
    }

    fn apply_volume(&mut self, choice: u32) -> Result<()> {
        if choice == 3
            && let Some(audio) = &self.audio
        {
            audio.toggle_mute()?;
        }
        self.refresh_volume();
        Ok(())
    }

    fn volume_menu(&mut self, anchor: i32) -> Result<Option<Action>> {
        self.refresh_volume();
        let mut menu = Popup::new(self.hwnd, 280)?;
        menu.heading(crate::i18n::text("Sound"))?;
        menu.action(7, crate::i18n::text("Choose sound output…"))?;
        self.volume_rows(&mut menu)?;
        menu.divider()?;
        menu.action(6, crate::i18n::text("Sound Settings…"))?;
        let choice = self.track_menu(&menu, anchor)?;
        self.apply_volume(choice)?;
        Ok(match choice {
            6 => Some(Action::WindowsSettings("ms-settings:sound")),
            7 => Some(Action::System(SystemAction::SoundOutput)),
            _ => None,
        })
    }

    fn input_menu(&mut self, anchor: i32) -> Result<Option<Action>> {
        self.refresh_input();
        let target = self.input_target;
        let active = target.active();
        let mut menu = Popup::new(self.hwnd, 300)?;
        menu.heading(crate::i18n::text("Keyboard layout"))?;
        let layouts = input_layout::available(active);
        for (index, &layout) in layouts.iter().enumerate() {
            let name = input_layout::layout_name(layout);
            menu.option(
                index as u32 + 2,
                &input_layout::label(layout, name.as_deref()),
                true,
                layout == active,
            )?;
        }
        menu.divider()?;
        menu.action(1, crate::i18n::text("Language settings…"))?;
        let choice = self.track_menu(&menu, anchor)?;
        if choice == 1 {
            return Ok(Some(Action::WindowsSettings("ms-settings:regionlanguage")));
        }
        if let Some(index) = choice.checked_sub(2)
            && let Some(&layout) = layouts.get(index as usize)
            && layout != active
        {
            target.select(layout)?;
        }
        Ok(None)
    }

    fn network_menu(&mut self, anchor: i32) -> Result<Option<Action>> {
        crate::platform::hotspot::refresh(self.hwnd);
        panels::refresh_radios(self.hwnd);
        let (choice, rows) = loop {
            let (menu, rows) = self.network_panel()?;
            let choice = self.track_menu(&menu, anchor)?;
            if choice != crate::popup::REFRESH_PANEL {
                break (choice, rows);
            }
        };
        if let Some(index) = choice.checked_sub(100)
            && let Some(network) = rows.get(index as usize)
        {
            if network.connected {
                return Ok(None);
            }
            if !network.profile.is_empty() {
                panels::connect(network)?;
                return Ok(None);
            }
            return Ok(Some(Action::WindowsSettings("ms-availablenetworks:")));
        }
        Ok(match choice {
            1 => Some(Action::System(SystemAction::ToggleWifi)),
            2 => Some(Action::WindowsSettings("ms-availablenetworks:")),
            3 => Some(Action::WindowsSettings("ms-settings:network-wifi")),
            4 => Some(Action::WindowsSettings("ms-settings:network-ethernet")),
            5 => Some(Action::WindowsSettings("ms-settings:network-cellular")),
            6 => Some(Action::WindowsSettings("ms-settings:network")),
            91 => Some(Action::System(SystemAction::ToggleHotspot)),
            92 => Some(Action::WindowsSettings("ms-settings:network-mobilehotspot")),
            _ => None,
        })
    }

    fn network_panel(&mut self) -> Result<(Popup, Vec<panels::Network>)> {
        self.refresh_network();
        let mut menu = Popup::new(self.hwnd, 310)?;
        menu.refresh_on(crate::platform::hotspot::WM_HOTSPOT);
        menu.heading(crate::i18n::text("Network"))?;
        if matches!(self.network.primary(), None | Some(Connection::Other)) {
            menu.detail(self.network.label())?;
        }
        if let Some(connected) = self.network.ethernet {
            menu.tile(
                4,
                Symbol::Ethernet,
                crate::i18n::text("Ethernet"),
                crate::i18n::text(if connected {
                    "Connected"
                } else {
                    "Disconnected"
                }),
                true,
                false,
            )?;
        }
        if let Some(connected) = self.network.cellular {
            menu.tile(
                5,
                Symbol::Cellular,
                crate::i18n::text("Cellular"),
                crate::i18n::text(if connected {
                    "Connected"
                } else {
                    "Disconnected"
                }),
                true,
                false,
            )?;
        }
        self.hotspot_rows(&mut menu)?;
        menu.divider()?;
        menu.toggle(1, crate::i18n::text("Wi-Fi"), panels::radio_state(false))?;
        let wifi_available = self.network.wifi.is_some() || panels::radio_state(false).is_some();
        let networks = wifi_available.then(panels::networks);
        let mut rows = Vec::new();
        match networks {
            Some(Ok(mut networks)) => {
                networks.truncate(5);
                rows = networks;
                menu.detail(if rows.is_empty() {
                    crate::i18n::text("No networks found")
                } else {
                    crate::i18n::text("Networks")
                })?;
                for (index, network) in rows.iter().enumerate() {
                    let status = if network.connected {
                        crate::i18n::text("Connected")
                    } else if !network.profile.is_empty() {
                        crate::i18n::text("Saved")
                    } else {
                        crate::i18n::text("Available")
                    };
                    let security = if network.secure {
                        crate::i18n::text("Secured")
                    } else {
                        crate::i18n::text("Open")
                    };
                    menu.tile(
                        index as u32 + 100,
                        Symbol::Wifi,
                        &network.name,
                        &format!("{status} · {security} · {}%", network.signal),
                        !network.connected,
                        network.connected,
                    )?;
                }
            }
            Some(Err(_)) => {
                menu.detail(crate::i18n::text("Network list unavailable in this app"))?;
                menu.detail(crate::i18n::text("View networks in Windows below"))?;
            }
            None => menu.detail(crate::i18n::text("Wi-Fi unavailable"))?,
        }
        if wifi_available {
            if !rows.iter().any(|network| network.connected)
                && let Some(connected) = self.network.wifi
            {
                menu.detail(crate::i18n::text(if connected {
                    "Wi-Fi connected"
                } else {
                    "Wi-Fi disconnected"
                }))?;
            }
            menu.action(2, crate::i18n::text("Other networks…"))?;
            menu.action(3, crate::i18n::text("Wi-Fi settings…"))?;
        }
        menu.divider()?;
        menu.action(6, crate::i18n::text("Network settings…"))?;
        Ok((menu, rows))
    }

    fn bluetooth_menu(&mut self, anchor: i32) -> Result<Option<Action>> {
        panels::refresh_radios(self.hwnd);
        let mut menu = Popup::new(self.hwnd, 350)?;
        menu.heading(crate::i18n::text("Bluetooth"))?;
        menu.toggle(1, crate::i18n::text("Bluetooth"), panels::radio_state(true))?;
        menu.divider()?;
        let devices = panels::bluetooth_devices();
        menu.detail(if devices.is_empty() {
            crate::i18n::text("No cached Bluetooth devices")
        } else {
            crate::i18n::text("Your devices")
        })?;
        for (index, (name, connected)) in devices.iter().take(5).enumerate() {
            menu.tile(
                index as u32 + 100,
                Symbol::Bluetooth,
                name,
                if *connected {
                    crate::i18n::text("Connected")
                } else {
                    crate::i18n::text("Paired")
                },
                false,
                *connected,
            )?;
        }
        menu.divider()?;
        menu.action(2, crate::i18n::text("Add or manage devices…"))?;
        let choice = self.track_menu(&menu, anchor)?;
        Ok(match choice {
            1 => Some(Action::System(SystemAction::ToggleBluetooth)),
            2 => Some(Action::WindowsSettings("ms-settings:bluetooth")),
            _ => None,
        })
    }

    fn display_menu(&mut self, anchor: i32) -> Result<Option<Action>> {
        let brightness = panels::Brightness::for_monitor(self.monitor);
        let screen = monitor_info(self.monitor)?.monitorInfo.rcMonitor;
        let mut menu = Popup::new(self.hwnd, 350)?;
        menu.heading(crate::i18n::text("Display"))?;
        menu.detail(&crate::tr!(
            "{width} × {height} · {scale}% scaling",
            width = screen.right - screen.left,
            height = screen.bottom - screen.top,
            scale = (self.dpi * 100 + 48) / 96
        ))?;
        if let Some(brightness) = &brightness {
            menu.slider(crate::i18n::text("Brightness"), brightness.level, true)?;
        } else {
            menu.detail(crate::i18n::text(
                "Brightness is managed by Windows or your monitor",
            ))?;
        }
        menu.divider()?;
        menu.tile(
            10,
            Symbol::Appearance,
            crate::i18n::text("Color mode"),
            if crate::theme::dark() {
                crate::i18n::text("Dark · change in dock settings")
            } else {
                crate::i18n::text("Light · change in dock settings")
            },
            true,
            false,
        )?;
        menu.tile(
            11,
            Symbol::Moon,
            crate::i18n::text("Night light"),
            crate::i18n::text("Open Windows night light controls"),
            true,
            false,
        )?;
        menu.divider()?;
        menu.action(12, crate::i18n::text("Display settings…"))?;
        let choice = menu.track_with_slider(
            self.hwnd,
            POINT {
                x: self.bounds.left + anchor,
                y: self.bounds.bottom + scale(3, self.dpi),
            },
            TPM_RIGHTALIGN,
            &|level| {
                if let Some(brightness) = &brightness {
                    brightness.set(level)?;
                }
                Ok(())
            },
        )?;
        Ok(match choice {
            10 => Some(Action::DockSettings),
            11 => Some(Action::WindowsSettings("ms-settings:nightlight")),
            12 => Some(Action::WindowsSettings("ms-settings:display")),
            _ => None,
        })
    }

    pub fn keyboard_menu(&mut self) -> Result<Option<Action>> {
        self.quick_settings(self.bounds.right - self.bounds.left - scale(14, self.dpi))
    }

    fn tray_menu(&mut self, anchor: i32) -> Result<Option<Action>> {
        let mut menu = Popup::new(self.hwnd, 300)?;
        menu.heading(crate::i18n::text("All app tray icons"))?;
        // Do not hold a RefCell borrow while native popup loops dispatch tray IPC.
        let icons: Vec<_> = self
            .tray
            .as_ref()
            .map(|tray| {
                tray.state()
                    .icons()
                    .map(|icon| (icon.menu_key(), icon.title.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if icons.is_empty() {
            menu.detail(crate::i18n::text("No app tray icons available"))?;
        }
        for (index, icon) in icons.iter().enumerate() {
            menu.action(
                index as u32 + 1,
                if icon.1.is_empty() {
                    crate::i18n::text("App tray icon")
                } else {
                    &icon.1
                },
            )?;
        }
        let choice = menu.track(
            self.hwnd,
            POINT {
                x: self.bounds.left + anchor,
                y: self.bounds.bottom,
            },
            TPM_RIGHTALIGN,
        );
        if let Some(icon) = choice
            .checked_sub(1)
            .and_then(|index| icons.get(index as usize))
        {
            let mut action = Popup::new(self.hwnd, 220)?;
            action.action(1, crate::i18n::text("Open"))?;
            action.action(2, crate::i18n::text("Show app menu"))?;
            let point = POINT {
                x: self.bounds.left + anchor,
                y: self.bounds.bottom,
            };
            let choice = action.track(self.hwnd, point, TPM_RIGHTALIGN);
            if let Some(tray) = &self.tray {
                if let Some(current) = tray
                    .state()
                    .icons()
                    .find(|current| current.menu_key() == icon.0)
                {
                    match choice {
                        1 => current.click(WM_LBUTTONUP, point),
                        2 => current.click(WM_RBUTTONUP, point),
                        _ => {}
                    }
                }
            }
        }
        Ok(None)
    }

    fn quick_settings(&mut self, anchor: i32) -> Result<Option<Action>> {
        self.refresh_network();
        self.refresh_volume();
        panels::refresh_radios(self.hwnd);
        let mut menu = Popup::new(self.hwnd, 300)?;
        menu.heading(crate::i18n::text("Quick Settings"))?;
        let connection = self.network.primary();
        let networks = if connection == Some(Connection::Wifi) {
            panels::networks().unwrap_or_default()
        } else {
            Vec::new()
        };
        let network_name = networks
            .iter()
            .find(|network| network.connected)
            .map_or_else(
                || {
                    if connection.is_some() {
                        crate::i18n::text("Connected")
                    } else {
                        self.network.label()
                    }
                },
                |network| network.name.as_str(),
            );
        menu.tile(
            10,
            connection.unwrap_or(Connection::Other).symbol(),
            connection.unwrap_or(Connection::Other).name(),
            network_name,
            true,
            connection.is_some(),
        )?;
        let bluetooth = panels::radio_state(true);
        menu.tile(
            11,
            Symbol::Bluetooth,
            crate::i18n::text("Bluetooth"),
            match bluetooth {
                Some(true) => crate::i18n::text("On"),
                Some(false) => crate::i18n::text("Off"),
                None => crate::i18n::text("Manage devices"),
            },
            true,
            bluetooth == Some(true),
        )?;
        menu.divider()?;
        let brightness = panels::Brightness::for_monitor(self.monitor);
        if let Some(brightness) = &brightness {
            menu.slider_for(
                BRIGHTNESS_SLIDER,
                crate::i18n::text("Brightness"),
                brightness.level,
                true,
            )?;
        }
        self.volume_rows(&mut menu)?;
        menu.divider()?;
        menu.card(
            13,
            Symbol::Moon,
            crate::i18n::text("Focus"),
            crate::i18n::text("Windows"),
            false,
        )?;
        menu.card(
            14,
            Symbol::Battery,
            crate::i18n::text("Power & battery"),
            &self.battery.label(),
            false,
        )?;
        menu.divider()?;
        menu.action(12, crate::i18n::text("Display options…"))?;
        menu.action(15, crate::i18n::text("Dock settings"))?;
        if self.tray.is_some() {
            menu.action(90, crate::i18n::text("All app tray icons…"))?;
        }
        let choice = self.track_controls(&menu, anchor, brightness.as_ref())?;
        self.apply_volume(choice)?;
        match choice {
            10 => self.network_menu(anchor),
            90 => self.tray_menu(anchor),
            11 => self.bluetooth_menu(anchor),
            12 => self.display_menu(anchor),
            13 => Ok(Some(Action::WindowsSettings("ms-settings:quiethours"))),
            14 => self.battery_menu(anchor),
            15 => Ok(Some(Action::DockSettings)),
            _ => Ok(None),
        }
    }

    fn hotspot_rows(&self, menu: &mut Popup) -> Result<()> {
        let state = crate::platform::hotspot::status();
        menu.divider()?;
        menu.toggle(
            91,
            crate::i18n::text("Mobile hotspot"),
            if state.busy { None } else { state.on },
        )?;
        menu.detail(crate::i18n::text(&state.detail))?;
        menu.action(92, crate::i18n::text("Mobile hotspot settings…"))
    }

    fn track_menu(&mut self, menu: &Popup, anchor: i32) -> Result<u32> {
        self.track_controls(menu, anchor, None)
    }

    fn track_controls(
        &mut self,
        menu: &Popup,
        anchor: i32,
        brightness: Option<&panels::Brightness>,
    ) -> Result<u32> {
        let choice = menu.track_with_sliders(
            self.hwnd,
            POINT {
                x: self.bounds.left + anchor,
                y: self.bounds.bottom + scale(3, self.dpi),
            },
            TPM_RIGHTALIGN,
            &|id, level| {
                if id == BRIGHTNESS_SLIDER {
                    if let Some(brightness) = brightness {
                        brightness.set(level)?;
                    }
                } else if let Some(audio) = &self.audio {
                    audio.set_volume(level)?;
                }
                Ok(())
            },
        );
        self.battery = Power::status();
        self.refresh_volume();
        self.tick();
        choice
    }
}

impl Drop for TopBar {
    fn drop(&mut self) {
        crate::platform::hotspot::detach(self.hwnd);
        drop(self.tray.take());
        drop(self.appbar.take());
        drop(self.audio.take());
        drop(self.network_changes.take());
        drop(self.power.take());
        unsafe {
            let _ = KillTimer(Some(self.hwnd), CLOCK_TIMER);
            let _ = KillTimer(Some(self.hwnd), INPUT_TIMER);
            if !self.input_tooltip.is_invalid() {
                let _ = DestroyWindow(self.input_tooltip);
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let notification = match message {
        WM_ACTIVATE if wparam.0 & 0xffff != WA_INACTIVE as usize => {
            crate::ui::accessibility::activated(hwnd, HWND(lparam.0 as *mut _));
            return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
        }
        WM_COMMAND if crate::ui::accessibility::command(hwnd, wparam, lparam) => return LRESULT(0),
        WM_DRAWITEM if crate::ui::accessibility::owner_draw(hwnd, wparam) => return LRESULT(1),
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_CLOSE => {
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        WM_APPBAR if wparam.0 as u32 == ABN_POSCHANGED => WM_REPOSITION,
        WM_DPICHANGED => WM_REPOSITION,
        WM_POWERBROADCAST => {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_POWER_STATUS, WPARAM(0), LPARAM(0));
                if matches!(
                    wparam.0 as u32,
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND | PBT_APMRESUMECRITICAL
                ) {
                    let _ = PostMessageW(Some(hwnd), WM_CLOCK_CHANGED, WPARAM(0), LPARAM(0));
                }
            }
            return LRESULT(1);
        }
        WM_SETTINGCHANGE | WM_TIMECHANGE => WM_CLOCK_CHANGED,
        _ => return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    };
    let _ = unsafe { PostMessageW(Some(hwnd), notification, WPARAM(0), LPARAM(0)) };
    LRESULT(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;

    #[test]
    fn stale_monitor_notification_recovers_without_losing_reservation_or_focus() -> Result<()> {
        let instance = unsafe { GetModuleHandleW(None)? }.into();
        let monitor = unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) };
        let foreground = unsafe { GetForegroundWindow() };
        let mut bar = TopBar::new(instance, monitor, 0.36, false)?;
        bar.set_visible(true)?;
        bar.paint()?;
        assert!(
            bar.accessible_keys
                .iter()
                .any(|(key, icon)| { key == "input" && *icon == BarIcon::Input })
        );
        assert!(!bar.input_layout.is_invalid());
        // An invalidated display handle must fall back before its paired dock
        // window has received and processed WM_DISPLAYCHANGE.
        bar.update(HMONITOR::default())?;
        assert!(!bar.monitor.is_invalid());
        assert!(monitor_info(bar.monitor)?.monitorInfo.rcWork.top >= bar.bounds.bottom);
        bar.resume();
        let current = unsafe { GetForegroundWindow() };
        // Another application may change focus during this desktop test. The
        // invariant is that showing/recovering our bar never activates it.
        assert!(
            current != bar.hwnd && !unsafe { IsChild(bar.hwnd, current).as_bool() },
            "status bar stole focus from {foreground:?}"
        );
        Ok(())
    }
}
