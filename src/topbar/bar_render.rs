use std::collections::HashMap;
use std::ops::Range;
use std::time::Instant;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::core::{Error, Interface, Result};
use windows_numerics::{Matrix3x2, Vector2};

use crate::bar_status::Battery;
use crate::icons::{self, Symbol, VolumeLevel};
use crate::render::{Surface, color, rect};

pub const HEIGHT: i32 = 32;
const ICON: f32 = 20.0;
const PADDING: f32 = 14.0;
const LABEL_GAP: f32 = 5.0;

#[derive(Clone, Copy, PartialEq)]
pub enum BarIcon {
    System,
    Active,
    Wifi,
    Bluetooth,
    Display,
    Battery,
    Volume,
    Clock,
    QuickSettings,
    Tray(usize),
}

const ART: [(BarIcon, &[u8]); 8] = [
    (
        BarIcon::System,
        include_bytes!("../../assets/topbar/windows.svg"),
    ),
    (
        BarIcon::Wifi,
        include_bytes!("../../assets/topbar/wifi.svg"),
    ),
    (
        BarIcon::Bluetooth,
        include_bytes!("../../assets/topbar/bluetooth.svg"),
    ),
    (
        BarIcon::Display,
        include_bytes!("../../assets/topbar/display.svg"),
    ),
    (
        BarIcon::Battery,
        include_bytes!("../../assets/topbar/battery.svg"),
    ),
    (
        BarIcon::Volume,
        include_bytes!("../../assets/topbar/volume.svg"),
    ),
    (
        BarIcon::Clock,
        include_bytes!("../../assets/topbar/clock.svg"),
    ),
    (
        BarIcon::QuickSettings,
        include_bytes!("../../assets/topbar/quick-settings.svg"),
    ),
];

pub struct BarContents<'a> {
    pub tray: &'a [&'a crate::tray::TrayIcon],
    pub active: &'a str,
    pub clock: &'a str,
    pub battery: Battery,
    pub wifi_disconnected: bool,
    pub audio: Option<(f32, bool)>,
    pub hovered: Option<BarIcon>,
}

pub struct BarRenderer {
    surface: Surface,
    svg: ID2D1DeviceContext5,
    icons: Vec<(BarIcon, ID2D1SvgDocument)>,
    write: IDWriteFactory,
    text: IDWriteTextFormat,
    brush: ID2D1SolidColorBrush,
    opacity: f32,
    fades: Vec<(BarIcon, f32)>,
    last_frame: Instant,
    icons_dark: bool,
    tray_bitmaps: HashMap<u64, ID2D1Bitmap1>,
}

