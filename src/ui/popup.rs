use std::cell::{Cell, RefCell};
use std::time::Instant;

use crate::icons::{self, Symbol, VolumeLevel};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, Result, w};

pub const UI_FONT: PCWSTR = w!("Segoe UI Variable Text");
pub const MENU_SURFACE: COLORREF = rgb(0x192437);
pub const MENU_BORDER: COLORREF = rgb(0x41536f);
const SS_OWNERDRAW: u32 = 0x000d;
const SS_NOPREFIX: u32 = 0x0080;
const TBM_GETPOS: u32 = WM_USER;

pub const PANEL: COLORREF = rgb(0xf7f8fb);
pub const SIDEBAR: COLORREF = rgb(0xeef1f6);
pub const INK: COLORREF = rgb(0x1c2739);
pub const MUTED: COLORREF = rgb(0x526078);
pub const LINE: COLORREF = rgb(0xdbe1eb);
pub const ACCENT: COLORREF = rgb(0x235bc3);
pub const SELECTED: COLORREF = rgb(0xdce8fd);

pub const fn rgb(hex: u32) -> COLORREF {
    COLORREF(((hex & 0xff) << 16) | (hex & 0xff00) | ((hex >> 16) & 0xff))
}

pub fn adapted(color: COLORREF) -> COLORREF {
    if high_contrast() {
        return color;
    }
    let raw = ((color.0 & 0xff) << 16) | (color.0 & 0xff00) | ((color.0 >> 16) & 0xff);
    let settings = crate::theme::settings(raw);
    rgb(if settings != raw {
        settings
    } else {
        crate::theme::ui(raw)
    })
}
pub fn high_contrast() -> bool {
    let mut contrast = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            Some((&raw mut contrast).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && contrast.dwFlags.0 & HCF_HIGHCONTRASTON.0 != 0
    }
}

pub fn scale(dip: i32, dpi: i32) -> i32 {
    (dip * dpi + 48) / 96
}

pub struct Font(pub HFONT);

impl Font {
    pub fn new(dip: i32, dpi: i32, weight: FONT_WEIGHT) -> Self {
        Self(unsafe {
            CreateFontW(
                -scale(dip, dpi),
                0,
                0,
                0,
                weight.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                UI_FONT,
            )
        })
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(self.0.into()) };
    }
}

pub fn fill(dc: HDC, rect: &RECT, color: COLORREF) {
    unsafe {
        SetDCBrushColor(dc, adapted(color));
        FillRect(dc, rect, HBRUSH(GetStockObject(DC_BRUSH).0));
    }
}

pub fn rounded(dc: HDC, rect: &RECT, color: COLORREF, diameter: i32) {
    unsafe {
        SetDCBrushColor(dc, adapted(color));
        let brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        let pen = SelectObject(dc, GetStockObject(NULL_PEN));
        let _ = RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            diameter,
            diameter,
        );
        SelectObject(dc, pen);
        SelectObject(dc, brush);
    }
}

