use std::mem::size_of;
use std::path::Path;
use std::process::Command;

use windows::Win32::Foundation::{
    COLORREF, E_FAIL, E_INVALIDARG, HINSTANCE, HWND, LPARAM, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, HSTRING, PCWSTR, PWSTR, Result, w};

use crate::config::{Appearance, Config, MAX_PINS, Pin, Source};
use crate::placement::display_names;
use crate::popup::{self, ACCENT, Font, INK, LINE, MUTED, PANEL, SELECTED, SIDEBAR};

pub const WM_SETTINGS_OPEN: u32 = WM_APP + 20;
pub const WM_SETTINGS_SAVED: u32 = WM_APP + 21;

const ID_DISPLAY: i32 = 110;
const ID_MODE: i32 = 111;
const ID_TOP_BAR: i32 = 112;
const ID_FULLSCREEN: i32 = 113;
const ID_TASKBAR: i32 = 114;
const ID_DOCK_SIZE: i32 = 115;
const ID_HOVER_ZOOM: i32 = 116;
const ID_ANIMATION_SPEED: i32 = 117;
const ID_HIDE_DELAY: i32 = 118;
const ID_LAUNCH_BOUNCE: i32 = 119;
const ID_PINS: i32 = 120;
const ID_UP: i32 = 121;
const ID_DOWN: i32 = 122;
const ID_REMOVE: i32 = 123;
const ID_NAME: i32 = 124;
const ID_TARGET: i32 = 125;
const ID_BROWSE: i32 = 126;
const ID_ADD: i32 = 127;
const ID_UPDATE: i32 = 128;
const ID_PATH: i32 = 129;
const ID_DEFAULTS: i32 = 130;
const ID_DOCK_OPACITY: i32 = 131;
const ID_TOP_BAR_OPACITY: i32 = 132;
const ID_PAGES: i32 = 133;
const ID_PAGE_TITLE: i32 = 134;
const ID_PAGE_DESCRIPTION: i32 = 135;
const ID_BRAND: i32 = 136;
const ID_DOCK_OPACITY_VALUE: i32 = 137;
const ID_TOP_BAR_OPACITY_VALUE: i32 = 138;
const ID_THEME: i32 = 139;
const ID_ICON_SIZE: i32 = 140;
const ID_ICON_SPACING: i32 = 141;
const ID_EDGE_PADDING: i32 = 142;
const ID_LABEL_DELAY: i32 = 143;
const ID_TRAY: i32 = 144;
const ID_START: i32 = 145;
const ID_SEARCH: i32 = 146;
const ID_DOWNLOADS: i32 = 147;
const ID_INSTALLED: i32 = 148;
const ID_LANGUAGE: i32 = 149;
const ID_STARTUP: i32 = 150;
const TBM_GETPOS: u32 = WM_USER;

const PAGES: [(&str, &str, &str, &[i32]); 6] = [
    (
        "General",
        "General Settings",
        "Choose your language and Windows startup preferences.",
        &[231, ID_LANGUAGE, ID_STARTUP, 232],
    ),
    (
        "Display",
        "Display & Behavior",
        "Choose where the dock appears and how it behaves.",
        &[
            200,
            ID_DISPLAY,
            201,
            ID_MODE,
            ID_TOP_BAR,
            ID_TRAY,
            ID_FULLSCREEN,
            ID_TASKBAR,
        ],
    ),
    (
        "Appearance",
        "Appearance & Motion",
        "Adjust the dock, top bar, and their animations.",
        &[
            212,
            ID_HOVER_ZOOM,
            213,
            214,
            ID_ANIMATION_SPEED,
            215,
            216,
            ID_HIDE_DELAY,
            217,
            218,
            ID_DOCK_OPACITY,
            ID_DOCK_OPACITY_VALUE,
            219,
            ID_TOP_BAR_OPACITY,
            ID_TOP_BAR_OPACITY_VALUE,
            ID_THEME,
            222,
            ID_LAUNCH_BOUNCE,
        ],
    ),
    (
        "Layout",
        "Dock Layout",
        "Sizes use logical pixels and follow dock size and display scaling.",
        &[
            210,
            ID_DOCK_SIZE,
            211,
            223,
            ID_ICON_SIZE,
            224,
            225,
            ID_ICON_SPACING,
            226,
            227,
            ID_EDGE_PADDING,
            228,
            229,
            ID_LABEL_DELAY,
            230,
        ],
    ),
    (
        "Shortcuts",
        "Dock Shortcuts",
        "Choose which Windows shortcuts appear in the dock.",
        &[ID_START, ID_SEARCH, ID_DOWNLOADS],
    ),
    (
        "Pinned Apps",
        "Pinned Apps",
        "Choose which apps stay in your dock and set their order.",
        &[
            ID_PINS,
            ID_UP,
            ID_DOWN,
            ID_REMOVE,
            220,
            ID_NAME,
            221,
            ID_TARGET,
            ID_BROWSE,
            ID_ADD,
            ID_UPDATE,
            ID_INSTALLED,
        ],
    ),
];

// Keep a marker for every page; a missing marker must fail at compile time.
const PAGE_MARKS: [&str; PAGES.len()] = ["⚙", "▣", "◐", "▦", "⌕", "▤"];

pub struct SettingsWindow {
    hwnd: HWND,
    _state: Box<State>,
}

