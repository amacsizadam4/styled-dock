use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::Result;

use crate::config::Appearance;

pub const BOTTOM_GAP: f32 = 12.0;

#[derive(Clone, Copy, PartialEq)]
pub struct DockSize {
    pub factor: f32,
    pub zoom: f32,
    pub icon: f32,
    pub spacing: f32,
    pub padding: f32,
}

impl DockSize {
    pub fn new(appearance: Appearance) -> Self {
        Self {
            factor: appearance.size_percent as f32 / 100.0,
            zoom: appearance.zoom_percent as f32 / 100.0,
            icon: appearance.icon_size as f32,
            spacing: appearance.icon_spacing as f32,
            padding: appearance.edge_padding as f32,
        }
    }

    pub fn panel_width(self, count: usize) -> f32 {
        (count as f32 * (self.icon + self.spacing) - self.spacing + self.padding * 2.0).max(96.0)
            * self.factor
    }

    pub fn panel_height(self) -> f32 {
        (self.icon + 28.0) * self.factor
    }

    pub fn side_room(self) -> f32 {
        (self.icon * 2.0 + 8.0) * (self.zoom / 0.68).max(1.0) * self.factor
    }

    pub fn headroom(self) -> f32 {
        (self.icon * self.zoom + 16.0) * self.factor + 58.0
    }

    pub fn radius(self) -> f32 {
        14.0 * self.factor
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub count: usize,
    pub pixels: (u32, u32),
    pub dock: DockSize,
}

impl Layout {
    pub fn for_window(hwnd: HWND, count: usize, dock: DockSize) -> Result<Self> {
        let mut rect = RECT::default();
        unsafe { GetClientRect(hwnd, &mut rect)? };
        let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
        let pixels = (rect.right as u32, rect.bottom as u32);
        Ok(Self {
            width: pixels.0 as f32 / scale,
            height: pixels.1 as f32 / scale,
            scale,
            count,
            pixels,
            dock,
        })
    }

    pub fn icon(self, index: usize) -> (f32, f32, f32) {
        let size = self.icon_size();
        let gap = self.gap();
        let total = self.count as f32 * (size + gap) - gap;
        let left = (self.width - total) / 2.0 + index as f32 * (size + gap);
        (
            left,
            self.panel_top() + (self.panel_height() - size) / 2.0,
            size,
        )
    }

    pub fn panel_height(self) -> f32 {
        (self.height - BOTTOM_GAP).clamp(1.0, self.dock.panel_height())
    }

    pub fn panel_top(self) -> f32 {
        (self.height - self.panel_height() - BOTTOM_GAP).max(0.0)
    }

    pub fn base_width(self) -> f32 {
        let desired = self.dock.panel_width(self.count);
        desired.min(self.width * desired / (desired + self.dock.side_room()))
    }

    pub fn icon_size(self) -> f32 {
        (self.dock.icon * self.dock.factor * self.base_width() / self.dock.panel_width(self.count))
            .min((self.panel_height() - 8.0 * self.dock.factor).max(1.0))
    }

    pub fn gap(self) -> f32 {
        self.dock.spacing * self.dock.factor * self.base_width() / self.dock.panel_width(self.count)
    }
}

#[derive(Clone, Copy)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Bounds {
    pub fn pixels(self, scale: f32) -> RECT {
        let px = |dip: f32| (dip * scale).round() as i32;
        RECT {
            left: px(self.x),
            top: px(self.y),
            right: px(self.x + self.width),
            bottom: px(self.y + self.height),
        }
    }
}

pub struct Scene {
    pub panel: Bounds,
    pub icons: Vec<Bounds>,
}

impl Scene {
    pub fn overlay(&self, layout: Layout, index: usize) -> Bounds {
        let icon = self.icons[index];
        let width = (layout.width - 12.0).clamp(1.0, 222.0);
        Bounds {
            x: (icon.x + (icon.width - width) / 2.0).clamp(0.0, (layout.width - width).max(0.0)),
            y: (icon.y - 56.0).max(0.0),
            width,
            height: 49.0,
        }
    }

    pub fn hit(&self, point: POINT, scale: f32) -> Option<usize> {
        self.icons.iter().position(|icon| {
            let rect = icon.pixels(scale);
            point.x >= rect.left
                && point.x < rect.right
                && point.y >= rect.top
                && point.y < rect.bottom
        })
    }

    pub fn insertion(&self, point: POINT, scale: f32) -> Option<(usize, f32)> {
        let first = self.icons.first()?;
        let last = self.icons.last()?;
        let x = point.x as f32 / scale;
        let y = point.y as f32 / scale;
        let inside_panel = x >= self.panel.x
            && x < self.panel.x + self.panel.width
            && y >= self.panel.y
            && y < self.panel.y + self.panel.height;
        if !inside_panel && self.hit(point, scale).is_none() {
            return None;
        }
        let slot = self
            .icons
            .partition_point(|icon| x >= icon.x + icon.width / 2.0);
        let marker = if slot == 0 {
            (self.panel.x + first.x) / 2.0
        } else if slot == self.icons.len() {
            (last.x + last.width + self.panel.x + self.panel.width) / 2.0
        } else {
            let left = self.icons[slot - 1];
            (left.x + left.width + self.icons[slot].x) / 2.0
        };
        Some((slot, marker))
    }
}