pub fn draw_text(dc: HDC, value: &str, rect: &mut RECT, font: HFONT, color: COLORREF) {
    unsafe {
        let font = if font.is_invalid() {
            GetStockObject(DEFAULT_GUI_FONT)
        } else {
            font.into()
        };
        let previous = SelectObject(dc, font);
        SetTextColor(dc, adapted(color));
        SetBkMode(dc, TRANSPARENT);
        DrawTextW(
            dc,
            &mut value.encode_utf16().collect::<Vec<_>>(),
            rect,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Heading,
    Detail,
    Action,
    Slider,
    Tile,
    Card,
    Toggle,
    Divider,
}

impl Kind {
    fn height(self) -> i32 {
        match self {
            Self::Heading => 32,
            Self::Detail => 23,
            Self::Action => 34,
            Self::Slider => 62,
            Self::Tile => 66,
            Self::Card => 94,
            Self::Toggle => 46,
            Self::Divider => 13,
        }
    }
}

struct Row {
    id: u32,
    label: String,
    kind: Kind,
    enabled: bool,
    checked: bool,
    symbol: Option<Symbol>,
    subtitle: String,
}

pub struct Popup {
    handle: HMENU,
    dpi: i32,
    width: i32,
    rows: Vec<Row>,
    slider_level: f32,
}

impl Popup {
    pub fn new(owner: HWND, width: i32) -> Result<Self> {
        Ok(Self {
            handle: unsafe { CreatePopupMenu()? },
            dpi: (unsafe { GetDpiForWindow(owner) } as i32).max(1),
            width,
            rows: Vec::new(),
            slider_level: 0.0,
        })
    }

    pub fn heading(&mut self, label: &str) -> Result<()> {
        self.add(0, label, Kind::Heading, false, false)
    }

    pub fn detail(&mut self, label: &str) -> Result<()> {
        self.add(0, label, Kind::Detail, false, false)
    }

    pub fn action(&mut self, id: u32, label: &str) -> Result<()> {
        self.option(id, label, true, false)
    }

    pub fn option(&mut self, id: u32, label: &str, enabled: bool, checked: bool) -> Result<()> {
        self.add(id, label, Kind::Action, enabled, checked)
    }

    pub fn tile(
        &mut self,
        id: u32,
        symbol: Symbol,
        label: &str,
        subtitle: &str,
        enabled: bool,
        checked: bool,
    ) -> Result<()> {
        self.add(id, label, Kind::Tile, enabled, checked)?;
        if let Some(row) = self.rows.last_mut() {
            row.symbol = Some(symbol);
            row.subtitle = subtitle.into();
        }
        Ok(())
    }
    pub fn card(
        &mut self,
        id: u32,
        symbol: Symbol,
        label: &str,
        subtitle: &str,
        checked: bool,
    ) -> Result<()> {
        self.tile(id, symbol, label, subtitle, true, checked)?;
        self.rows.last_mut().unwrap().kind = Kind::Card;
        Ok(())
    }
    pub fn toggle(&mut self, id: u32, label: &str, on: Option<bool>) -> Result<()> {
        self.add(id, label, Kind::Toggle, on.is_some(), on == Some(true))
    }
    fn card_right(&self, index: usize) -> bool {
        self.rows[..index]
            .iter()
            .rev()
            .take_while(|r| r.kind == Kind::Card)
            .count()
            % 2
            == 1
    }
    fn row_height(&self, index: usize) -> i32 {
        let row = &self.rows[index];
        if row.kind == Kind::Card
            && !self.card_right(index)
            && self
                .rows
                .get(index + 1)
                .is_some_and(|r| r.kind == Kind::Card)
        {
            0
        } else {
            row.kind.height()
        }
    }
    pub fn divider(&mut self) -> Result<()> {
        self.add(0, "", Kind::Divider, false, false)
    }

    pub fn slider(&mut self, label: &str, level: f32, enabled: bool) -> Result<()> {
        self.slider_level = level.clamp(0.0, 1.0);
        self.add(0, label, Kind::Slider, enabled, false)
    }

    pub fn track_with_slider(
        &self,
        owner: HWND,
        point: POINT,
        alignment: TRACK_POPUP_MENU_FLAGS,
        on_slide: &dyn Fn(f32) -> Result<()>,
    ) -> Result<u32> {
        if self.rows.iter().any(|row| row.kind == Kind::Slider) {
            self.show_panel(owner, point, alignment, Some(on_slide))
        } else {
            Ok(self.track(owner, point, alignment))
        }
    }

    fn add(
        &mut self,
        id: u32,
        label: &str,
        kind: Kind,
        enabled: bool,
        checked: bool,
    ) -> Result<()> {
        let flags = if kind == Kind::Divider {
            MF_SEPARATOR
        } else {
            MF_STRING
                | if enabled { MF_ENABLED } else { MF_GRAYED }
                | if checked { MF_CHECKED } else { MF_UNCHECKED }
        };
        let text = HSTRING::from(label.replace('&', "&&"));
        unsafe { AppendMenuW(self.handle, flags, id as usize, &text)? };
        self.rows.push(Row {
            id,
            label: label.into(),
            kind,
            enabled,
            checked,
            symbol: None,
            subtitle: String::new(),
        });
        Ok(())
    }

    pub fn track(&self, owner: HWND, point: POINT, alignment: TRACK_POPUP_MENU_FLAGS) -> u32 {
        if !high_contrast()
            && let Ok(choice) = self.show_panel(owner, point, alignment, None)
        {
            return choice;
        }
        unsafe {
            let foreground = GetForegroundWindow();
            let _ = SetForegroundWindow(owner);
            let choice = TrackPopupMenu(
                self.handle,
                TPM_RETURNCMD | TPM_NONOTIFY | alignment,
                point.x,
                point.y,
                None,
                owner,
                None,
            )
            .0 as u32;
            if GetForegroundWindow() == owner && !foreground.0.is_null() {
                let _ = SetForegroundWindow(foreground);
            }
            choice
        }
    }

    fn show_panel(
        &self,
        owner: HWND,
        point: POINT,
        alignment: TRACK_POPUP_MENU_FLAGS,
        on_slide: Option<&dyn Fn(f32) -> Result<()>>,
    ) -> Result<u32> {
        let px = |dip| scale(dip, self.dpi);
        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        let work = crate::placement::monitor_info(monitor)?.monitorInfo.rcWork;
        let width = px(self.width.max(220) + 20).min(work.right - work.left);
        let height = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, _)| px(self.row_height(index)))
            .sum::<i32>()
            + px(20);
        if height > work.bottom - work.top || width < px(120) {
            return Err(windows::core::Error::empty());
        }
        let x = if alignment.contains(TPM_RIGHTALIGN) {
            point.x - width
        } else {
            point.x
        };
        let y = if alignment.contains(TPM_BOTTOMALIGN) {
            point.y - height
        } else {
            point.y
        };
        let x = x.clamp(work.left, work.right - width);
        let y = y.clamp(work.top, work.bottom - height);
        unsafe {
            let instance = GetModuleHandleW(None)?.into();
            let class = WNDCLASSW {
                style: CS_DROPSHADOW,
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpfnWndProc: Some(panel_proc),
                lpszClassName: w!("StyledDockMenuPanel"),
                ..Default::default()
            };
            // The same class is reused for subsequent panels.
            RegisterClassW(&class);
            let panel = Panel {
                popup: self,
                fonts: [
                    Font::new(19, self.dpi, FW_SEMIBOLD),
                    Font::new(13, self.dpi, FW_NORMAL),
                    Font::new(14, self.dpi, FW_MEDIUM),
                ],
                result: Cell::new(None),
                hot: Cell::new(HWND::default()),
                slider: Cell::new(HWND::default()),
                slider_label: Cell::new(HWND::default()),
                slider_value: Cell::new((self.slider_level * 100.0).round() as u32),
                on_slide,
                error: RefCell::new(None),
            };
            let previous = GetForegroundWindow();
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED,
                class.lpszClassName,
                w!("Styled Dock menu"),
                WS_POPUP | WS_CLIPCHILDREN,
                x,
                y,
                width,
                height,
                Some(owner),
                None,
                Some(instance),
                Some((&raw const panel).cast()),
            )?;
            let window = PanelWindow(hwnd);
            let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, px(28), px(28));
            if region.is_invalid() || SetWindowRgn(hwnd, Some(region), false) == 0 {
                let _ = DeleteObject(region.into());
                return Err(windows::core::Error::from_thread());
            }
            let mut actions = Vec::new();
            let mut top = px(10);
            for (index, row) in self.rows.iter().enumerate() {
                let height = px(row.kind.height());
                if row.kind != Kind::Divider {
                    let (class, style) = if matches!(
                        row.kind,
                        Kind::Action | Kind::Tile | Kind::Card | Kind::Toggle
                    ) {
                        (w!("BUTTON"), WINDOW_STYLE(BS_OWNERDRAW as u32) | WS_TABSTOP)
                    } else {
                        (w!("STATIC"), WINDOW_STYLE(SS_OWNERDRAW | SS_NOPREFIX))
                    };
                    let name = if row.checked {
                        format!("{} (active)", row.label)
                    } else {
                        row.label.clone()
                    };
                    let child = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        class,
                        &HSTRING::from(name),
                        WS_CHILD
                            | WS_VISIBLE
                            | style
                            | if matches!(
                                row.kind,
                                Kind::Action | Kind::Tile | Kind::Card | Kind::Toggle
                            ) && !row.enabled
                            {
                                WS_DISABLED
                            } else {
                                WINDOW_STYLE(0)
                            },
                        if row.kind == Kind::Card && self.card_right(index) {
                            px(24) + (width - px(36)) / 2
                        } else if row.kind == Kind::Card {
                            px(12)
                        } else {
                            px(7)
                        },
                        top,
                        if row.kind == Kind::Card {
                            (width - px(36)) / 2
                        } else {
                            width - px(14)
                        },
                        if row.kind == Kind::Slider {
                            px(23)
                        } else {
                            height
                        },
                        Some(hwnd),
                        Some(HMENU((index + 1) as *mut _)),
                        Some(instance),
                        None,
                    )?;
                    if row.kind == Kind::Slider {
                        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                            dwICC: ICC_BAR_CLASSES,
                        })
                        .ok()?;
                        let slider = CreateWindowExW(
                            WINDOW_EX_STYLE(0),
                            TRACKBAR_CLASSW,
                            &HSTRING::from(&row.label),
                            WS_CHILD
                                | WS_VISIBLE
                                | WS_TABSTOP
                                | WINDOW_STYLE(TBS_NOTICKS | TBS_HORZ)
                                | if row.enabled {
                                    WINDOW_STYLE(0)
                                } else {
                                    WS_DISABLED
                                },
                            px(13),
                            top + px(24),
                            width - px(26),
                            px(34),
                            Some(hwnd),
                            Some(HMENU(0xf001usize as *mut _)),
                            Some(instance),
                            None,
                        )?;
                        SendMessageW(
                            slider,
                            TBM_SETRANGE,
                            Some(WPARAM(0)),
                            Some(LPARAM(100 << 16)),
                        );
                        SendMessageW(slider, TBM_SETPAGESIZE, Some(WPARAM(0)), Some(LPARAM(5)));
                        SendMessageW(
                            slider,
                            TBM_SETPOS,
                            Some(WPARAM(1)),
                            Some(LPARAM(panel.slider_value.get() as isize)),
                        );
                        panel.slider.set(slider);
                        SetWindowSubclass(
                            slider,
                            Some(slider_proc),
                            1,
                            (&raw const panel) as usize,
                        )
                        .ok()?;
                        panel.slider_label.set(child);
                        if row.enabled {
                            actions.push(slider);
                        }
                    } else if row.enabled {
                        actions.push(child);
                    }
                }
                top += px(self.row_height(index));
            }
            const FADE_TIMER: usize = 0xface;
            let animate = !high_contrast();
            let opened = Instant::now();
            let mut closing: Option<Instant> = None;
            let mut current_opacity = 0.0f32;
            let mut closing_opacity = 1.0f32;
            if animate {
                SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA)?;
                SetTimer(Some(hwnd), FADE_TIMER, 10, None);
            } else {
                SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
            }
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(Some(
                if !panel.slider.get().0.is_null() && IsWindowEnabled(panel.slider.get()).as_bool()
                {
                    panel.slider.get()
                } else {
                    hwnd
                },
            ));
            let mut msg = MSG::default();
            let mut deferred = Vec::new();
            loop {
                if panel.result.get().is_some() && closing.is_none() {
                    if !animate {
                        break;
                    }
                    closing_opacity = current_opacity;
                    closing = Some(Instant::now());
                    SetTimer(Some(hwnd), FADE_TIMER, 10, None);
                }
                let status = GetMessageW(&mut msg, None, 0, 0).0;
                if status <= 0 {
                    if status == 0 {
                        PostQuitMessage(msg.wParam.0 as i32);
                    }
                    break;
                }
                if msg.hwnd == hwnd && msg.message == WM_TIMER && msg.wParam.0 == FADE_TIMER {
                    let opacity = if let Some(started) = closing {
                        closing_opacity * (1.0 - started.elapsed().as_secs_f32() / 0.16)
                    } else {
                        opened.elapsed().as_secs_f32() / 0.09
                    }
                    .clamp(0.0, 1.0);
                    current_opacity = opacity;
                    SetLayeredWindowAttributes(
                        hwnd,
                        COLORREF(0),
                        (opacity * 255.0).round() as u8,
                        LWA_ALPHA,
                    )?;
                    if closing.is_some() && opacity <= 0.0 {
                        break;
                    }
                    if closing.is_none() && opacity >= 1.0 {
                        let _ = KillTimer(Some(hwnd), FADE_TIMER);
                    }
                    continue;
                }
                if panel.result.get().is_some() {
                    // WM_PAINT must be validated during fade-out. Leaving an update
                    // region dirty generates WM_PAINT continuously and starves the
                    // lower-priority fade timer, trapping the caller in this loop.
                    if msg.message == WM_PAINT {
                        DispatchMessageW(&msg);
                        continue;
                    }
                    if (WM_APP..=0xbfff).contains(&msg.message)
                        && msg.hwnd != hwnd
                        && !IsChild(hwnd, msg.hwnd).as_bool()
                    {
                        deferred.push(msg);
                        continue;
                    }
                    // Keep input for the window that took focus.
                    if msg.hwnd != hwnd && !IsChild(hwnd, msg.hwnd).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    continue;
                }
                let inside = msg.hwnd == hwnd || IsChild(hwnd, msg.hwnd).as_bool();
                match msg.message {
                    WM_APP..=0xbfff if !inside => deferred.push(msg),
                    WM_KEYDOWN if inside => {
                        let key = VIRTUAL_KEY(msg.wParam.0 as u16);
                        let focused = GetFocus();
                        let index = actions.iter().position(|&child| child == focused);
                        match key {
                            VK_ESCAPE => panel.result.set(Some(0)),
                            VK_RETURN if index.is_some() && focused != panel.slider.get() => {
                                panel.choose(focused);
                            }
                            VK_UP | VK_DOWN | VK_TAB | VK_HOME | VK_END
                                if !actions.is_empty()
                                    && (focused != panel.slider.get() || key == VK_TAB) =>
                            {
                                let last = actions.len() - 1;
                                let next =
                                    match key {
                                        VK_HOME => 0,
                                        VK_END => last,
                                        VK_UP => index
                                            .map_or(last, |i| if i == 0 { last } else { i - 1 }),
                                        VK_TAB if GetKeyState(VK_SHIFT.0 as i32) < 0 => index
                                            .map_or(last, |i| if i == 0 { last } else { i - 1 }),
                                        _ => index.map_or(0, |i| (i + 1) % actions.len()),
                                    };
                                let _ = SetFocus(Some(actions[next]));
                            }
                            _ => {
                                let _ = TranslateMessage(&msg);
                                DispatchMessageW(&msg);
                            }
                        }
                    }
                    WM_MOUSEMOVE => {
                        panel.hover(if actions.contains(&msg.hwnd) {
                            msg.hwnd
                        } else {
                            HWND::default()
                        });
                        DispatchMessageW(&msg);
                    }
                    WM_MOUSELEAVE if msg.hwnd == panel.hot.get() => panel.hover(HWND::default()),
                    WM_RBUTTONUP if inside && alignment.contains(TPM_RIGHTBUTTON) => {
                        panel.choose(msg.hwnd)
                    }
                    WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN if !inside => {
                        panel.result.set(Some(0));
                    }
                    _ => {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            let restore_focus = GetForegroundWindow() == hwnd;
            let choice = panel.result.get().unwrap_or(0);
            drop(window);
            if restore_focus && !previous.0.is_null() {
                let _ = SetForegroundWindow(previous);
            }
            for msg in deferred {
                let _ = PostMessageW(Some(msg.hwnd), msg.message, msg.wParam, msg.lParam);
            }
            if let Some(error) = panel.error.into_inner() {
                Err(error)
            } else {
                Ok(choice)
            }
        }
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        let _ = unsafe { DestroyMenu(self.handle) };
    }
}

