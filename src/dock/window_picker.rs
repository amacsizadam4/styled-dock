use std::cell::Cell;
use std::rc::Rc;

use windows::Win32::Foundation::{
    COLORREF, E_BOUNDS, E_FAIL, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_RECTDESTINATION, DWM_TNP_VISIBLE,
    DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUnregisterThumbnail,
    DwmUpdateThumbnailProperties,
};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Error, HSTRING, Result, w};

use crate::popup::{self, Font, MENU_BORDER, MENU_SURFACE, rgb};
use crate::watcher;

pub(super) struct Picker {
    hwnd: HWND,
    pub target: String,
    previous: HWND,
    panel: Rc<Panel>,
    thumbnails: Vec<Thumbnail>,
}

pub(super) enum Input {
    Ignored,
    Handled,
    Dismiss,
    Choose(HWND),
}

struct Card {
    window: HWND,
    title: String,
    active: bool,
    minimized: bool,
}

struct Panel {
    cards: Vec<Card>,
    selected: Cell<usize>,
    dismissed: Cell<bool>,
    font: Font,
    small_font: Font,
    dpi: i32,
    padding: i32,
    gap: i32,
    columns: usize,
    card_width: i32,
    preview_height: i32,
    width: i32,
    height: i32,
    high_contrast: bool,
    fallback_icon: HICON,
}

