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
pub const REFRESH_PANEL: u32 = u32::MAX;
pub const MENU_SURFACE: COLORREF = rgb(0x192437);
pub const MENU_BORDER: COLORREF = rgb(0x41536f);
const SS_OWNERDRAW: u32 = 0x000d;
const SS_NOPREFIX: u32 = 0x0080;
const TBM_GETPOS: u32 = WM_USER;
const FIRST_SLIDER_ID: i32 = 0xf001;

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
            Self::Heading => 30,
            Self::Detail => 22,
            Self::Action => 30,
            Self::Slider => 54,
            Self::Tile => 44,
            Self::Card => 48,
            Self::Toggle => 36,
            Self::Divider => 10,
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
    level: u32,
}

pub struct Popup {
    refresh_message: Option<u32>,
    handle: HMENU,
    dpi: i32,
    width: i32,
    rows: Vec<Row>,
}

impl Popup {
    pub fn new(owner: HWND, width: i32) -> Result<Self> {
        Ok(Self {
            refresh_message: None,
            handle: unsafe { CreatePopupMenu()? },
            dpi: (unsafe { GetDpiForWindow(owner) } as i32).max(1),
            width,
            rows: Vec::new(),
        })
    }

    pub fn heading(&mut self, label: &str) -> Result<()> {
        self.add(0, label, Kind::Heading, false, false)
    }