impl BarRenderer {
    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity.clamp(0.0, 1.0);
    }
    pub fn new(hwnd: HWND, opacity: f32) -> Result<Self> {
        let surface = Surface::new(hwnd, (1, 1))?;
        unsafe {
            surface
                .context
                .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let svg: ID2D1DeviceContext5 = surface.context.cast()?;
            let icons = ART
                .iter()
                .map(|&(icon, art)| {
                    let art = themed_art(art);
                    let stream = SHCreateMemStream(Some(&art)).ok_or_else(Error::empty)?;
                    let document = svg.CreateSvgDocument(
                        &stream,
                        D2D_SIZE_F {
                            width: ICON,
                            height: ICON,
                        },
                    )?;
                    Ok((icon, document))
                })
                .collect::<Result<Vec<_>>>()?;
            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let text = crate::render::text_format(&write, 13.0, DWRITE_FONT_WEIGHT_MEDIUM)?;
            let brush = surface
                .context
                .CreateSolidColorBrush(&color(0xf7f9ff, 1.0), None)?;
            Ok(Self {
                surface,
                svg,
                icons,
                write,
                text,
                brush,
                opacity,
                fades: ART
                    .iter()
                    .map(|(icon, _)| (*icon, 0.0))
                    .chain([(BarIcon::Active, 0.0)])
                    .collect(),
                last_frame: Instant::now(),
                icons_dark: crate::theme::dark(),
                tray_bitmaps: HashMap::new(),
            })
        }
    }

    pub fn paint(
        &mut self,
        pixels: (u32, u32),
        dpi: i32,
        contents: BarContents<'_>,
    ) -> Result<Vec<(BarIcon, Range<i32>)>> {
        if self.icons_dark != crate::theme::dark() {
            for ((_, document), (_, art)) in self.icons.iter_mut().zip(ART) {
                let art = themed_art(art);
                unsafe {
                    let stream = SHCreateMemStream(Some(&art)).ok_or_else(Error::empty)?;
                    *document = self.svg.CreateSvgDocument(
                        &stream,
                        D2D_SIZE_F {
                            width: ICON,
                            height: ICON,
                        },
                    )?;
                }
            }
            self.icons_dark = crate::theme::dark();
        }
        let now = Instant::now();
        self.fades.retain(|(icon, _)| match icon {
            BarIcon::Tray(index) => *index < contents.tray.len(),
            _ => true,
        });
        for index in 0..contents.tray.len() {
            if !self
                .fades
                .iter()
                .any(|(icon, _)| *icon == BarIcon::Tray(index))
            {
                self.fades.push((BarIcon::Tray(index), 0.0));
            }
        }
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.032);
        self.last_frame = now;
        for (icon, fade) in &mut self.fades {
            let target = if contents.hovered == Some(*icon) {
                1.0
            } else {
                0.0
            };
            let step = dt / if target > *fade { 0.09 } else { 0.16 };
            *fade += (target - *fade).clamp(-step, step);
        }
        let scale = dpi as f32 / 96.0;
        self.tray_bitmaps
            .retain(|serial, _| contents.tray.iter().any(|icon| icon.serial == *serial));
        for icon in contents.tray {
            if !self.tray_bitmaps.contains_key(&icon.serial) {
                let image = &icon.image;
                let bitmap = unsafe {
                    self.surface.context.CreateBitmap(
                        D2D_SIZE_U {
                            width: image.width,
                            height: image.height,
                        },
                        Some(image.pixels.as_ptr().cast()),
                        image.width * 4,
                        &D2D1_BITMAP_PROPERTIES1 {
                            pixelFormat: D2D1_PIXEL_FORMAT {
                                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                            },
                            dpiX: 96.0,
                            dpiY: 96.0,
                            ..Default::default()
                        },
                    )?
                };
                self.tray_bitmaps.insert(icon.serial, bitmap);
            }
        }
        let width = pixels.0 as f32 / scale;
        let height = pixels.1 as f32 / scale;
        let clock_width = (width - ART.len() as f32 * (ICON + PADDING) - LABEL_GAP).max(1.0);
        let clock = self.text_layout(contents.clock, clock_width, height)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { clock.GetMetrics(&mut metrics)? };
        let clock_width = if contents.clock.is_empty() {
            0.0
        } else {
            LABEL_GAP + metrics.width.ceil()
        };
        let battery_text = contents
            .battery
            .percent()
            .map_or_else(String::new, |p| format!("{p}%"));
        let battery_label = self.text_layout(&battery_text, 48.0, height)?;
        unsafe { battery_label.GetMetrics(&mut metrics)? };
        let battery_label_width = if battery_text.is_empty() {
            0.0
        } else {
            LABEL_GAP + metrics.width.ceil()
        };
        let battery_plugged = matches!(
            contents.battery,
            Battery::Present {
                plugged_in: Some(true),
                ..
            } | Battery::Present {
                charging: Some(true),
                ..
            }
        );
        let battery_icon_width = ICON + if battery_plugged { 10.0 } else { 0.0 };
        let mut right = width - PADDING;
        let mut slots = Vec::with_capacity(ART.len());
        let mut hitboxes = Vec::with_capacity(ART.len());
        let px = |x: f32| (x * scale).round() as i32;
        for (icon, document) in self
            .icons
            .iter()
            .rev()
            .filter(|(icon, _)| *icon != BarIcon::System)
        {
            let label_width = if *icon == BarIcon::Clock {
                clock_width
            } else if *icon == BarIcon::Battery {
                battery_label_width
            } else {
                0.0
            };
            let icon_width = if *icon == BarIcon::Battery {
                battery_icon_width
            } else {
                ICON
            };
            let x = right - icon_width - label_width;
            if x - PADDING / 2.0 < PADDING * 1.5 + ICON {
                break;
            }
            slots.push((*icon, document, x));
            hitboxes.push((*icon, px(x - PADDING / 2.0)..px(right + PADDING / 2.0)));
            right = x - PADDING;
        }
        if let Some((icon, document)) = self.icons.iter().find(|(icon, _)| *icon == BarIcon::System)
        {
            slots.push((*icon, document, PADDING));
            hitboxes.push((*icon, px(PADDING / 2.0)..px(PADDING * 1.5 + ICON)));
        }
        let mut tray_slots = Vec::new();
        // Keep native status controls on the right, app tray icons just to their left.
        for (index, icon) in contents.tray.iter().enumerate().rev() {
            let x = right - ICON;
            if x < PADDING * 2.0 + ICON + 60.0 {
                break;
            }
            tray_slots.push((index, icon.serial, x));
            hitboxes.push((BarIcon::Tray(index), px(x - 4.0)..px(right + 4.0)));
            right = x - 10.0;
        }
        let active_left = PADDING * 2.0 + ICON;
        let active = self.text_layout(contents.active, (right - active_left).max(1.0), height)?;
        self.surface.begin(pixels, dpi as f32)?;
        unsafe {
            self.brush.SetColor(&color(0x18212b, self.opacity));
            self.surface
                .context
                .FillRectangle(&rect(0.0, 0.0, width, height), &self.brush);
        }
        if right > active_left {
            let mut metrics = DWRITE_TEXT_METRICS::default();
            unsafe {
                active.GetMetrics(&mut metrics)?;
            }
            let label_width = metrics.width.ceil().min(right - active_left);
            hitboxes.push((
                BarIcon::Active,
                px(active_left - 5.0)..px(active_left + label_width + 5.0),
            ));
            self.tile(BarIcon::Active, active_left, label_width, height);
            self.draw_text(&active, active_left);
        }
        let y = (height - ICON) / 2.0;
        for (index, serial, x) in tray_slots {
            self.tile(BarIcon::Tray(index), x, ICON, height);
            if let Some(bitmap) = self.tray_bitmaps.get(&serial) {
                unsafe {
                    self.surface.context.DrawBitmap(
                        bitmap,
                        Some(&rect(x, y, ICON, ICON)),
                        1.0,
                        D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                        None,
                        None,
                    )
                };
            }
        }
        for (icon, document, x) in slots {
            unsafe {
                self.tile(
                    icon,
                    x,
                    (if icon == BarIcon::Battery {
                        battery_icon_width
                    } else {
                        ICON
                    }) + if icon == BarIcon::Clock {
                        clock_width
                    } else if icon == BarIcon::Battery {
                        battery_label_width
                    } else {
                        0.0
                    },
                    height,
                );
                if icon == BarIcon::Volume {
                    self.brush.SetColor(&color(0xf7f9ff, 1.0));
                    icons::draw_d2d(
                        &self.surface.context,
                        &self.brush,
                        Symbol::Volume(VolumeLevel::from_status(contents.audio)),
                        x,
                        y,
                        ICON,
                        1.5,
                    );
                } else if icon == BarIcon::Battery {
                    self.draw_battery(contents.battery, x, y);
                } else {
                    self.surface
                        .context
                        .SetTransform(&Matrix3x2::translation(x, y));
                    self.svg.DrawSvgDocument(document);
                    self.surface.context.SetTransform(&Matrix3x2::identity());
                }
                if icon == BarIcon::Wifi && contents.wifi_disconnected {
                    for (rgb, stroke) in [(0x141922, 3.5), (0xf7f9ff, 1.5)] {
                        self.brush.SetColor(&color(rgb, 1.0));
                        self.surface.context.DrawLine(
                            Vector2 {
                                X: x + 4.0,
                                Y: y + 3.0,
                            },
                            Vector2 {
                                X: x + 16.0,
                                Y: y + 17.0,
                            },
                            &self.brush,
                            stroke,
                            None,
                        );
                    }
                }
            }
            if icon == BarIcon::Clock && clock_width > 0.0 {
                self.draw_text(&clock, x + ICON + LABEL_GAP);
            }
            if icon == BarIcon::Battery && battery_label_width > 0.0 {
                self.draw_text(&battery_label, x + battery_icon_width + LABEL_GAP);
            }
        }
        self.surface.present()?;
        Ok(hitboxes)
    }

    fn draw_battery(&self, battery: Battery, x: f32, y: f32) {
        let charging = matches!(
            battery,
            Battery::Present {
                charging: Some(true),
                ..
            }
        );
        let plugged = matches!(
            battery,
            Battery::Present {
                plugged_in: Some(true),
                ..
            }
        );
        unsafe {
            if let Some(percent) = battery.percent() {
                let ink = if charging {
                    0x66dda0
                } else if percent <= 20 {
                    0xff7474
                } else if percent <= 40 {
                    0xf3c96a
                } else {
                    0xf7f9ff
                };
                self.brush.SetColor(&color(ink, 1.0));
                self.surface.context.FillRectangle(
                    &rect(x + 4.0, y + 7.5, 10.0 * f32::from(percent) / 100.0, 5.0),
                    &self.brush,
                );
            }
            self.brush.SetColor(&color(0xf7f9ff, 1.0));
            let symbol = if matches!(battery, Battery::AcPower) {
                Symbol::Plug
            } else {
                Symbol::Battery
            };
            icons::draw_d2d(&self.surface.context, &self.brush, symbol, x, y, ICON, 1.4);
            if charging || plugged {
                self.brush
                    .SetColor(&color(if charging { 0x66dda0 } else { 0xf7f9ff }, 1.0));
                icons::draw_d2d(
                    &self.surface.context,
                    &self.brush,
                    if charging { Symbol::Bolt } else { Symbol::Plug },
                    x + 20.0,
                    y + 4.0,
                    11.0,
                    1.25,
                );
            }
            if matches!(
                battery,
                Battery::Unavailable | Battery::Present { percent: None, .. }
            ) {
                self.brush.SetColor(&color(0xf7f9ff, 0.7));
                self.surface.context.DrawLine(
                    Vector2 {
                        X: x + 7.0,
                        Y: y + 10.0,
                    },
                    Vector2 {
                        X: x + 12.0,
                        Y: y + 10.0,
                    },
                    &self.brush,
                    1.4,
                    None,
                );
            }
        }
    }

    pub fn animating(&self, hovered: Option<BarIcon>) -> bool {
        self.fades
            .iter()
            .any(|(icon, value)| *value != if hovered == Some(*icon) { 1.0 } else { 0.0 })
    }

    fn tile(&self, icon: BarIcon, x: f32, width: f32, height: f32) {
        let opacity = self
            .fades
            .iter()
            .find(|(key, _)| *key == icon)
            .map_or(0.0, |(_, value)| *value);
        if opacity <= 0.0 {
            return;
        }
        unsafe {
            let tile = D2D1_ROUNDED_RECT {
                rect: rect(x - 5.0, 3.0, width + 10.0, (height - 6.0).max(1.0)),
                radiusX: 7.0,
                radiusY: 7.0,
            };
            self.brush.SetColor(&color(0x263447, 0.78 * opacity));
            self.surface
                .context
                .FillRoundedRectangle(&tile, &self.brush);
            self.brush.SetColor(&color(0xe5edff, 0.27 * opacity));
            self.surface
                .context
                .DrawRoundedRectangle(&tile, &self.brush, 1.0, None);
        }
    }

    fn text_layout(&self, label: &str, width: f32, height: f32) -> Result<IDWriteTextLayout> {
        let text: Vec<_> = label.encode_utf16().collect();
        unsafe {
            self.write
                .CreateTextLayout(&text, &self.text, width.max(1.0), height)
        }
    }

    fn draw_text(&self, layout: &IDWriteTextLayout, x: f32) {
        unsafe {
            self.brush.SetColor(&color(
                0x11151d,
                if crate::theme::dark() { 0.85 } else { 0.0 },
            ));
            for (dx, y) in [(0.0, -0.75), (-0.75, 0.0), (0.75, 0.0), (0.0, 1.0)] {
                self.surface.context.DrawTextLayout(
                    Vector2 { X: x + dx, Y: y },
                    layout,
                    &self.brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            self.brush.SetColor(&color(0xf7f9ff, 1.0));
            self.surface.context.DrawTextLayout(
                Vector2 { X: x, Y: 0.0 },
                layout,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
        }
    }
}

fn themed_art(art: &[u8]) -> Vec<u8> {
    if crate::theme::dark() {
        art.to_vec()
    } else {
        String::from_utf8_lossy(art)
            .replace("#f7f9ff", "#202b3c")
            .replace("#141922", "#f7f9ff")
            .into_bytes()
    }
}