impl SettingsWindow {
    pub fn open(instance: HINSTANCE, owner: Option<HWND>) -> Result<Self> {
        let source = Source::open()?;
        let config = source.draft()?;
        let original_language = config.general.language.clone();
        crate::i18n::configure(&config.general.language);
        crate::theme::configure(config.appearance.theme);
        let mut state = Box::new(State {
            owner,
            source,
            config,
            displays: Vec::new(),
            skin: None,
            saved: false,
            captions: Vec::new(),
            original_language,
        });
        let hwnd = unsafe {
            InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES,
            })
            .ok()?;
            CreateDialogParamW(
                Some(instance),
                PCWSTR(101usize as *const u16),
                None,
                Some(dialog_proc),
                LPARAM((&raw mut *state) as isize),
            )?
        };
        let window = Self {
            hwnd,
            _state: state,
        };
        window.focus();
        Ok(window)
    }

    pub fn focus(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    pub fn is_open(&self) -> bool {
        unsafe { IsWindow(Some(self.hwnd)).as_bool() }
    }

    pub fn route(&self, message: &MSG) -> bool {
        self.is_open() && unsafe { IsDialogMessageW(self.hwnd, message).as_bool() }
    }
}

impl Drop for SettingsWindow {
    fn drop(&mut self) {
        if self.is_open() {
            let _ = unsafe { DestroyWindow(self.hwnd) };
        }
    }
}

pub fn open_standalone() -> Result<()> {
    let _ole = crate::files::Ole::new()?;
    show_standalone().map(|_| ())
}

/// Setup runs before the dock changes any Windows shell state. Closing it exits
/// without marking setup complete, so it can be resumed on the next launch.
pub fn first_time_setup() -> Result<bool> {
    show_standalone()
}

fn show_standalone() -> Result<bool> {
    unsafe {
        let settings = SettingsWindow::open(GetModuleHandleW(None)?.into(), None)?;
        let mut message = MSG::default();
        while settings.is_open() {
            match GetMessageW(&mut message, None, 0, 0).0 {
                -1 => return Err(Error::from_thread()),
                0 => break,
                _ if settings.route(&message) => {}
                _ => {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        return Ok(settings._state.saved);
    }
}

pub fn restart() -> Result<()> {
    let error = |error: std::io::Error| {
        Error::new(
            E_FAIL,
            crate::tr!(
                "Settings were saved. Reopen Styled Dock to apply them.\n\n{error}",
                error = error
            ),
        )
    };
    Command::new(std::env::current_exe().map_err(error)?)
        .spawn()
        .map_err(error)?;
    Ok(())
}

struct State {
    owner: Option<HWND>,
    source: Source,
    config: Config,
    displays: Vec<String>,
    skin: Option<SettingsSkin>,
    saved: bool,
    captions: Vec<(HWND, String)>,
    original_language: String,
}

struct SettingsSkin {
    title: Font,
    brand: Font,
    sidebar_font: Font,
    panel: HBRUSH,
    sidebar: HBRUSH,
}

impl SettingsSkin {
    fn new(dpi: i32) -> Self {
        Self {
            title: Font::new(24, dpi, FW_SEMIBOLD),
            brand: Font::new(16, dpi, FW_SEMIBOLD),
            sidebar_font: Font::new(13, dpi, FW_MEDIUM),
            panel: unsafe { CreateSolidBrush(popup::adapted(PANEL)) },
            sidebar: unsafe { CreateSolidBrush(popup::adapted(SIDEBAR)) },
        }
    }

    fn background(hwnd: HWND, dc: HDC) {
        let mut client = RECT::default();
        let mut guides = RECT {
            right: 115,
            bottom: 240,
            ..Default::default()
        };
        unsafe {
            let _ = GetClientRect(hwnd, &mut client);
            let _ = MapDialogRect(hwnd, &mut guides);
        }
        popup::fill(dc, &client, PANEL);
        popup::fill(
            dc,
            &RECT {
                right: guides.right,
                ..client
            },
            SIDEBAR,
        );
        popup::fill(
            dc,
            &RECT {
                left: guides.right,
                right: guides.right + 1,
                ..client
            },
            LINE,
        );
        popup::fill(
            dc,
            &RECT {
                left: guides.right + 1,
                top: guides.bottom,
                bottom: guides.bottom + 1,
                ..client
            },
            LINE,
        );
    }
}

impl Drop for SettingsSkin {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.panel.into());
            let _ = DeleteObject(self.sidebar.into());
        }
    }
}