    pub fn refresh_on(&mut self, message: u32) {
        self.refresh_message = Some(message);
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
        let caption = HSTRING::from(format!("{label} · {subtitle}").replace('&', "&&"));
        unsafe {
            ModifyMenuW(
                self.handle,
                (self.rows.len() - 1) as u32,
                MF_BYPOSITION
                    | MF_STRING
                    | if enabled { MF_ENABLED } else { MF_GRAYED }
                    | if checked { MF_CHECKED } else { MF_UNCHECKED },
                id as usize,
                &caption,
            )?;
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
    fn slider_states(&self) -> Vec<Option<SliderState>> {
        self.rows
            .iter()
            .map(|row| {
                (row.kind == Kind::Slider).then(|| SliderState {
                    control: Cell::default(),
                    label: Cell::default(),
                    value: Cell::new(row.level),
                })
            })
            .collect()
    }
    pub fn divider(&mut self) -> Result<()> {
        self.add(0, "", Kind::Divider, false, false)
    }

    pub fn slider(&mut self, label: &str, level: f32, enabled: bool) -> Result<()> {
        self.slider_for(0, label, level, enabled)
    }

    pub fn slider_for(&mut self, id: u32, label: &str, level: f32, enabled: bool) -> Result<()> {
        self.add(id, label, Kind::Slider, enabled, false)?;
        self.rows.last_mut().unwrap().level = (level.clamp(0.0, 1.0) * 100.0).round() as u32;
        Ok(())
    }

    pub fn track_with_slider(
        &self,
        owner: HWND,
        point: POINT,
        alignment: TRACK_POPUP_MENU_FLAGS,
        on_slide: &dyn Fn(f32) -> Result<()>,
    ) -> Result<u32> {
        self.track_with_sliders(owner, point, alignment, &|_, level| on_slide(level))
    }

    pub fn track_with_sliders(
        &self,
        owner: HWND,
        point: POINT,
        alignment: TRACK_POPUP_MENU_FLAGS,
        on_slide: &dyn Fn(u32, f32) -> Result<()>,
    ) -> Result<u32> {
        if high_contrast() {
            return Ok(self.track_native(owner, point, alignment));
        }
        if self.rows.iter().any(|row| row.kind == Kind::Slider) {
            match self.show_panel(owner, point, alignment, Some(on_slide)) {
                Err(error) if error.code() == windows::Win32::Foundation::E_BOUNDS => {
                    Ok(self.track_native(owner, point, alignment))
                }
                result => result,
            }
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
                | if enabled && kind != Kind::Slider {
                    MF_ENABLED
                } else {
                    MF_GRAYED
                }
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
            level: 0,
        });
        Ok(())
    }

    pub fn track(&self, owner: HWND, point: POINT, alignment: TRACK_POPUP_MENU_FLAGS) -> u32 {
        if !high_contrast()
            && let Ok(choice) = self.show_panel(owner, point, alignment, None)
        {
            return choice;
        }
        self.track_native(owner, point, alignment)
    }

    fn track_native(&self, owner: HWND, point: POINT, alignment: TRACK_POPUP_MENU_FLAGS) -> u32 {
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
        on_slide: Option<&dyn Fn(u32, f32) -> Result<()>>,
    ) -> Result<u32> {
        let px = |dip| scale(dip, self.dpi);
        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        let work = crate::placement::monitor_info(monitor)?.monitorInfo.rcWork;
        let font = Font::new(14, self.dpi, FW_MEDIUM);
        let dc = unsafe { GetDC(Some(owner)) };
        let mut measured = 0;
        if !dc.is_invalid() {
            let previous = unsafe { SelectObject(dc, font.0.into()) };
            for row in &self.rows {
                let mut size = windows::Win32::Foundation::SIZE::default();
                let text: Vec<_> = row.label.encode_utf16().collect();
                if unsafe { GetTextExtentPoint32W(dc, &text, &mut size).as_bool() } {
                    measured = measured.max(if row.kind == Kind::Card {
                        size.cx * 2 + px(110)
                    } else {
                        size.cx + px(64)
                    });
                }
            }
            unsafe {
                SelectObject(dc, previous);
                ReleaseDC(Some(owner), dc);
            }
        }
        let width = px(self.width.max(220) + 20)
            .max(measured.min(px(640)))
            .min(work.right - work.left);
        let height = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, _)| px(self.row_height(index)))
            .sum::<i32>()
            + px(20);
        if height > work.bottom - work.top || width < px(120) {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_BOUNDS,
            ));
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
                    Font::new(16, self.dpi, FW_SEMIBOLD),
                    Font::new(13, self.dpi, FW_NORMAL),
                    Font::new(14, self.dpi, FW_MEDIUM),
                ],
                result: Cell::new(None),
                hot: Cell::new(HWND::default()),
                sliders: self.slider_states(),
                on_slide,
                error: RefCell::new(None),
            };
            let previous = GetForegroundWindow();
            let previous_focus = GetFocus();
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED,
                class.lpszClassName,
                &HSTRING::from(crate::i18n::text("Styled Dock menu")),
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
            InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_WIN95_CLASSES,
            })
            .ok()?;
            let tooltip = CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASSW,
                None,
                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                0,
                0,
                0,
                0,
                Some(hwnd),
                None,
                Some(instance),
                None,
            )?;
            SendMessageW(
                tooltip,
                TTM_SETMAXTIPWIDTH,
                None,
                Some(LPARAM(px(440).min(work.right - work.left) as isize)),
            );
            let tooltip_text: Vec<_> = self
                .rows
                .iter()
                .map(|row| {
                    HSTRING::from(if row.subtitle.is_empty() {
                        row.label.clone()
                    } else {
                        format!("{}\n{}", row.label, row.subtitle)
                    })
                })
                .collect();
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
                        crate::tr!("{label} (active)", label = row.label)
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
                    let tool = TTTOOLINFOW {
                        cbSize: size_of::<TTTOOLINFOW>() as u32,
                        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                        hwnd,
                        uId: child.0 as usize,
                        lpszText: windows::core::PWSTR(tooltip_text[index].as_ptr() as *mut _),
                        ..Default::default()
                    };
                    SendMessageW(
                        tooltip,
                        TTM_ADDTOOLW,
                        None,
                        Some(LPARAM((&raw const tool) as isize)),
                    );
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
                            top + px(21),
                            width - px(26),
                            height - px(21),
                            Some(hwnd),
                            Some(HMENU((FIRST_SLIDER_ID as usize + index) as *mut _)),
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
                            Some(LPARAM(row.level as isize)),
                        );
                        let state = panel.sliders[index].as_ref().unwrap();
                        state.control.set(slider);
                        state.label.set(child);
                        style_slider(slider, MENU_SURFACE)?;
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
            let first_slider = panel.sliders.iter().flatten().find_map(|state| {
                let control = state.control.get();
                IsWindowEnabled(control).as_bool().then_some(control)
            });
            let _ = SetFocus(Some(first_slider.unwrap_or(hwnd)));
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
                if self.refresh_message == Some(msg.message)
                    && msg.hwnd == owner
                    && panel.result.get().is_none()
                {
                    panel.result.set(Some(REFRESH_PANEL));
                    continue;
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
                            VK_RETURN if index.is_some() && panel.slider(focused).is_none() => {
                                panel.choose(focused);
                            }
                            VK_UP | VK_DOWN | VK_TAB | VK_HOME | VK_END
                                if !actions.is_empty()
                                    && (panel.slider(focused).is_none() || key == VK_TAB) =>
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
                if !previous_focus.is_invalid() && IsWindow(Some(previous_focus)).as_bool() {
                    let _ = SetFocus(Some(previous_focus));
                }
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

struct SliderState {
    control: Cell<HWND>,
    label: Cell<HWND>,
    value: Cell<u32>,
}

struct Panel<'a> {
    popup: &'a Popup,
    fonts: [Font; 3],
    result: Cell<Option<u32>>,
    hot: Cell<HWND>,
    sliders: Vec<Option<SliderState>>,
    on_slide: Option<&'a dyn Fn(u32, f32) -> Result<()>>,
    error: RefCell<Option<windows::core::Error>>,
}