impl Picker {
    pub fn open(owner: HWND, anchor: POINT, target: &str, windows: &[HWND]) -> Result<Self> {
        unsafe {
            let previous = GetForegroundWindow();
            let active_owner = GetAncestor(previous, GA_ROOTOWNER);
            let cards: Vec<_> = windows
                .iter()
                .copied()
                .filter(|&window| IsWindow(Some(window)).as_bool())
                .map(|window| Card {
                    window,
                    title: watcher::title(window),
                    active: window == previous || window == active_owner,
                    minimized: IsIconic(window).as_bool(),
                })
                .collect();
            if cards.is_empty() {
                return Err(Error::from_hresult(E_FAIL));
            }
            let selected = cards.iter().position(|card| card.active).unwrap_or(0);
            let monitor = MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST);
            let work = crate::placement::monitor_info(monitor)?.monitorInfo.rcWork;
            let dpi = match GetDpiForWindow(owner) {
                0 => 96,
                dpi => dpi as i32,
            };
            let px = |dip| popup::scale(dip, dpi);
            let padding = px(10);
            let gap = px(8);
            let available_width = work.right - work.left - 2 * padding;
            let card_width = px(196).min(available_width);
            if card_width < px(80) || work.bottom - work.top < px(110) {
                return Err(Error::from_hresult(E_BOUNDS));
            }
            let columns = cards
                .len()
                .min(((available_width + gap) / (card_width + gap)) as usize);
            let paged = columns < cards.len();
            let preview_height =
                px(112).min(work.bottom - work.top - 2 * padding - px(if paged { 76 } else { 45 }));
            let width = 2 * padding + columns as i32 * (card_width + gap) - gap;
            let height = 2 * padding + preview_height + px(if paged { 76 } else { 45 });
            let x = (anchor.x - width / 2).clamp(work.left, work.right - width);
            let y = (anchor.y - px(8) - height).clamp(work.top, work.bottom - height);
            let panel = Rc::new(Panel {
                cards,
                selected: Cell::new(selected),
                dismissed: Cell::new(false),
                font: Font::new(13, dpi, FW_MEDIUM),
                small_font: Font::new(11, dpi, FW_NORMAL),
                dpi,
                padding,
                gap,
                columns,
                card_width,
                preview_height,
                width,
                height,
                high_contrast: popup::high_contrast(),
                fallback_icon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
            });
            let instance = GetModuleHandleW(None)?.into();
            let class = WNDCLASSW {
                style: CS_DROPSHADOW,
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpfnWndProc: Some(window_proc),
                lpszClassName: w!("StyledDockWindowPicker"),
                ..Default::default()
            };
            RegisterClassW(&class);
            // DWM renders the preview directly into this top-level window.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class.lpszClassName,
                &HSTRING::from(crate::i18n::text("Choose a window")),
                WS_POPUP,
                x,
                y,
                width,
                height,
                Some(owner),
                None,
                Some(instance),
                Some(Rc::as_ptr(&panel).cast()),
            )?;
            let mut picker = Self {
                hwnd,
                target: target.to_owned(),
                previous,
                panel,
                thumbnails: Vec::new(),
            };
            let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, px(20), px(20));
            if !region.is_invalid() && SetWindowRgn(hwnd, Some(region), false) == 0 {
                let _ = DeleteObject(region.into());
            }
            picker.show_page();
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            if GetForegroundWindow() != hwnd {
                return Err(Error::from_hresult(E_FAIL));
            }
            Ok(picker)
        }
    }

    pub fn dismissed(&self) -> bool {
        self.panel.dismissed.get()
    }

    pub fn matches(&self, windows: &[HWND]) -> bool {
        windows.len() == self.panel.cards.len()
            && self
                .panel
                .cards
                .iter()
                .all(|card| windows.contains(&card.window))
    }

    pub fn focus(&mut self, window: HWND) -> bool {
        if !unsafe { IsWindow(Some(window)).as_bool() } {
            return false;
        }
        // The target can take focus asynchronously, especially if it is minimized.
        // Closing the picker must not send focus back to the previous window.
        self.previous = HWND::default();
        watcher::focus(window);
        true
    }

    pub fn input(&mut self, message: &MSG) -> Input {
        if message.hwnd != self.hwnd {
            return Input::Ignored;
        }
        let panel = &self.panel;
        let selected = panel.selected.get();
        let last = panel.cards.len() - 1;
        let backward = || panel.first().saturating_sub(panel.columns);
        let forward = || (panel.first() + panel.columns).min(last);
        let next = match message.message {
            WM_KEYDOWN => match VIRTUAL_KEY(message.wParam.0 as u16) {
                VK_ESCAPE => return Input::Dismiss,
                VK_RETURN | VK_SPACE => return Input::Choose(panel.cards[selected].window),
                VK_LEFT | VK_UP => selected.checked_sub(1).unwrap_or(last),
                VK_RIGHT | VK_DOWN => (selected + 1) % panel.cards.len(),
                VK_HOME => 0,
                VK_END => last,
                VK_PRIOR => backward(),
                VK_NEXT => forward(),
                _ => return Input::Ignored,
            },
            WM_MOUSEWHEEL if panel.columns < panel.cards.len() => {
                if (message.wParam.0 >> 16) as i16 > 0 {
                    backward()
                } else {
                    forward()
                }
            }
            WM_MOUSEMOVE | WM_LBUTTONUP => {
                let point = POINT {
                    x: message.lParam.0 as i16 as i32,
                    y: (message.lParam.0 >> 16) as i16 as i32,
                };
                if let Some(index) = panel.hit_card(point) {
                    if message.message == WM_LBUTTONUP {
                        return Input::Choose(panel.cards[index].window);
                    }
                    index
                } else if message.message == WM_LBUTTONUP && panel.columns < panel.cards.len() {
                    let [left, right] = panel.page_buttons();
                    if contains(left, point) && panel.first() > 0 {
                        backward()
                    } else if contains(right, point) && panel.first() + panel.columns <= last {
                        forward()
                    } else {
                        return Input::Handled;
                    }
                } else {
                    return Input::Handled;
                }
            }
            _ => return Input::Ignored,
        };
        let first = panel.first();
        if panel.selected.replace(next) != next {
            if first != panel.first() {
                self.show_page();
            }
            let _ = unsafe { InvalidateRect(Some(self.hwnd), None, false) };
        }
        Input::Handled
    }

    fn show_page(&mut self) {
        self.thumbnails.clear();
        let first = self.panel.first();
        let visible = self.panel.cards.iter().skip(first).take(self.panel.columns);
        self.thumbnails
            .extend(visible.enumerate().filter_map(|(column, card)| {
                Thumbnail::new(self.hwnd, card.window, self.panel.preview_rect(column)).ok()
            }));
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        self.thumbnails.clear();
        unsafe {
            if GetForegroundWindow() == self.hwnd && IsWindow(Some(self.previous)).as_bool() {
                let _ = SetForegroundWindow(self.previous);
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl Panel {
    fn px(&self, dip: i32) -> i32 {
        popup::scale(dip, self.dpi)
    }

    fn first(&self) -> usize {
        self.selected.get() / self.columns * self.columns
    }

    fn card_rect(&self, column: usize) -> RECT {
        let left = self.padding + column as i32 * (self.card_width + self.gap);
        RECT {
            left,
            top: self.padding,
            right: left + self.card_width,
            bottom: self.padding + self.preview_height + self.px(45),
        }
    }

    fn preview_rect(&self, column: usize) -> RECT {
        let mut rect = self.card_rect(column);
        let inset = self.px(6);
        rect.left += inset;
        rect.right -= inset;
        rect.top += inset;
        rect.bottom = self.padding + self.preview_height;
        rect
    }

    fn hit_card(&self, point: POINT) -> Option<usize> {
        let first = self.first();
        (0..self.columns.min(self.cards.len() - first))
            .find(|&column| contains(self.card_rect(column), point))
            .map(|column| first + column)
    }

    fn page_buttons(&self) -> [RECT; 2] {
        let top = self.card_rect(0).bottom + self.px(5);
        let bottom = self.height - self.padding;
        let button_width = self.px(44);
        [
            RECT {
                left: self.padding,
                right: self.padding + button_width,
                top,
                bottom,
            },
            RECT {
                left: self.width - self.padding - button_width,
                right: self.width - self.padding,
                top,
                bottom,
            },
        ]
    }

    fn paint(&self, dc: HDC) {
        let (surface, tile, text, detail, highlight, selected_text) = if self.high_contrast {
            unsafe {
                (
                    COLORREF(GetSysColor(COLOR_WINDOW)),
                    COLORREF(GetSysColor(COLOR_WINDOW)),
                    COLORREF(GetSysColor(COLOR_WINDOWTEXT)),
                    COLORREF(GetSysColor(COLOR_WINDOWTEXT)),
                    COLORREF(GetSysColor(COLOR_HIGHLIGHT)),
                    COLORREF(GetSysColor(COLOR_HIGHLIGHTTEXT)),
                )
            }
        } else {
            (
                MENU_SURFACE,
                rgb(0x202e44),
                rgb(0xf6f8ff),
                rgb(0xb3c0d5),
                rgb(0x90c2ff),
                rgb(0xf6f8ff),
            )
        };
        let bounds = RECT {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height,
        };
        popup::fill(dc, &bounds, surface);
        popup::rounded(
            dc,
            &bounds,
            if self.high_contrast {
                text
            } else {
                MENU_BORDER
            },
            self.px(20),
        );
        popup::rounded(dc, &inset(bounds, 1), surface, self.px(19));
        let first = self.first();
        for (column, card) in self.cards.iter().skip(first).take(self.columns).enumerate() {
            let selected = first + column == self.selected.get();
            let rect = self.card_rect(column);
            if selected || card.active {
                let border = if selected { highlight } else { detail };
                popup::rounded(dc, &rect, border, self.px(12));
            }
            let background = if selected && self.high_contrast {
                highlight
            } else {
                tile
            };
            popup::rounded(dc, &inset(rect, self.px(2)), background, self.px(10));

            let preview = self.preview_rect(column);
            popup::rounded(dc, &preview, surface, self.px(8));
            // The icon remains visible if DWM has no frame to composite.
            self.draw_icon(dc, card.window, preview);

            let left = rect.left + self.px(10);
            let mut title = RECT {
                left,
                right: rect.right - self.px(10),
                top: preview.bottom + self.px(4),
                bottom: preview.bottom + self.px(22),
            };
            let label_color = if selected { selected_text } else { text };
            popup::draw_text(dc, &card.title, &mut title, self.font.0, label_color);
            if card.active || card.minimized {
                let status = crate::i18n::text(if card.active { "Active" } else { "Minimized" });
                let mut sub = RECT {
                    top: title.bottom,
                    bottom: rect.bottom - self.px(5),
                    ..title
                };
                let status_color = if selected && self.high_contrast {
                    selected_text
                } else {
                    detail
                };
                popup::draw_text(dc, status, &mut sub, self.small_font.0, status_color);
            }
        }
        if self.columns < self.cards.len() {
            let [mut left, mut right] = self.page_buttons();
            if first > 0 {
                popup::rounded(dc, &left, tile, self.px(8));
                left.left += self.px(15);
                popup::draw_text(dc, "‹", &mut left, self.font.0, text);
            }
            if first + self.columns < self.cards.len() {
                popup::rounded(dc, &right, tile, self.px(8));
                right.left += self.px(15);
                popup::draw_text(dc, "›", &mut right, self.font.0, text);
            }
        }
    }

    fn draw_icon(&self, dc: HDC, window: HWND, rect: RECT) {
        unsafe {
            let icon = [GCLP_HICON, GCLP_HICONSM]
                .into_iter()
                .map(|index| HICON(GetClassLongPtrW(window, index) as *mut _))
                .find(|icon| !icon.is_invalid())
                .unwrap_or(self.fallback_icon);
            let size = self.px(32).min(rect.bottom - rect.top);
            let x = rect.left + (rect.right - rect.left - size) / 2;
            let y = rect.top + (rect.bottom - rect.top - size) / 2;
            let _ = DrawIconEx(dc, x, y, icon, size, size, 0, None, DI_NORMAL);
        }
    }
}

struct Thumbnail(isize);

impl Thumbnail {
    fn new(destination: HWND, source: HWND, bounds: RECT) -> Result<Self> {
        unsafe {
            let thumbnail = Self(DwmRegisterThumbnail(destination, source)?);
            let size = DwmQueryThumbnailSourceSize(thumbnail.0)?;
            let rect = fit(bounds, size).ok_or_else(|| Error::from_hresult(E_BOUNDS))?;
            DwmUpdateThumbnailProperties(
                thumbnail.0,
                &DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE,
                    rcDestination: rect,
                    fVisible: true.into(),
                    ..Default::default()
                },
            )?;
            Ok(thumbnail)
        }
    }
}

impl Drop for Thumbnail {
    fn drop(&mut self) {
        let _ = unsafe { DwmUnregisterThumbnail(self.0) };
    }
}

fn fit(bounds: RECT, source: SIZE) -> Option<RECT> {
    if source.cx <= 0 || source.cy <= 0 {
        return None;
    }
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    let scale =
        (f64::from(width) / f64::from(source.cx)).min(f64::from(height) / f64::from(source.cy));
    let width_fit = ((f64::from(source.cx) * scale).round() as i32).max(1);
    let height_fit = ((f64::from(source.cy) * scale).round() as i32).max(1);
    let left = bounds.left + (width - width_fit) / 2;
    let top = bounds.top + (height - height_fit) / 2;
    Some(RECT {
        left,
        top,
        right: left + width_fit,
        bottom: top + height_fit,
    })
}

fn inset(rect: RECT, amount: i32) -> RECT {
    RECT {
        left: rect.left + amount,
        top: rect.top + amount,
        right: rect.right - amount,
        bottom: rect.bottom - amount,
    }
}

fn contains(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let panel = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Panel).as_ref();
        match (message, panel) {
            (WM_PAINT, Some(panel)) => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                panel.paint(dc);
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            (WM_ERASEBKGND, _) => LRESULT(1),
            (WM_ACTIVATE | WM_CLOSE, Some(panel))
                if message == WM_CLOSE || wparam.0 & 0xffff == WA_INACTIVE as usize =>
            {
                panel.dismissed.set(true);
                let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                LRESULT(0)
            }
            (WM_NCDESTROY, _) => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_fit_wide_and_tall_windows_without_stretching() {
        let bounds = RECT {
            left: 10,
            top: 20,
            right: 210,
            bottom: 120,
        };
        assert_eq!(
            fit(bounds, SIZE { cx: 1600, cy: 600 }),
            Some(RECT {
                left: 10,
                top: 32,
                right: 210,
                bottom: 107
            }),
        );
        assert_eq!(
            fit(bounds, SIZE { cx: 600, cy: 1200 }),
            Some(RECT {
                left: 85,
                top: 20,
                right: 135,
                bottom: 120
            }),
        );
        assert_eq!(fit(bounds, SIZE::default()), None);
    }
}