impl State {
    fn style(&mut self, hwnd: HWND) {
        unsafe {
            let dark = i32::from(crate::theme::dark() && !popup::high_contrast());
            let _ = DwmSetWindowAttribute(
                hwnd,
                windows::Win32::Graphics::Dwm::DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const dark).cast(),
                size_of::<i32>() as u32,
            );
            let _ = EnumChildWindows(Some(hwnd), Some(theme_control), LPARAM(0));
        }
        let dpi = unsafe { GetDpiForWindow(hwnd) } as i32;
        let skin = if popup::high_contrast() {
            None
        } else {
            Some(SettingsSkin::new(dpi))
        };
        let dialog_font = unsafe { SendMessageW(hwnd, WM_GETFONT, None, None) }.0 as usize;
        for (id, font) in [
            (ID_PAGE_TITLE, skin.as_ref().map(|skin| skin.title.0)),
            (ID_BRAND, skin.as_ref().map(|skin| skin.brand.0)),
        ] {
            control_message(
                hwnd,
                id,
                WM_SETFONT,
                font.map_or(dialog_font, |font| font.0 as usize),
                1,
            );
        }
        self.skin = skin;
        control_message(
            hwnd,
            ID_PAGES,
            LB_SETITEMHEIGHT,
            0,
            popup::scale(36, dpi) as isize,
        );
        unsafe {
            let preference = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const preference).cast(),
                size_of_val(&preference) as u32,
            );
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
    }

    fn show_page(hwnd: HWND, page: usize) {
        if let Some((_, title, description, _)) = PAGES.get(page) {
            for (index, (_, _, _, ids)) in PAGES.iter().enumerate() {
                for &id in *ids {
                    if let Ok(control) = unsafe { GetDlgItem(Some(hwnd), id) } {
                        let _ = unsafe {
                            ShowWindow(control, if index == page { SW_SHOWNA } else { SW_HIDE })
                        };
                    }
                }
            }
            set_text(hwnd, ID_PAGE_TITLE, crate::i18n::text(title));
            set_text(hwnd, ID_PAGE_DESCRIPTION, crate::i18n::text(description));
        }
    }

    fn draw_page(&self, hwnd: HWND, item: &DRAWITEMSTRUCT) {
        let Some((label, ..)) = PAGES.get(item.itemID as usize) else {
            return;
        };
        let bounds = item.rcItem;
        let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
        let dpi = unsafe { GetDpiForWindow(hwnd) } as i32;
        let px = |dip| popup::scale(dip, dpi);
        let (font, ink) = if let Some(skin) = &self.skin {
            popup::fill(item.hDC, &bounds, SIDEBAR);
            if selected {
                popup::rounded(
                    item.hDC,
                    &RECT {
                        top: bounds.top + px(2),
                        bottom: bounds.bottom - px(2),
                        ..bounds
                    },
                    SELECTED,
                    px(10),
                );
            }
            let badge = RECT {
                left: bounds.left + px(9),
                top: bounds.top + px(8),
                right: bounds.left + px(29),
                bottom: bounds.bottom - px(8),
            };
            popup::rounded(
                item.hDC,
                &badge,
                if selected { ACCENT } else { SELECTED },
                px(6),
            );
            let mark = PAGE_MARKS[item.itemID as usize];
            popup::draw_text(
                item.hDC,
                mark,
                &mut RECT {
                    left: badge.left + px(4),
                    ..badge
                },
                skin.sidebar_font.0,
                if selected {
                    popup::rgb(0xffffff)
                } else {
                    ACCENT
                },
            );
            (skin.sidebar_font.0, if selected { ACCENT } else { INK })
        } else {
            let (background, text) = if selected {
                (COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT)
            } else {
                (COLOR_WINDOW, COLOR_WINDOWTEXT)
            };
            popup::fill(
                item.hDC,
                &bounds,
                COLORREF(unsafe { GetSysColor(background) }),
            );
            (HFONT::default(), COLORREF(unsafe { GetSysColor(text) }))
        };
        popup::draw_text(
            item.hDC,
            crate::i18n::text(label),
            &mut RECT {
                left: bounds.left + px(38),
                ..bounds
            },
            font,
            ink,
        );
        if item.itemState.0 & ODS_FOCUS.0 != 0 {
            let _ = unsafe {
                DrawFocusRect(
                    item.hDC,
                    &RECT {
                        left: bounds.left + px(2),
                        top: bounds.top + px(3),
                        right: bounds.right - px(2),
                        bottom: bounds.bottom - px(3),
                    },
                )
            };
        }
    }

    fn fill(&mut self, hwnd: HWND) {
        control_message(hwnd, ID_LANGUAGE, CB_RESETCONTENT, 0, 0);
        add_text(
            hwnd,
            ID_LANGUAGE,
            CB_ADDSTRING,
            crate::i18n::text("Auto (Windows)"),
        );
        for language in crate::i18n::LANGUAGES {
            add_text(hwnd, ID_LANGUAGE, CB_ADDSTRING, language.name);
        }
        let language = crate::i18n::LANGUAGES
            .iter()
            .position(|language| language.code == self.config.general.language)
            .map_or(0, |index| index + 1);
        control_message(hwnd, ID_LANGUAGE, CB_SETCURSEL, language, 0);
        let _ = unsafe {
            CheckDlgButton(
                hwnd,
                ID_STARTUP,
                if crate::startup::enabled() {
                    BST_CHECKED
                } else {
                    BST_UNCHECKED
                },
            )
        };
        self.displays = vec!["primary".into(), "cursor".into()];
        self.displays.extend(display_names());
        let monitor = &self.config.placement.monitor;
        if !self
            .displays
            .iter()
            .any(|name| name.eq_ignore_ascii_case(monitor))
        {
            self.displays.push(monitor.clone());
        }
        control_message(hwnd, ID_DISPLAY, CB_RESETCONTENT, 0, 0);
        for (index, name) in self.displays.iter().enumerate() {
            let label = match name.as_str() {
                "primary" => crate::i18n::text("Primary display"),
                "cursor" => crate::i18n::text("Display under cursor"),
                _ => name.strip_prefix(r"\\.\").unwrap_or(name),
            };
            add_text(hwnd, ID_DISPLAY, CB_ADDSTRING, label);
            if name.eq_ignore_ascii_case(monitor) {
                control_message(hwnd, ID_DISPLAY, CB_SETCURSEL, index, 0);
            }
        }
        control_message(hwnd, ID_MODE, CB_RESETCONTENT, 0, 0);
        for label in ["Floating", "Reserve space", "Auto-hide"] {
            add_text(hwnd, ID_MODE, CB_ADDSTRING, label);
        }
        let placement = &self.config.placement;
        let mode = if placement.auto_hide {
            2
        } else {
            usize::from(placement.reserve_space)
        };
        control_message(hwnd, ID_MODE, CB_SETCURSEL, mode, 0);
        for (id, value) in [
            (ID_START, self.config.dock.show_start),
            (ID_SEARCH, self.config.dock.show_search),
            (ID_DOWNLOADS, self.config.dock.show_downloads),
            (ID_TOP_BAR, self.config.top_bar.enabled),
            (ID_TRAY, self.config.top_bar.show_tray),
            (ID_FULLSCREEN, placement.hide_on_fullscreen),
            (ID_TASKBAR, placement.hide_taskbar),
            (ID_LAUNCH_BOUNCE, self.config.appearance.launch_bounce),
        ] {
            let _ = unsafe {
                CheckDlgButton(hwnd, id, if value { BST_CHECKED } else { BST_UNCHECKED })
            };
        }
        let appearance = self.config.appearance;
        control_message(hwnd, ID_THEME, CB_RESETCONTENT, 0, 0);
        for label in ["Auto (Windows)", "Light", "Dark"] {
            add_text(hwnd, ID_THEME, CB_ADDSTRING, label);
        }
        control_message(
            hwnd,
            ID_THEME,
            CB_SETCURSEL,
            match appearance.theme {
                crate::theme::ThemeMode::Auto => 0,
                crate::theme::ThemeMode::Light => 1,
                crate::theme::ThemeMode::Dark => 2,
            },
            0,
        );
        for (id, amount) in [
            (ID_DOCK_SIZE, appearance.size_percent),
            (ID_ICON_SIZE, appearance.icon_size),
            (ID_ICON_SPACING, appearance.icon_spacing),
            (ID_EDGE_PADDING, appearance.edge_padding),
            (ID_LABEL_DELAY, appearance.label_delay_ms),
            (ID_HOVER_ZOOM, appearance.zoom_percent),
            (ID_ANIMATION_SPEED, appearance.animation_speed_percent),
            (ID_HIDE_DELAY, appearance.auto_hide_delay_ms),
        ] {
            set_text(hwnd, id, &amount.to_string());
        }
        for (id, amount) in [
            (ID_DOCK_OPACITY, appearance.dock_opacity_percent),
            (ID_TOP_BAR_OPACITY, appearance.top_bar_opacity_percent),
        ] {
            control_message(hwnd, id, TBM_SETRANGE, 1, 100 << 16);
            control_message(hwnd, id, TBM_SETPOS, 1, amount.min(100) as isize);
            update_opacity(hwnd, id);
        }
        self.fill_pins(hwnd, None);
    }

    fn translate(&self, hwnd: HWND) {
        crate::i18n::translate_captions(&self.captions);
        for (id, labels) in [
            (ID_MODE, &["Floating", "Reserve space", "Auto-hide"][..]),
            (ID_THEME, &["Auto (Windows)", "Light", "Dark"][..]),
        ] {
            let selection = control_message(hwnd, id, CB_GETCURSEL, 0, 0);
            control_message(hwnd, id, CB_RESETCONTENT, 0, 0);
            for label in labels {
                add_text(hwnd, id, CB_ADDSTRING, label);
            }
            control_message(hwnd, id, CB_SETCURSEL, selection.max(0) as usize, 0);
        }
        let selection = control_message(hwnd, ID_DISPLAY, CB_GETCURSEL, 0, 0);
        control_message(hwnd, ID_DISPLAY, CB_RESETCONTENT, 0, 0);
        for name in &self.displays {
            let label = match name.as_str() {
                "primary" => "Primary display",
                "cursor" => "Display under cursor",
                _ => name.strip_prefix(r"\\.\").unwrap_or(name),
            };
            add_text(hwnd, ID_DISPLAY, CB_ADDSTRING, label);
        }
        control_message(hwnd, ID_DISPLAY, CB_SETCURSEL, selection.max(0) as usize, 0);
        let language = control_message(hwnd, ID_LANGUAGE, CB_GETCURSEL, 0, 0);
        control_message(hwnd, ID_LANGUAGE, CB_DELETESTRING, 0, 0);
        add_text(hwnd, ID_LANGUAGE, CB_INSERTSTRING, "Auto (Windows)");
        control_message(hwnd, ID_LANGUAGE, CB_SETCURSEL, language.max(0) as usize, 0);
        let page = control_message(hwnd, ID_PAGES, LB_GETCURSEL, 0, 0).max(0) as usize;
        control_message(hwnd, ID_PAGES, LB_RESETCONTENT, 0, 0);
        for (label, ..) in PAGES {
            add_text(hwnd, ID_PAGES, LB_ADDSTRING, label);
        }
        control_message(hwnd, ID_PAGES, LB_SETCURSEL, page, 0);
        Self::show_page(hwnd, page);
        if !self.config.general.setup_completed && page == 0 {
            set_text(
                hwnd,
                ID_PAGE_TITLE,
                crate::i18n::text("Welcome to Styled Dock"),
            );
            set_text(
                hwnd,
                ID_PAGE_DESCRIPTION,
                crate::i18n::text("Set your preferences, explore the pages, then finish setup."),
            );
        }
        let key = if !self.config.general.setup_completed {
            "&Finish setup"
        } else if self.owner.is_none() {
            "&Save"
        } else {
            "&Save and restart"
        };
        set_text(hwnd, 1, crate::i18n::text(key));
        set_text(hwnd, ID_PATH, &self.source.path.display().to_string());
        update_opacity(hwnd, ID_DOCK_OPACITY);
        update_opacity(hwnd, ID_TOP_BAR_OPACITY);
        let _ = unsafe { InvalidateRect(Some(hwnd), None, true) };
    }

    fn fill_pins(&self, hwnd: HWND, selection: Option<usize>) {
        control_message(hwnd, ID_PINS, LB_RESETCONTENT, 0, 0);
        for pin in &self.config.pinned {
            add_text(hwnd, ID_PINS, LB_ADDSTRING, &pin.name);
        }
        if let Some(index) = selection {
            control_message(hwnd, ID_PINS, LB_SETCURSEL, index, 0);
        }
        self.select(hwnd);
    }

    fn select(&self, hwnd: HWND) {
        let selected = selected_pin(hwnd);
        let pin = selected.and_then(|index| self.config.pinned.get(index));
        set_text(hwnd, ID_NAME, pin.map_or("", |pin| &pin.name));
        set_text(hwnd, ID_TARGET, pin.map_or("", |pin| &pin.target));
        for (id, enabled) in [
            (ID_UP, selected.is_some_and(|i| i > 0)),
            (
                ID_DOWN,
                selected.is_some_and(|i| i + 1 < self.config.pinned.len()),
            ),
            (ID_REMOVE, pin.is_some()),
            (ID_UPDATE, pin.is_some()),
        ] {
            if let Ok(control) = unsafe { GetDlgItem(Some(hwnd), id) } {
                let _ = unsafe {
                    windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(control, enabled)
                };
            }
        }
    }

    fn edit_pin(&mut self, hwnd: HWND, index: Option<usize>) -> Result<()> {
        let pin = read_pin(hwnd);
        if pin.name.trim().is_empty() || pin.target.trim().is_empty() {
            return Err(Error::new(
                E_INVALIDARG,
                crate::i18n::text("Enter both a name and a path or App ID."),
            ));
        }
        if self.config.pinned.iter().enumerate().any(|(i, existing)| {
            Some(i) != index && existing.target.eq_ignore_ascii_case(pin.target.trim())
        }) {
            return Err(Error::new(
                E_INVALIDARG,
                crate::i18n::text("This app is already pinned. Select it to update its target."),
            ));
        }
        if let Some(index) = index {
            self.config.pinned[index] = pin;
            self.fill_pins(hwnd, Some(index));
        } else {
            if self.config.pinned.len() >= MAX_PINS {
                return Err(Error::new(
                    E_INVALIDARG,
                    crate::i18n::text("The dock supports up to 32 pinned apps."),
                ));
            }
            self.config.pinned.push(pin);
            self.fill_pins(hwnd, Some(self.config.pinned.len() - 1));
        }
        Ok(())
    }

    fn save(&mut self, hwnd: HWND) -> Result<()> {
        let appearance = Appearance {
            theme: match control_message(hwnd, ID_THEME, CB_GETCURSEL, 0, 0) {
                1 => crate::theme::ThemeMode::Light,
                2 => crate::theme::ThemeMode::Dark,
                _ => crate::theme::ThemeMode::Auto,
            },
            size_percent: read_number(hwnd, ID_DOCK_SIZE, "dock size")?,
            icon_size: read_number(hwnd, ID_ICON_SIZE, "icon size")?,
            icon_spacing: read_number(hwnd, ID_ICON_SPACING, "icon spacing")?,
            edge_padding: read_number(hwnd, ID_EDGE_PADDING, "edge padding")?,
            label_delay_ms: read_number(hwnd, ID_LABEL_DELAY, "app label delay")?,
            zoom_percent: read_number(hwnd, ID_HOVER_ZOOM, "hover zoom")?,
            animation_speed_percent: read_number(hwnd, ID_ANIMATION_SPEED, "animation speed")?,
            auto_hide_delay_ms: read_number(hwnd, ID_HIDE_DELAY, "auto-hide delay")?,
            dock_opacity_percent: opacity(hwnd, ID_DOCK_OPACITY),
            top_bar_opacity_percent: opacity(hwnd, ID_TOP_BAR_OPACITY),
            launch_bounce: checked(hwnd, ID_LAUNCH_BOUNCE),
        };
        appearance
            .validate()
            .map_err(|error| Error::new(E_INVALIDARG, error))?;
        let selection = selected_pin(hwnd);
        let draft = read_pin(hwnd);
        if selection.map_or(
            !draft.name.is_empty() || !draft.target.is_empty(),
            |index| self.config.pinned[index] != draft,
        ) {
            self.edit_pin(hwnd, selection)?;
        }
        let display = control_message(hwnd, ID_DISPLAY, CB_GETCURSEL, 0, 0);
        if let Some(display) = self.displays.get(display as usize) {
            self.config.placement.monitor.clone_from(display);
        }
        let mode = control_message(hwnd, ID_MODE, CB_GETCURSEL, 0, 0);
        self.config.placement.reserve_space = mode == 1;
        self.config.placement.auto_hide = mode == 2;
        self.config.dock.show_start = checked(hwnd, ID_START);
        self.config.dock.show_search = checked(hwnd, ID_SEARCH);
        self.config.dock.show_downloads = checked(hwnd, ID_DOWNLOADS);
        self.config.top_bar.enabled = checked(hwnd, ID_TOP_BAR);
        self.config.top_bar.show_tray = checked(hwnd, ID_TRAY);
        self.config.placement.hide_on_fullscreen = checked(hwnd, ID_FULLSCREEN);
        self.config.placement.hide_taskbar = checked(hwnd, ID_TASKBAR);
        self.config.appearance = appearance;
        let language = control_message(hwnd, ID_LANGUAGE, CB_GETCURSEL, 0, 0);
        self.config.general.language = language
            .checked_sub(1)
            .and_then(|index| crate::i18n::LANGUAGES.get(index as usize))
            .map_or("auto", |language| language.code)
            .into();
        let previous_startup = crate::startup::enabled();
        let startup = checked(hwnd, ID_STARTUP);
        if previous_startup != startup {
            crate::startup::set_enabled(startup)?;
        }
        let setup_completed = self.config.general.setup_completed;
        self.config.general.setup_completed = true;
        if let Err(error) = self.source.save(&self.config) {
            self.config.general.setup_completed = setup_completed;
            if previous_startup != startup {
                if let Err(rollback) = crate::startup::set_enabled(previous_startup) {
                    return Err(Error::new(E_FAIL, format!("{error}\n\n{rollback}")));
                }
            }
            return Err(error);
        }
        self.saved = true;
        if let Some(owner) = self.owner {
            unsafe { PostMessageW(Some(owner), WM_SETTINGS_SAVED, WPARAM(0), LPARAM(0))? };
        }
        unsafe { DestroyWindow(hwnd)? };
        Ok(())
    }

    fn command(&mut self, hwnd: HWND, id: i32) -> Result<()> {
        let selection = selected_pin(hwnd);
        match id {
            1 => self.save(hwnd)?,
            ID_DEFAULTS => {
                self.config = self.source.defaults()?;
                self.fill(hwnd);
                crate::i18n::configure(&self.config.general.language);
                self.translate(hwnd);
            }
            ID_PINS => self.select(hwnd),
            ID_BROWSE => browse(hwnd)?,
            ID_INSTALLED => {
                if let Some(pin) = crate::app_picker::show(hwnd)? {
                    set_text(hwnd, ID_NAME, &pin.name);
                    set_text(hwnd, ID_TARGET, &pin.target);
                }
                if let Some(owner) = self.owner {
                    unsafe {
                        let _ = PostMessageW(
                            Some(owner),
                            crate::shell::WM_SHELL_RESULT,
                            WPARAM(0),
                            LPARAM(0),
                        );
                        let _ = PostMessageW(
                            Some(owner),
                            crate::watcher::WM_APPS_CHANGED,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
            ID_ADD => self.edit_pin(hwnd, None)?,
            ID_UPDATE if selection.is_some() => self.edit_pin(hwnd, selection)?,
            ID_UP | ID_DOWN | ID_REMOVE if let Some(index) = selection => {
                if id == ID_REMOVE {
                    self.config.pinned.remove(index);
                    let next = self
                        .config
                        .pinned
                        .len()
                        .checked_sub(1)
                        .map(|last| index.min(last));
                    self.fill_pins(hwnd, next);
                } else {
                    let next = if id == ID_UP {
                        index.checked_sub(1)
                    } else {
                        Some(index + 1)
                    };
                    if let Some(next) = next.filter(|&index| index < self.config.pinned.len()) {
                        self.config.pinned.swap(index, next);
                        self.fill_pins(hwnd, Some(next));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn checked(hwnd: HWND, id: i32) -> bool {
    unsafe { IsDlgButtonChecked(hwnd, id) == BST_CHECKED.0 }
}

fn selected_pin(hwnd: HWND) -> Option<usize> {
    usize::try_from(control_message(hwnd, ID_PINS, LB_GETCURSEL, 0, 0)).ok()
}

fn control_message(hwnd: HWND, id: i32, message: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendDlgItemMessageW(hwnd, id, message, WPARAM(wparam), LPARAM(lparam)).0 }
}

fn add_text(hwnd: HWND, id: i32, message: u32, text: &str) {
    let text = if id == ID_PINS {
        text
    } else {
        crate::i18n::text(text)
    };
    let text = HSTRING::from(text);
    control_message(hwnd, id, message, 0, text.as_ptr() as isize);
}

fn set_text(hwnd: HWND, id: i32, text: &str) {
    let _ = unsafe { SetDlgItemTextW(hwnd, id, &HSTRING::from(text)) };
}

fn read_text(hwnd: HWND, id: i32) -> String {
    let Ok(control) = (unsafe { GetDlgItem(Some(hwnd), id) }) else {
        return String::new();
    };
    let length = unsafe { GetWindowTextLengthW(control) };
    let mut buffer = vec![0; length as usize + 1];
    let length = unsafe { GetWindowTextW(control, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length as usize])
}

fn read_pin(hwnd: HWND) -> Pin {
    Pin {
        name: read_text(hwnd, ID_NAME),
        target: read_text(hwnd, ID_TARGET),
    }
}

fn read_number(hwnd: HWND, id: i32, label: &str) -> Result<u32> {
    read_text(hwnd, id).trim().parse().map_err(|_| {
        Error::new(
            E_INVALIDARG,
            crate::tr!(
                "Enter a whole number for {label}.",
                label = crate::i18n::text(label)
            ),
        )
    })
}

fn opacity(hwnd: HWND, id: i32) -> u32 {
    control_message(hwnd, id, TBM_GETPOS, 0, 0) as u32
}

fn update_opacity(hwnd: HWND, id: i32) {
    let value = if id == ID_DOCK_OPACITY {
        ID_DOCK_OPACITY_VALUE
    } else {
        ID_TOP_BAR_OPACITY_VALUE
    };
    set_text(hwnd, value, &format!("{}%", opacity(hwnd, id)));
}

fn browse(hwnd: HWND) -> Result<()> {
    let mut path = [0u16; 4096];
    let filter = HSTRING::from(format!(
        "{}\0*.exe;*.lnk\0{}\0*.*\0\0",
        crate::i18n::text("Applications and shortcuts"),
        crate::i18n::text("All files")
    ));
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(path.as_mut_ptr()),
        nMaxFile: path.len() as u32,
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_EXPLORER,
        ..Default::default()
    };
    if unsafe { GetOpenFileNameW(&mut dialog).as_bool() } {
        let length = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        let target = String::from_utf16_lossy(&path[..length]);
        set_text(hwnd, ID_TARGET, &target);
        if read_text(hwnd, ID_NAME).trim().is_empty()
            && let Some(name) = Path::new(&target)
                .file_stem()
                .and_then(|name| name.to_str())
        {
            set_text(hwnd, ID_NAME, name);
        }
    } else if unsafe { CommDlgExtendedError().0 } != 0 {
        return Err(Error::new(
            E_FAIL,
            crate::i18n::text("Could not open the file picker."),
        ));
    }
    Ok(())
}

unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match message {
        WM_INITDIALOG => {
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, lparam.0) };
            let state = unsafe { &mut *(lparam.0 as *mut State) };
            state.captions = crate::i18n::captions(hwnd);
            state.style(hwnd);
            for (label, ..) in PAGES {
                add_text(hwnd, ID_PAGES, LB_ADDSTRING, label);
            }
            control_message(hwnd, ID_PAGES, LB_SETCURSEL, 0, 0);
            state.fill(hwnd);
            state.translate(hwnd);
            1
        }
        WM_NCDESTROY => {
            let data = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const State;
            if let Some(state) = unsafe { data.as_ref() } {
                crate::i18n::configure(if state.saved {
                    &state.config.general.language
                } else {
                    &state.original_language
                });
                crate::theme::configure(state.config.appearance.theme);
                if let Some(owner) = state.owner {
                    unsafe {
                        let _ = PostMessageW(
                            Some(owner),
                            crate::placement::WM_REPOSITION,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            0
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
            1
        }
        WM_DPICHANGED | WM_SETTINGCHANGE => {
            crate::theme::refresh();
            let data = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
            if let Some(state) = unsafe { data.as_mut() } {
                state.style(hwnd);
            }
            0
        }
        WM_ERASEBKGND | WM_CTLCOLORDLG | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX
        | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
            let data = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const State;
            let Some(skin) = (unsafe { data.as_ref() }).and_then(|state| state.skin.as_ref())
            else {
                return 0;
            };
            let dc = HDC(wparam.0 as *mut _);
            if message == WM_ERASEBKGND {
                SettingsSkin::background(hwnd, dc);
                return 1;
            }
            if message == WM_CTLCOLORDLG {
                return skin.panel.0 as isize;
            }
            let id = unsafe { GetDlgCtrlID(HWND(lparam.0 as *mut _)) };
            if message == WM_CTLCOLORLISTBOX && id != ID_PAGES {
                return 0;
            }
            let sidebar = matches!(id, ID_BRAND | ID_PAGES);
            unsafe {
                SetBkColor(dc, popup::adapted(if sidebar { SIDEBAR } else { PANEL }));
                SetTextColor(
                    dc,
                    popup::adapted(
                        if matches!(id, ID_PAGE_DESCRIPTION | ID_PATH | 211 | 213 | 215 | 217) {
                            MUTED
                        } else {
                            INK
                        },
                    ),
                );
            }
            if sidebar {
                skin.sidebar.0 as isize
            } else {
                skin.panel.0 as isize
            }
        }
        WM_DRAWITEM if wparam.0 as i32 == ID_PAGES && lparam.0 != 0 => {
            let data = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const State;
            if let Some(state) = unsafe { data.as_ref() } {
                state.draw_page(hwnd, unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) });
            }
            1
        }
        WM_HSCROLL if lparam.0 != 0 => {
            let id = unsafe { GetDlgCtrlID(HWND(lparam.0 as *mut _)) };
            if matches!(id, ID_DOCK_OPACITY | ID_TOP_BAR_OPACITY) {
                update_opacity(hwnd, id);
            }
            0
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as i32;
            let code = (wparam.0 >> 16) as u32;
            if id == ID_LANGUAGE && code == CBN_SELCHANGE {
                let selection = control_message(hwnd, ID_LANGUAGE, CB_GETCURSEL, 0, 0);
                let code = selection
                    .checked_sub(1)
                    .and_then(|index| crate::i18n::LANGUAGES.get(index as usize))
                    .map_or("auto", |language| language.code);
                crate::i18n::configure(code);
                let data = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
                if let Some(state) = unsafe { data.as_mut() } {
                    state.translate(hwnd);
                }
                return 1;
            }
            if id == ID_THEME && code == CBN_SELCHANGE {
                let mode = match control_message(hwnd, ID_THEME, CB_GETCURSEL, 0, 0) {
                    1 => crate::theme::ThemeMode::Light,
                    2 => crate::theme::ThemeMode::Dark,
                    _ => crate::theme::ThemeMode::Auto,
                };
                crate::theme::configure(mode);
                let data = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
                if let Some(state) = unsafe { data.as_mut() } {
                    state.style(hwnd);
                    if let Some(owner) = state.owner {
                        unsafe {
                            let _ = PostMessageW(
                                Some(owner),
                                crate::placement::WM_REPOSITION,
                                WPARAM(0),
                                LPARAM(0),
                            );
                        }
                    }
                }
                return 1;
            }
            if id == ID_PAGES && code == LBN_SELCHANGE {
                let page = control_message(hwnd, ID_PAGES, LB_GETCURSEL, 0, 0) as usize;
                State::show_page(hwnd, page);
                return 1;
            }
            if id == 2 {
                let _ = unsafe { DestroyWindow(hwnd) };
                return 1;
            }
            let button = matches!(
                id,
                1 | ID_UP
                    | ID_DOWN
                    | ID_REMOVE
                    | ID_BROWSE
                    | ID_INSTALLED
                    | ID_ADD
                    | ID_UPDATE
                    | ID_DEFAULTS
            );
            if !(button && code == BN_CLICKED || id == ID_PINS && code == LBN_SELCHANGE) {
                return 0;
            }
            let data = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
            if let Some(state) = unsafe { data.as_mut() }
                && let Err(error) = state.command(hwnd, id)
            {
                unsafe {
                    MessageBoxW(
                        Some(hwnd),
                        &HSTRING::from(error.message()),
                        &HSTRING::from(crate::i18n::text("Styled Dock Settings")),
                        MB_OK | MB_ICONWARNING,
                    )
                };
            }
            1
        }
        _ => 0,
    }
}

unsafe extern "system" fn theme_control(hwnd: HWND, _: LPARAM) -> windows::core::BOOL {
    unsafe {
        let _ = SetWindowTheme(
            hwnd,
            if crate::theme::dark() && !popup::high_contrast() {
                w!("DarkMode_Explorer")
            } else {
                w!("Explorer")
            },
            None,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
    true.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_preview_preserves_edits_and_cancel_does_not_save() -> Result<()> {
        let settings = SettingsWindow::open(unsafe { GetModuleHandleW(None)? }.into(), None)?;
        let hwnd = settings.hwnd;
        let data = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
        let original_language = unsafe { (*data).original_language.clone() };
        let path = unsafe { (*data).source.path.clone() };
        let original_file = std::fs::read(&path).ok();
        set_text(hwnd, ID_DOCK_SIZE, "125");
        set_text(hwnd, ID_NAME, "Downloads"); // User-owned names must stay intact.
        for (selection, code, cancel) in [(2, "tr", "İptal"), (1, "en", "Cancel")] {
            control_message(hwnd, ID_LANGUAGE, CB_SETCURSEL, selection, 0);
            unsafe {
                SendMessageW(
                    hwnd,
                    WM_COMMAND,
                    Some(WPARAM(
                        ID_LANGUAGE as usize | ((CBN_SELCHANGE as usize) << 16),
                    )),
                    None,
                );
            }
            assert_eq!(read_text(hwnd, 2), cancel);
            assert_eq!(read_text(hwnd, ID_DOCK_SIZE), "125");
            assert_eq!(read_text(hwnd, ID_NAME), "Downloads");
            assert_eq!(crate::i18n::text("Cancel"), cancel, "{code}");
            for (page, (_, title, _, ids)) in PAGES.iter().enumerate() {
                State::show_page(hwnd, page);
                assert_eq!(read_text(hwnd, ID_PAGE_TITLE), crate::i18n::text(title));
                assert!(unsafe { IsWindowVisible(GetDlgItem(Some(hwnd), ids[0])?) }.as_bool());
            }
        }
        unsafe {
            SendMessageW(hwnd, WM_CLOSE, None, None);
        }
        assert!(!settings._state.saved);
        assert_eq!(std::fs::read(path).ok(), original_file);
        crate::i18n::configure(&original_language);
        Ok(())
    }

    #[test]
    fn settings_opens_and_draws_every_page() -> Result<()> {
        let settings = SettingsWindow::open(unsafe { GetModuleHandleW(None)? }.into(), None)?;
        let hwnd = settings.hwnd;
        let sidebar = unsafe { GetDlgItem(Some(hwnd), ID_PAGES)? };
        assert_eq!(
            control_message(hwnd, ID_PAGES, LB_GETCOUNT, 0, 0),
            PAGES.len() as isize
        );

        // Exercise the custom painter even on a high-contrast desktop.
        let data = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
        unsafe { (*data).skin = Some(SettingsSkin::new(GetDpiForWindow(hwnd) as i32)) };
        let dc = unsafe { GetDC(Some(sidebar)) };
        assert!(!dc.is_invalid());
        for (page, (_, title, _, ids)) in PAGES.iter().enumerate() {
            control_message(hwnd, ID_PAGES, LB_SETCURSEL, page, 0);
            unsafe {
                SendMessageW(
                    hwnd,
                    WM_COMMAND,
                    Some(WPARAM(ID_PAGES as usize | ((LBN_SELCHANGE as usize) << 16))),
                    Some(LPARAM(sidebar.0 as isize)),
                );
            }
            assert_eq!(read_text(hwnd, ID_PAGE_TITLE), crate::i18n::text(title));
            assert!(unsafe { IsWindowVisible(GetDlgItem(Some(hwnd), ids[0])?) }.as_bool());
            for item_state in [ODS_SELECTED, ODS_FOCUS] {
                let item = DRAWITEMSTRUCT {
                    CtlID: ID_PAGES as u32,
                    itemID: page as u32,
                    itemState: item_state,
                    hwndItem: sidebar,
                    hDC: dc,
                    rcItem: RECT {
                        right: 200,
                        bottom: 36,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                unsafe {
                    SendMessageW(
                        hwnd,
                        WM_DRAWITEM,
                        Some(WPARAM(ID_PAGES as usize)),
                        Some(LPARAM((&raw const item) as isize)),
                    );
                }
            }
        }
        unsafe { ReleaseDC(Some(sidebar), dc) };
        unsafe { SendMessageW(hwnd, WM_CLOSE, None, None) };
        assert!(!settings.is_open());
        Ok(())
    }
}