impl Panel<'_> {
    fn slider(&self, control: HWND) -> Option<(usize, &SliderState)> {
        if control.is_invalid() {
            return None;
        }
        let id = unsafe { GetDlgCtrlID(control) };
        let index = usize::try_from(id - FIRST_SLIDER_ID).ok()?;
        let state = self.sliders.get(index)?.as_ref()?;
        (state.control.get() == control).then_some((index, state))
    }

    fn slide(&self, control: HWND) {
        let Some((index, state)) = self.slider(control) else {
            return;
        };
        let value = unsafe {
            SendMessageW(control, TBM_GETPOS, None, None)
                .0
                .clamp(0, 100) as u32
        };
        if value == state.value.get() {
            return;
        }
        if let Some(on_slide) = self.on_slide
            && let Err(error) = on_slide(self.popup.rows[index].id, value as f32 / 100.0)
        {
            *self.error.borrow_mut() = Some(error);
            self.result.set(Some(0));
            return;
        }
        state.value.set(value);
        unsafe {
            let _ = InvalidateRect(Some(control), None, false);
            let _ = InvalidateRect(Some(state.label.get()), None, false);
        }
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
        let Some(index) = (item.CtlID as usize).checked_sub(1) else {
            return;
        };
        let Some(row) = self.popup.rows.get(index) else {
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
            if card && !highlighted {
                rounded(item.hDC, &bounds, rgb(0x202e44), px(14));
            }
            let icon_left = bounds.left + px(10);
            let icon_top = (bounds.top + bounds.bottom - px(26)) / 2;
            if row.checked {
                rounded(
                    item.hDC,
                    &RECT {
                        left: icon_left,
                        top: icon_top,
                        right: icon_left + px(26),
                        bottom: icon_top + px(26),
                    },
                    rgb(0x293952),
                    px(26),
                );
            }
            if let Some(symbol) = row.symbol {
                icons::draw_gdi(
                    item.hDC,
                    symbol,
                    icon_left + px(3),
                    icon_top + px(3),
                    px(20),
                    if row.checked {
                        rgb(0xa9c7ff)
                    } else {
                        rgb(0xf6f8ff)
                    },
                );
            }
            let left = bounds.left + px(43);
            let top = (bounds.top + bounds.bottom - px(34)) / 2;
            let right = bounds.right - px(if row.checked { 28 } else { 8 });
            draw_text(
                item.hDC,
                &row.label,
                &mut RECT {
                    left,
                    top,
                    right,
                    bottom: top + px(18),
                },
                self.fonts[2].0,
                rgb(0xe8eefb),
            );
            draw_text(
                item.hDC,
                &row.subtitle,
                &mut RECT {
                    left,
                    top: top + px(17),
                    right,
                    bottom: top + px(34),
                },
                self.fonts[1].0,
                rgb(0xb3c0d5),
            );
            if row.checked {
                draw_text(
                    item.hDC,
                    "✓",
                    &mut RECT {
                        left: bounds.right - px(22),
                        ..bounds
                    },
                    self.fonts[2].0,
                    rgb(0xa9c7ff),
                );
            }
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
        let slider = self.sliders[index].as_ref();
        let text = if let Some(slider) = slider.filter(|_| row.enabled) {
            format!("{} · {}%", row.label, slider.value.get())
        } else {
            row.label.clone()
        };
        let symbol = slider.and_then(|slider| {
            let muted = row.label == crate::i18n::text("Volume muted");
            if muted
                || [
                    crate::i18n::text("Volume"),
                    crate::i18n::text("Volume unavailable"),
                ]
                .contains(&row.label.as_str())
            {
                Some(Symbol::Volume(VolumeLevel::from_status(
                    row.enabled
                        .then_some((slider.value.get() as f32 / 100.0, muted)),
                )))
            } else if row.label == crate::i18n::text("Brightness") {
                Some(Symbol::Display)
            } else {
                None
            }
        });
        if let Some(symbol) = symbol {
            icons::draw_gdi(item.hDC, symbol, label.left, label.top + px(2), px(20), ink);
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
            WM_HSCROLL if lparam.0 != 0 => {
                panel.slide(HWND(lparam.0 as *mut _));
                LRESULT(0)
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

pub fn style_slider(hwnd: HWND, surface: COLORREF) -> Result<()> {
    unsafe { SetWindowSubclass(hwnd, Some(slider_proc), 1, surface.0 as usize) }.ok()
}

fn draw_slider(slider: HWND, dc: HDC, surface: COLORREF) {
    unsafe {
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
        let px = |dip| scale(dip, GetDpiForWindow(slider) as i32);
        let center = (thumb.top + thumb.bottom) / 2;
        channel.top = center - px(3);
        channel.bottom = center + px(3);
        fill(dc, &bounds, surface);
        rounded(dc, &channel, MENU_BORDER, px(6));
        let enabled = IsWindowEnabled(slider).as_bool();
        let accent = if enabled {
            rgb(0xa9c7ff)
        } else {
            rgb(0x8291a8)
        };
        rounded(
            dc,
            &RECT {
                right: (thumb.left + thumb.right) / 2,
                ..channel
            },
            accent,
            px(6),
        );
        let x = (thumb.left + thumb.right) / 2;
        if GetFocus() == slider {
            rounded(
                dc,
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
            dc,
            &RECT {
                left: x - px(7),
                right: x + px(7),
                top: center - px(7),
                bottom: center + px(7),
            },
            MENU_BORDER,
            px(14),
        );
        rounded(
            dc,
            &RECT {
                left: x - px(7) + 1,
                right: x + px(7) - 1,
                top: center - px(7) + 1,
                bottom: center + px(7) - 1,
            },
            if enabled { rgb(0xffffff) } else { accent },
            px(14),
        );
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
            draw_slider(hwnd, dc, COLORREF(data as u32));
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
    fn oversized_panels_report_layout_limit_for_native_scrollable_fallback() -> Result<()> {
        let owner = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Panel overflow test"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )?
        };
        let mut menu = Popup::new(owner, 400)?;
        for id in 1..=100 {
            menu.action(id, "A long localized action that must remain reachable")?;
        }
        menu.slider("Volume", 0.5, true)?;
        let result = menu.show_panel(owner, POINT::default(), TPM_LEFTALIGN, None);
        let _ = unsafe { DestroyWindow(owner) };
        assert_eq!(
            result.unwrap_err().code(),
            windows::Win32::Foundation::E_BOUNDS
        );
        Ok(())
    }

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
            [0, 48, 0, 48, 0, 48]
        );
        assert_eq!(
            (0..menu.rows.len())
                .map(|i| menu.row_height(i))
                .sum::<i32>(),
            30 + 3 * 48 + 10 + 54 + 30
        );
        assert!(menu.card_right(2));
        assert!(!menu.card_right(3));
        Ok(())
    }

    unsafe extern "system" fn adjust_sliders(owner: HWND, _: u32, timer: usize, _: u32) {
        unsafe {
            if let Ok(panel) = GetWindow(owner, GW_ENABLEDPOPUP)
                && panel != owner
            {
                let _ = KillTimer(Some(owner), timer);
                for (index, level) in [(0, 30), (1, 80)] {
                    if let Ok(control) = GetDlgItem(Some(panel), FIRST_SLIDER_ID + index) {
                        SendMessageW(control, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(level)));
                        SendMessageW(panel, WM_HSCROLL, None, Some(LPARAM(control.0 as isize)));
                    }
                }
                let _ = PostMessageW(Some(panel), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }

    #[test]
    fn sliders_report_their_own_control_id_and_value() -> Result<()> {
        unsafe {
            let owner = PanelWindow(CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Independent slider test"),
                WS_OVERLAPPED,
                0,
                0,
                100,
                100,
                None,
                None,
                None,
                None,
            )?);
            let mut menu = Popup::new(owner.0, 280)?;
            menu.slider_for(30, "Brightness", 0.5, true)?;
            menu.slider_for(10, "Volume", 0.6, true)?;
            let updates = RefCell::new(Vec::new());
            let thread_id = GetCurrentThreadId();
            let (done, wait) = std::sync::mpsc::channel();
            let watchdog = std::thread::spawn(move || {
                if wait.recv_timeout(Duration::from_secs(3)).is_err() {
                    let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            });
            SetTimer(Some(owner.0), 124, 150, Some(adjust_sliders));
            let result = menu.show_panel(
                owner.0,
                POINT { x: 100, y: 100 },
                TPM_LEFTALIGN,
                Some(&|id, value| {
                    updates.borrow_mut().push((id, value));
                    Ok(())
                }),
            );
            let _ = done.send(());
            let _ = watchdog.join();
            let _ = KillTimer(Some(owner.0), 124);
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, WM_QUIT, WM_QUIT, PM_REMOVE);
            assert_eq!(result?, 0);
            assert_eq!(updates.into_inner(), [(30, 0.30), (10, 0.80)]);
            Ok(())
        }
    }

    fn sample_controls(owner: HWND, dpi: i32) -> Result<Popup> {
        let t = crate::i18n::text;
        let mut menu = Popup::new(owner, 320)?;
        menu.dpi = dpi;
        menu.heading(t("Quick Settings"))?;
        menu.tile(10, Symbol::Wifi, t("Wi-Fi"), t("Connected"), true, true)?;
        menu.tile(11, Symbol::Bluetooth, t("Bluetooth"), t("On"), true, true)?;
        menu.divider()?;
        menu.slider_for(30, t("Brightness"), 0.48, true)?;
        menu.slider(t("Volume"), 0.66, true)?;
        menu.action(3, t("Mute"))?;
        menu.divider()?;
        menu.card(13, Symbol::Moon, t("Focus"), t("Windows"), false)?;
        menu.card(
            14,
            Symbol::Battery,
            t("Power & battery"),
            &crate::tr!("Battery {percent}%", percent = 73),
            false,
        )?;
        menu.divider()?;
        menu.action(12, t("Display options…"))?;
        menu.action(15, t("Dock settings"))?;
        Ok(menu)
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
            let menu = sample_controls(owner, GetDpiForWindow(owner) as i32)?;
            menu.track_with_slider(owner, POINT { x: 130, y: 150 }, TPM_LEFTALIGN, &|_| Ok(()))?;
            Ok(())
        }
    }

    #[test]
    #[ignore = "Writes English/Turkish panel artwork at 100%, 150%, and 200%"]
    fn render_panel_artwork() -> Result<()> {
        unsafe {
            let (width, height) = (1960, 1300);
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
            for (mode, name, language) in [
                (crate::theme::ThemeMode::Dark, "dark", "en"),
                (crate::theme::ThemeMode::Light, "light", "en"),
                (crate::theme::ThemeMode::Dark, "dark-tr", "tr"),
                (crate::theme::ThemeMode::Light, "light-tr", "tr"),
            ] {
                crate::theme::configure(mode);
                crate::i18n::configure(language);
                fill(
                    dc,
                    &RECT {
                        right: width,
                        bottom: height,
                        ..Default::default()
                    },
                    MENU_SURFACE,
                );
                for (dpi, x) in [(96, 16), (144, 455), (192, 1101)] {
                    let px = |dip| scale(dip, dpi);
                    let menu = sample_controls(HWND::default(), dpi)?;
                    let panel = Panel {
                        popup: &menu,
                        fonts: [
                            Font::new(16, dpi, FW_SEMIBOLD),
                            Font::new(13, dpi, FW_NORMAL),
                            Font::new(14, dpi, FW_MEDIUM),
                        ],
                        result: Cell::new(None),
                        hot: Cell::new(HWND::default()),
                        sliders: menu.slider_states(),
                        on_slide: None,
                        error: RefCell::new(None),
                    };
                    let mut y = px(10);
                    let panel_width = px(340);
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