struct PanelWindow(HWND);

impl Drop for PanelWindow {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

struct Panel<'a> {
    popup: &'a Popup,
    fonts: [Font; 3],
    result: Cell<Option<u32>>,
    hot: Cell<HWND>,
    slider: Cell<HWND>,
    slider_label: Cell<HWND>,
    slider_value: Cell<u32>,
    on_slide: Option<&'a dyn Fn(f32) -> Result<()>>,
    error: RefCell<Option<windows::core::Error>>,
}

impl Panel<'_> {
    fn slide(&self) {
        let value = unsafe {
            SendMessageW(self.slider.get(), TBM_GETPOS, None, None)
                .0
                .clamp(0, 100) as u32
        };
        if value == self.slider_value.get() {
            return;
        }
        if let Some(on_slide) = self.on_slide
            && let Err(error) = on_slide(value as f32 / 100.0)
        {
            *self.error.borrow_mut() = Some(error);
            self.result.set(Some(0));
            return;
        }
        self.slider_value.set(value);
        unsafe {
            let _ = InvalidateRect(Some(self.slider.get()), None, false);
        }
        unsafe {
            let _ = InvalidateRect(Some(self.slider_label.get()), None, false);
        }
    }

    fn draw_slider(&self, draw: &NMCUSTOMDRAW) -> LRESULT {
        if high_contrast() || draw.dwDrawStage != CDDS_PREPAINT {
            return LRESULT(CDRF_DODEFAULT as isize);
        }
        unsafe {
            let slider = self.slider.get();
            let mut bounds = RECT::default();
            let mut channel = RECT::default();
            let mut thumb = RECT::default();
            let _ = GetClientRect(slider, &mut bounds);
            SendMessageW(
                slider,
                TBM_GETCHANNELRECT,
                None,
                Some(LPARAM((&raw mut channel) as isize)),
            );
            SendMessageW(
                slider,
                TBM_GETTHUMBRECT,
                None,
                Some(LPARAM((&raw mut thumb) as isize)),
            );
            let px = |dip| scale(dip, self.popup.dpi);
            let center = (thumb.top + thumb.bottom) / 2;
            channel.top = center - px(2);
            channel.bottom = center + px(2);
            fill(draw.hdc, &bounds, MENU_SURFACE);
            rounded(draw.hdc, &channel, MENU_BORDER, px(4));
            let enabled = IsWindowEnabled(slider).as_bool();
            let accent = if enabled {
                rgb(0xa9c7ff)
            } else {
                rgb(0x8291a8)
            };
            rounded(
                draw.hdc,
                &RECT {
                    right: (thumb.left + thumb.right) / 2,
                    ..channel
                },
                accent,
                px(4),
            );
            let x = (thumb.left + thumb.right) / 2;
            if GetFocus() == slider {
                rounded(
                    draw.hdc,
                    &RECT {
                        left: x - px(10),
                        right: x + px(10),
                        top: center - px(10),
                        bottom: center + px(10),
                    },
                    MENU_BORDER,
                    px(20),
                );
            }
            rounded(
                draw.hdc,
                &RECT {
                    left: x - px(7),
                    right: x + px(7),
                    top: center - px(7),
                    bottom: center + px(7),
                },
                accent,
                px(14),
            );
        }
        LRESULT(CDRF_SKIPDEFAULT as isize)
    }

    fn hover(&self, child: HWND) {
        let previous = self.hot.replace(child);
        if previous != child {
            unsafe {
                if !previous.0.is_null() {
                    let _ = InvalidateRect(Some(previous), None, false);
                }
                if !child.0.is_null() {
                    let _ = InvalidateRect(Some(child), None, false);
                    let _ = TrackMouseEvent(&mut TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: child,
                        dwHoverTime: 0,
                    });
                }
            }
        }
    }

    fn choose(&self, child: HWND) {
        let id = unsafe { GetDlgCtrlID(child) };
        if let Some(row) = usize::try_from(id)
            .ok()
            .and_then(|id| id.checked_sub(1))
            .and_then(|i| self.popup.rows.get(i))
            && row.enabled
        {
            self.result.set(Some(row.id));
        }
    }

    fn paint(&self, hwnd: HWND) {
        unsafe {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut bounds = RECT::default();
            let _ = GetClientRect(hwnd, &mut bounds);
            let px = |dip| scale(dip, self.popup.dpi);
            fill(dc, &bounds, MENU_BORDER);
            rounded(
                dc,
                &RECT {
                    left: 1,
                    top: 1,
                    right: bounds.right - 1,
                    bottom: bounds.bottom - 1,
                },
                MENU_SURFACE,
                px(28),
            );
            let mut y = px(10);
            for (index, row) in self.popup.rows.iter().enumerate() {
                let height = px(row.kind.height());
                if row.kind == Kind::Divider {
                    let top = y + height / 2;
                    fill(
                        dc,
                        &RECT {
                            left: px(16),
                            right: bounds.right - px(16),
                            top,
                            bottom: top + 1,
                        },
                        MENU_BORDER,
                    );
                }
                y += px(self.popup.row_height(index));
            }
            let _ = EndPaint(hwnd, &paint);
        }
    }

    fn draw(&self, item: &DRAWITEMSTRUCT) {
        let Some(row) = (item.CtlID as usize)
            .checked_sub(1)
            .and_then(|i| self.popup.rows.get(i))
        else {
            return;
        };
        let px = |dip| scale(dip, self.popup.dpi);
        let bounds = item.rcItem;
        fill(item.hDC, &bounds, MENU_SURFACE);
        let focused = item.itemState.0 & (ODS_FOCUS.0 | ODS_SELECTED.0) != 0;
        let highlighted = matches!(
            row.kind,
            Kind::Action | Kind::Tile | Kind::Card | Kind::Toggle
        ) && row.enabled
            && (focused || self.hot.get() == item.hwndItem);
        if highlighted {
            rounded(
                item.hDC,
                &RECT {
                    top: bounds.top + px(1),
                    bottom: bounds.bottom - px(1),
                    ..bounds
                },
                if focused {
                    rgb(0x354967)
                } else {
                    rgb(0x293952)
                },
                px(13),
            );
        }
        if matches!(row.kind, Kind::Tile | Kind::Card) {
            let card = row.kind == Kind::Card;
            let surface = if highlighted {
                rgb(0x354967)
            } else if row.checked {
                rgb(0x293952)
            } else {
                rgb(0x202e44)
            };
            rounded(
                item.hDC,
                &RECT {
                    top: bounds.top + px(2),
                    bottom: bounds.bottom - px(3),
                    ..bounds
                },
                surface,
                px(18),
            );
            let icon_left = bounds.left + px(14);
            let icon_top = bounds.top + px(12);
            rounded(
                item.hDC,
                &RECT {
                    left: icon_left,
                    top: icon_top,
                    right: icon_left + px(32),
                    bottom: icon_top + px(32),
                },
                if row.checked {
                    rgb(0x235bc3)
                } else {
                    MENU_BORDER
                },
                px(12),
            );
            if let Some(symbol) = row.symbol {
                icons::draw_gdi(
                    item.hDC,
                    symbol,
                    icon_left + px(5),
                    icon_top + px(5),
                    px(22),
                    if row.checked {
                        if crate::theme::dark() {
                            rgb(0x202b3c)
                        } else {
                            rgb(0xffffff)
                        }
                    } else {
                        rgb(0xf6f8ff)
                    },
                );
            }
            let left = bounds.left + px(if card { 14 } else { 58 });
            let top = bounds.top + px(if card { 49 } else { 12 });
            draw_text(
                item.hDC,
                &row.label,
                &mut RECT {
                    left,
                    top,
                    right: bounds.right - px(8),
                    bottom: top + px(20),
                },
                self.fonts[2].0,
                rgb(0xe8eefb),
            );
            draw_text(
                item.hDC,
                &row.subtitle,
                &mut RECT {
                    left,
                    top: top + px(20),
                    right: bounds.right - px(8),
                    bottom: top + px(37),
                },
                self.fonts[1].0,
                rgb(0xb3c0d5),
            );
            return;
        }
        if row.kind == Kind::Toggle {
            let right = bounds.right - px(10);
            let top = bounds.top + (bounds.bottom - bounds.top - px(22)) / 2;
            rounded(
                item.hDC,
                &RECT {
                    left: right - px(40),
                    top,
                    right,
                    bottom: top + px(22),
                },
                if row.checked {
                    rgb(0x235bc3)
                } else {
                    MENU_BORDER
                },
                px(22),
            );
            let left = right - px(if row.checked { 20 } else { 38 });
            rounded(
                item.hDC,
                &RECT {
                    left,
                    top: top + px(2),
                    right: left + px(18),
                    bottom: top + px(20),
                },
                rgb(0xffffff),
                px(18),
            );
            draw_text(
                item.hDC,
                &row.label,
                &mut RECT {
                    left: bounds.left + px(10),
                    right: right - px(48),
                    ..bounds
                },
                self.fonts[2].0,
                if row.enabled {
                    rgb(0xe8eefb)
                } else {
                    rgb(0x8291a8)
                },
            );
            return;
        }
        let mut label = RECT {
            left: bounds.left + px(10),
            right: bounds.right - px(9),
            ..bounds
        };
        let (font, ink) = match row.kind {
            Kind::Heading => (&self.fonts[0], rgb(0xf6f8ff)),
            Kind::Detail | Kind::Slider => (&self.fonts[1], rgb(0xb3c0d5)),
            _ => (
                &self.fonts[2],
                if row.enabled {
                    rgb(0xe8eefb)
                } else {
                    rgb(0x8291a8)
                },
            ),
        };
        if row.checked {
            label.right -= px(24);
            draw_text(
                item.hDC,
                "✓",
                &mut RECT {
                    left: label.right + px(6),
                    right: bounds.right,
                    ..bounds
                },
                font.0,
                rgb(0xa9c7ff),
            );
        }
        let text = if row.kind == Kind::Slider && row.enabled {
            let level = self.slider_value.get();
            if row.label == "Volume" {
                let description = match level {
                    0 => "Off",
                    1..=33 => "Low",
                    34..=66 => "Medium",
                    _ => "High",
                };
                format!("Volume · {description} · {level}%")
            } else {
                format!("{} · {level}%", row.label)
            }
        } else {
            row.label.clone()
        };
        if row.kind == Kind::Slider && row.label.starts_with("Volume") {
            let level = VolumeLevel::from_status(row.enabled.then_some((
                self.slider_value.get() as f32 / 100.0,
                row.label.contains("muted"),
            )));
            icons::draw_gdi(
                item.hDC,
                Symbol::Volume(level),
                label.left,
                label.top + px(2),
                px(20),
                ink,
            );
            label.left += px(28);
        }
        draw_text(item.hDC, &text, &mut label, font.0, ink);
    }
}

