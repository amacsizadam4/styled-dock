//! Shared 24px outline artwork for native panels and live status indicators.
use std::f32::consts::{PI, TAU};
use windows::Win32::Foundation::{COLORREF, POINT};
use windows::Win32::Graphics::Direct2D::{ID2D1DeviceContext, ID2D1SolidColorBrush};
use windows::Win32::Graphics::Gdi::*;
use windows_numerics::Vector2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VolumeLevel {
    Unavailable,
    Muted,
    Silent,
    Low,
    Medium,
    High,
}

impl VolumeLevel {
    pub fn from_status(status: Option<(f32, bool)>) -> Self {
        match status {
            None => Self::Unavailable,
            Some((_, true)) => Self::Muted,
            Some((level, false)) if level <= 0.0 => Self::Silent,
            Some((level, false)) if level < 0.34 => Self::Low,
            Some((level, false)) if level < 0.67 => Self::Medium,
            Some(_) => Self::High,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Symbol {
    Search,
    Wifi,
    Bluetooth,
    Display,
    Moon,
    Battery,
    Settings,
    Appearance,
    Bolt,
    Plug,
    Volume(VolumeLevel),
}

type Path = Vec<(f32, f32)>;

fn arc(x: f32, y: f32, radius: f32, start: f32, end: f32) -> Path {
    (0..=24)
        .map(|i| {
            let angle = start + (end - start) * i as f32 / 24.0;
            (x + radius * angle.cos(), y + radius * angle.sin())
        })
        .collect()
}

fn paths(symbol: Symbol) -> Vec<Path> {
    match symbol {
        Symbol::Search => vec![
            arc(10.0, 10.0, 6.5, 0.0, TAU),
            vec![(15.0, 15.0), (21.0, 21.0)],
        ],
        Symbol::Wifi => vec![
            arc(12.0, 20.0, 17.0, -PI * 0.75, -PI * 0.25),
            arc(12.0, 20.0, 11.0, -PI * 0.75, -PI * 0.25),
            arc(12.0, 20.0, 5.0, -PI * 0.75, -PI * 0.25),
            arc(12.0, 20.0, 0.6, 0.0, TAU),
        ],
        Symbol::Bluetooth => vec![vec![
            (6.0, 7.0),
            (17.0, 17.0),
            (12.0, 22.0),
            (12.0, 2.0),
            (17.0, 7.0),
            (6.0, 17.0),
        ]],
        Symbol::Display => vec![
            vec![
                (3.0, 4.0),
                (21.0, 4.0),
                (21.0, 17.0),
                (3.0, 17.0),
                (3.0, 4.0),
            ],
            vec![(12.0, 17.0), (12.0, 21.0)],
            vec![(8.0, 21.0), (16.0, 21.0)],
        ],
        Symbol::Moon => {
            // Crescent bounded by two smooth curves.
            vec![vec![
                (20.7, 14.4),
                (20.1, 16.8),
                (18.4, 19.0),
                (16.0, 20.5),
                (13.0, 21.0),
                (10.0, 20.5),
                (7.3, 18.8),
                (5.5, 16.2),
                (5.0, 13.2),
                (5.6, 10.0),
                (7.4, 7.4),
                (10.0, 5.6),
                (12.3, 5.0),
                (11.0, 7.7),
                (11.0, 10.5),
                (12.4, 13.0),
                (14.8, 14.8),
                (17.7, 15.3),
                (20.7, 14.4),
            ]]
        }
        Symbol::Battery => vec![
            vec![
                (3.0, 7.0),
                (19.0, 7.0),
                (19.0, 17.0),
                (3.0, 17.0),
                (3.0, 7.0),
            ],
            vec![(22.0, 10.0), (22.0, 14.0)],
        ],
        Symbol::Bolt => vec![vec![
            (13.0, 2.0),
            (6.0, 13.0),
            (12.0, 13.0),
            (10.0, 22.0),
            (18.0, 10.0),
            (12.0, 10.0),
            (13.0, 2.0),
        ]],
        Symbol::Plug => vec![
            vec![(8.0, 3.0), (8.0, 8.0)],
            vec![(16.0, 3.0), (16.0, 8.0)],
            vec![
                (6.0, 8.0),
                (18.0, 8.0),
                (18.0, 13.0),
                (15.0, 16.0),
                (9.0, 16.0),
                (6.0, 13.0),
                (6.0, 8.0),
            ],
            vec![(12.0, 16.0), (12.0, 22.0)],
        ],
        Symbol::Appearance => vec![
            arc(12.0, 12.0, 9.0, 0.0, TAU),
            vec![(12.0, 3.0), (12.0, 21.0)],
            vec![
                (12.0, 6.0),
                (16.0, 8.0),
                (18.0, 12.0),
                (16.0, 16.0),
                (12.0, 18.0),
            ],
        ],
        Symbol::Settings => {
            let mut gear = Vec::new();
            for i in 0..=48 {
                let angle = i as f32 * TAU / 48.0;
                let radius = if i % 6 == 1 || i % 6 == 2 { 10.0 } else { 8.0 };
                gear.push((12.0 + radius * angle.cos(), 12.0 + radius * angle.sin()));
            }
            vec![gear, arc(12.0, 12.0, 3.3, 0.0, TAU)]
        }
        Symbol::Volume(level) => {
            let mut result = vec![vec![
                (2.0, 9.0),
                (6.0, 9.0),
                (10.0, 5.0),
                (10.0, 19.0),
                (6.0, 15.0),
                (2.0, 15.0),
                (2.0, 9.0),
            ]];
            let waves = match level {
                VolumeLevel::Low => 1,
                VolumeLevel::Medium => 2,
                VolumeLevel::High => 3,
                _ => 0,
            };
            for i in 0..waves {
                result.push(arc(10.0, 12.0, 4.0 + i as f32 * 3.5, -0.85, 0.85));
            }
            if matches!(level, VolumeLevel::Muted | VolumeLevel::Unavailable) {
                result.push(vec![(15.0, 9.0), (21.0, 15.0)]);
                result.push(vec![(15.0, 15.0), (21.0, 9.0)]);
            }
            result
        }
    }
}

pub fn draw_gdi(dc: HDC, symbol: Symbol, x: i32, y: i32, size: i32, ink: COLORREF) {
    // Supersample outlines so curves stay smooth in GDI owner-drawn controls.
    unsafe {
        let high = size * 3;
        let buffer = CreateCompatibleDC(Some(dc));
        let bitmap = CreateCompatibleBitmap(dc, high, high);
        if buffer.is_invalid() || bitmap.is_invalid() {
            if !buffer.is_invalid() {
                let _ = DeleteDC(buffer);
            }
            if !bitmap.is_invalid() {
                let _ = DeleteObject(bitmap.into());
            }
            stroke_gdi(dc, symbol, x, y, size, ink);
            return;
        }
        let previous = SelectObject(buffer, bitmap.into());
        let _ = StretchBlt(
            buffer,
            0,
            0,
            high,
            high,
            Some(dc),
            x,
            y,
            size,
            size,
            SRCCOPY,
        );
        stroke_gdi(buffer, symbol, 0, 0, high, ink);
        let mode = SetStretchBltMode(dc, HALFTONE);
        let _ = StretchBlt(
            dc,
            x,
            y,
            size,
            size,
            Some(buffer),
            0,
            0,
            high,
            high,
            SRCCOPY,
        );
        SetStretchBltMode(dc, STRETCH_BLT_MODE(mode));
        SelectObject(buffer, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(buffer);
    }
}

fn stroke_gdi(dc: HDC, symbol: Symbol, x: i32, y: i32, size: i32, ink: COLORREF) {
    unsafe {
        let pen = CreatePen(
            PS_SOLID,
            (size as f32 / 14.0).round().max(1.0) as i32,
            crate::popup::adapted(ink),
        );
        let previous = SelectObject(dc, pen.into());
        for path in paths(symbol) {
            let points: Vec<_> = path
                .iter()
                .map(|&(a, b)| POINT {
                    x: x + (a * size as f32 / 24.0).round() as i32,
                    y: y + (b * size as f32 / 24.0).round() as i32,
                })
                .collect();
            let _ = Polyline(dc, &points);
        }
        SelectObject(dc, previous);
        let _ = DeleteObject(pen.into());
    }
}

pub fn draw_d2d(
    context: &ID2D1DeviceContext,
    brush: &ID2D1SolidColorBrush,
    symbol: Symbol,
    x: f32,
    y: f32,
    size: f32,
    stroke: f32,
) {
    for path in paths(symbol) {
        for line in path.windows(2) {
            unsafe {
                context.DrawLine(
                    Vector2 {
                        X: x + line[0].0 * size / 24.0,
                        Y: y + line[0].1 * size / 24.0,
                    },
                    Vector2 {
                        X: x + line[1].0 * size / 24.0,
                        Y: y + line[1].1 * size / 24.0,
                    },
                    brush,
                    stroke,
                    None,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_art_distinguishes_mute_silence_and_live_levels() {
        assert_eq!(VolumeLevel::from_status(None), VolumeLevel::Unavailable);
        assert_eq!(
            VolumeLevel::from_status(Some((1.0, true))),
            VolumeLevel::Muted
        );
        assert_eq!(
            VolumeLevel::from_status(Some((0.0, false))),
            VolumeLevel::Silent
        );
        assert_eq!(
            VolumeLevel::from_status(Some((0.2, false))),
            VolumeLevel::Low
        );
        assert_eq!(
            VolumeLevel::from_status(Some((0.5, false))),
            VolumeLevel::Medium
        );
        assert_eq!(
            VolumeLevel::from_status(Some((0.9, false))),
            VolumeLevel::High
        );
    }
}