unsafe extern "system" fn panel_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            return LRESULT(1);
        }
        let Some(panel) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Panel<'_>).as_ref()
        else {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        };
        match message {
            WM_HSCROLL if lparam.0 == panel.slider.get().0 as isize && lparam.0 != 0 => {
                panel.slide();
                LRESULT(0)
            }
            WM_NOTIFY if lparam.0 != 0 => {
                let header = &*(lparam.0 as *const NMHDR);
                if header.hwndFrom == panel.slider.get() && header.code == NM_CUSTOMDRAW {
                    panel.draw_slider(&*(lparam.0 as *const NMCUSTOMDRAW))
                } else {
                    DefWindowProcW(hwnd, message, wparam, lparam)
                }
            }
            WM_DRAWITEM => {
                panel.draw(&*(lparam.0 as *const DRAWITEMSTRUCT));
                LRESULT(1)
            }
            WM_PAINT => {
                panel.paint(hwnd);
                LRESULT(0)
            }
            WM_COMMAND if (wparam.0 >> 16) as u32 == BN_CLICKED && lparam.0 != 0 => {
                panel.choose(HWND(lparam.0 as *mut _));
                LRESULT(0)
            }
            WM_ACTIVATE if wparam.0 as u16 as u32 == WA_INACTIVE => {
                if panel.result.get().is_none() {
                    panel.result.set(Some(0));
                }
                let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                LRESULT(0)
            }
            WM_CLOSE => {
                panel.result.set(Some(0));
                LRESULT(0)
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

unsafe extern "system" fn slider_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        if message == WM_PAINT && !high_contrast() {
            // Paint the whole control; native trackbar partial paints leave old thumb trails.
            let _ = InvalidateRect(Some(hwnd), None, false);
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            let panel = &*(data as *const Panel<'_>);
            panel.draw_slider(&NMCUSTOMDRAW {
                hdc: dc,
                dwDrawStage: CDDS_PREPAINT,
                ..Default::default()
            });
            let _ = EndPaint(hwnd, &paint);
            return LRESULT(0);
        }
        if message == WM_ERASEBKGND && !high_contrast() {
            return LRESULT(1);
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use windows::Win32::System::Threading::GetCurrentThreadId;

    #[test]
    fn card_pairs_leave_room_for_sliders_and_actions() -> Result<()> {
        let mut menu = Popup::new(HWND::default(), 344)?;
        menu.heading("Quick Settings")?;
        for index in 0..6 {
            menu.card(index + 1, Symbol::Wifi, "Control", "Status", false)?;
        }
        menu.divider()?;
        menu.slider("Volume", 0.5, true)?;
        menu.action(7, "Settings")?;
        assert_eq!(
            (1..7).map(|i| menu.row_height(i)).collect::<Vec<_>>(),
            [0, 94, 0, 94, 0, 94]
        );
        assert_eq!(
            (0..menu.rows.len())
                .map(|i| menu.row_height(i))
                .sum::<i32>(),
            32 + 3 * 94 + 13 + 62 + 34
        );
        assert!(menu.card_right(2));
        assert!(!menu.card_right(3));
        Ok(())
    }

    #[test]
    #[ignore = "Interactive visual check for the shared panel controls"]
    fn preview_panel_controls() -> Result<()> {
        unsafe {
            crate::theme::configure(
                if std::env::var("STYLED_DOCK_PREVIEW").as_deref() == Ok("light") {
                    crate::theme::ThemeMode::Light
                } else {
                    crate::theme::ThemeMode::Dark
                },
            );
            let owner = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Styled Dock panel preview"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                100,
                100,
                420,
                600,
                None,
                None,
                None,
                None,
            )?;
            let _guard = PanelWindow(owner);
            let mut menu = Popup::new(owner, 400)?;
            menu.heading("Quick Settings")?;
            menu.card(1, Symbol::Wifi, "Wi-Fi", "Connected", true)?;
            menu.card(2, Symbol::Bluetooth, "Bluetooth", "On", true)?;
            menu.card(3, Symbol::Display, "Display", "Brightness & color", false)?;
            menu.card(4, Symbol::Moon, "Focus", "Windows focus settings", false)?;
            menu.card(5, Symbol::Battery, "Power & battery", "AC power", false)?;
            menu.card(
                6,
                Symbol::Settings,
                "Dock settings",
                "Appearance & apps",
                false,
            )?;
            menu.divider()?;
            menu.slider("Volume", 0.66, true)?;
            menu.action(7, "Mute")?;
            menu.divider()?;
            menu.action(8, "Sound settings…")?;
            menu.track_with_slider(owner, POINT { x: 130, y: 150 }, TPM_LEFTALIGN, &|_| Ok(()))?;
            Ok(())
        }
    }

    #[test]
    #[ignore = "Writes native panel artwork at 100% and 150% for visual inspection"]
    fn render_panel_artwork() -> Result<()> {
        unsafe {
            let (width, height) = (1100, 750);
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let previous = SelectObject(dc, bitmap.into());
            for (mode, name) in [
                (crate::theme::ThemeMode::Dark, "dark"),
                (crate::theme::ThemeMode::Light, "light"),
            ] {
                crate::theme::configure(mode);
                fill(
                    dc,
                    &RECT {
                        right: width,
                        bottom: height,
                        ..Default::default()
                    },
                    MENU_SURFACE,
                );
                for (dpi, x) in [(96, 16), (144, 455)] {
                    let px = |dip| scale(dip, dpi);
                    let mut menu = Popup::new(HWND::default(), 400)?;
                    menu.dpi = dpi;
                    menu.heading("Quick Settings")?;
                    menu.card(1, Symbol::Wifi, "Wi-Fi", "Connected", true)?;
                    menu.card(2, Symbol::Bluetooth, "Bluetooth", "On", true)?;
                    menu.card(3, Symbol::Display, "Display", "Brightness & color", false)?;
                    menu.card(4, Symbol::Moon, "Focus", "Windows focus settings", false)?;
                    menu.card(5, Symbol::Battery, "Power & battery", "Battery 73%", false)?;
                    menu.card(
                        6,
                        Symbol::Settings,
                        "Dock settings",
                        "Appearance & apps",
                        false,
                    )?;
                    menu.divider()?;
                    menu.slider("Volume", 0.66, true)?;
                    menu.action(7, "Mute")?;
                    let panel = Panel {
                        popup: &menu,
                        fonts: [
                            Font::new(19, dpi, FW_SEMIBOLD),
                            Font::new(13, dpi, FW_NORMAL),
                            Font::new(14, dpi, FW_MEDIUM),
                        ],
                        result: Cell::new(None),
                        hot: Cell::new(HWND::default()),
                        slider: Cell::new(HWND::default()),
                        slider_label: Cell::new(HWND::default()),
                        slider_value: Cell::new(66),
                        on_slide: None,
                        error: RefCell::new(None),
                    };
                    let mut y = px(10);
                    let panel_width = px(420);
                    for (index, row) in menu.rows.iter().enumerate() {
                        if row.kind != Kind::Divider {
                            let column = (panel_width - px(36)) / 2;
                            let left = x + if row.kind == Kind::Card {
                                if menu.card_right(index) {
                                    px(24) + column
                                } else {
                                    px(12)
                                }
                            } else {
                                px(7)
                            };
                            let row_width = if row.kind == Kind::Card {
                                column
                            } else {
                                panel_width - px(14)
                            };
                            panel.draw(&DRAWITEMSTRUCT {
                                CtlID: (index + 1) as u32,
                                hDC: dc,
                                // The test DC is also a stable non-null identity for hover comparison.
                                hwndItem: HWND(dc.0),
                                rcItem: RECT {
                                    left,
                                    top: y,
                                    right: left + row_width,
                                    bottom: y + px(if row.kind == Kind::Slider {
                                        23
                                    } else {
                                        row.kind.height()
                                    }),
                                },
                                ..Default::default()
                            });
                        }
                        y += px(menu.row_height(index));
                    }
                    for (i, level) in [
                        VolumeLevel::Muted,
                        VolumeLevel::Low,
                        VolumeLevel::Medium,
                        VolumeLevel::High,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        icons::draw_gdi(
                            dc,
                            Symbol::Volume(level),
                            x + px(25 + i as i32 * 72),
                            y + px(12),
                            px(24),
                            rgb(0xf6f8ff),
                        );
                    }
                }
                let pixels =
                    std::slice::from_raw_parts(bits.cast::<u8>(), (width * height * 4) as usize);
                let mut file = Vec::new();
                file.extend_from_slice(b"BM");
                file.extend_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
                file.extend_from_slice(&[0; 4]);
                file.extend_from_slice(&54u32.to_le_bytes());
                file.extend_from_slice(std::slice::from_raw_parts(
                    (&info.bmiHeader as *const BITMAPINFOHEADER).cast::<u8>(),
                    40,
                ));
                file.extend_from_slice(pixels);
                std::fs::create_dir_all(".test-tmp").unwrap();
                std::fs::write(format!(".test-tmp/control-panel-{name}.bmp"), file).unwrap();
            }
            SelectObject(dc, previous);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            Ok(())
        }
    }

    unsafe extern "system" fn select_action(owner: HWND, _: u32, id: usize, _: u32) {
        unsafe {
            if let Ok(panel) = GetWindow(owner, GW_ENABLEDPOPUP)
                && panel != owner
                && let Ok(button) = GetDlgItem(Some(panel), 2)
            {
                let _ = KillTimer(Some(owner), id);
                let _ = InvalidateRect(Some(button), None, false);
                SendMessageW(
                    panel,
                    WM_COMMAND,
                    Some(WPARAM(2)),
                    Some(LPARAM(button.0 as isize)),
                );
                let _ = PostMessageW(
                    Some(owner),
                    crate::settings::WM_SETTINGS_OPEN,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    #[test]
    fn selected_menu_closes_with_pending_repaint() -> Result<()> {
        unsafe {
            let owner = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Menu regression test"),
                WS_OVERLAPPED,
                0,
                0,
                100,
                100,
                None,
                None,
                None,
                None,
            )?;
            let guard = PanelWindow(owner);
            let thread_id = GetCurrentThreadId();
            let (done, wait) = std::sync::mpsc::channel();
            let watchdog = std::thread::spawn(move || {
                if wait.recv_timeout(Duration::from_secs(3)).is_err() {
                    let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            });
            SetTimer(Some(owner), 123, 150, Some(select_action));
            let mut menu = Popup::new(owner, 238)?;
            menu.heading("Test")?;
            menu.action(42, "Settings")?;
            let choice = menu.show_panel(owner, POINT { x: 100, y: 100 }, TPM_LEFTALIGN, None);
            let _ = done.send(());
            let _ = watchdog.join();
            let _ = KillTimer(Some(owner), 123);
            // Consume a watchdog quit message if this regression occurs.
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, WM_QUIT, WM_QUIT, PM_REMOVE);
            assert_eq!(
                choice?, 42,
                "menu did not finish its fade-out and return its action"
            );
            assert!(
                PeekMessageW(
                    &mut msg,
                    Some(owner),
                    crate::settings::WM_SETTINGS_OPEN,
                    crate::settings::WM_SETTINGS_OPEN,
                    PM_REMOVE
                )
                .as_bool(),
                "Settings request was lost during fade-out"
            );
            let instance = GetModuleHandleW(None)?.into();
            let settings = crate::settings::SettingsWindow::open(instance, None)?;
            assert!(
                settings.is_open(),
                "Settings did not open after the menu closed"
            );
            drop(settings);
            drop(guard);
            Ok(())
        }
    }
}
