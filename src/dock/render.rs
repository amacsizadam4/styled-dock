use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
};
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{Interface, Result, w};
use windows_numerics::Vector2;

use crate::layout::{Layout, Scene};
use crate::shell::Icon;

pub struct Renderer {
    surface: Surface,
    brush: ID2D1SolidColorBrush,
    icons: HashMap<String, ID2D1Bitmap1>,
    opacity: f32,
    write: IDWriteFactory,
    label: IDWriteTextFormat,
    detail: IDWriteTextFormat,
}

pub(crate) struct Surface {
    pub context: ID2D1DeviceContext,
    swap_chain: IDXGISwapChain1,
    size: (u32, u32),
    frame_ready: HANDLE,
    // Keep the composition tree attached to the HWND while the renderer is alive.
    _target: IDCompositionTarget,
    _composition: IDCompositionDevice,
}

impl Surface {
    pub fn new(hwnd: HWND, size: (u32, u32)) -> Result<Self> {
        // The device, swap chain, and D2D context are used only by the window thread.
        unsafe {
            let mut device = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?;
            let device = device.ok_or_else(windows::core::Error::empty)?;
            let dxgi: IDXGIDevice = device.cast()?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let swap_chain = factory.CreateSwapChainForComposition(
                &device,
                &DXGI_SWAP_CHAIN_DESC1 {
                    Width: size.0,
                    Height: size.1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    Scaling: DXGI_SCALING_STRETCH,
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                    AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                    Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
                    ..Default::default()
                },
                None,
            )?;

            let context = D2D1CreateDevice(&dxgi, None)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            bind_back_buffer(&context, &swap_chain)?;

            let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = composition.CreateTargetForHwnd(hwnd, true)?;
            let root = composition.CreateVisual()?;
            root.SetContent(&swap_chain)?;
            target.SetRoot(&root)?;
            composition.Commit()?;

            let paced: IDXGISwapChain2 = swap_chain.cast()?;
            paced.SetMaximumFrameLatency(1)?;
            let frame_ready = paced.GetFrameLatencyWaitableObject();
            if frame_ready.is_invalid() {
                return Err(windows::core::Error::from_thread());
            }
            Ok(Self {
                context,
                swap_chain,
                size,
                frame_ready,
                _target: target,
                _composition: composition,
            })
        }
    }

    pub fn begin(&mut self, size: (u32, u32), dpi: f32) -> Result<()> {
        unsafe {
            if size != self.size {
                // DXGI requires all back-buffer references to be released before resizing.
                self.context.SetTarget(None);
                self.swap_chain.ResizeBuffers(
                    0,
                    size.0,
                    size.1,
                    DXGI_FORMAT_UNKNOWN,
                    DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
                )?;
                bind_back_buffer(&self.context, &self.swap_chain)?;
                self.size = size;
            }
            self.context.SetDpi(dpi, dpi);
            self.context.BeginDraw();
            self.context.Clear(Some(&D2D1_COLOR_F::default()));
        }
        Ok(())
    }

    pub fn present(&self) -> Result<()> {
        unsafe {
            self.context.EndDraw(None, None)?;
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()
        }
    }
}

impl Renderer {
    pub fn new(hwnd: HWND, layout: Layout, opacity: f32) -> Result<Self> {
        let surface = Surface::new(hwnd, layout.pixels)?;
        let brush = unsafe {
            surface
                .context
                .CreateSolidColorBrush(&color(0xffffff, 1.0), None)?
        };
        let write = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let label = text_format(&write, 14.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        let detail = text_format(&write, 12.0, DWRITE_FONT_WEIGHT_NORMAL)?;
        Ok(Self {
            surface,
            brush,
            icons: HashMap::new(),
            opacity,
            write,
            label,
            detail,
        })
    }

    pub fn retain_icons(&mut self, targets: &std::collections::HashSet<String>) {
        self.icons.retain(|key, _| targets.contains(key));
    }

    pub fn set_icon(&mut self, target: String, icon: Icon) {
        let bitmap = unsafe {
            self.surface
                .context
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: icon.width,
                        height: icon.height,
                    },
                    Some(icon.pixels.as_ptr().cast()),
                    icon.width * 4,
                    &D2D1_BITMAP_PROPERTIES1 {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0,
                        dpiY: 96.0,
                        ..Default::default()
                    },
                )
                .ok()
        };
        if let Some(bitmap) = bitmap {
            self.icons.insert(target, bitmap);
        }
    }

    pub fn frame_ready(&self) -> HANDLE {
        self.surface.frame_ready
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity.clamp(0.0, 1.0);
    }

    pub fn paint(
        &mut self,
        layout: Layout,
        scene: &Scene,
        items: &[crate::window::DockItem],
        marker: Option<f32>,
        hover: Option<(usize, &str)>,
        dragged: Option<usize>,
        focused: Option<usize>,
    ) -> Result<()> {
        let size = layout.pixels;
        if size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        self.surface.begin(size, layout.scale * 96.0)?;
        unsafe {
            self.draw_dock(scene, items, layout, dragged);
            if let Some(bounds) = focused.and_then(|index| scene.icons.get(index)) {
                self.brush.SetColor(&color(0x90c2ff, 1.0));
                self.surface.context.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(
                            bounds.x - 3.0,
                            bounds.y - 3.0,
                            bounds.width + 6.0,
                            bounds.height + 6.0,
                        ),
                        radiusX: 8.0,
                        radiusY: 8.0,
                    },
                    &self.brush,
                    2.0,
                    None,
                );
            }
            if let Some(x) = marker {
                self.brush.SetColor(&color(0xbad7ff, 1.0));
                self.surface.context.FillRectangle(
                    &rect(
                        x - 1.0,
                        scene.panel.y + 10.0,
                        2.0,
                        scene.panel.height - 20.0,
                    ),
                    &self.brush,
                );
            }
        }
        let overlay_result = hover.map_or(Ok(()), |(index, detail)| {
            self.draw_overlay(scene.overlay(layout, index), &items[index].pin.name, detail)
        });
        let present_result = self.surface.present();
        overlay_result.and(present_result)
    }

    fn draw_dock(
        &self,
        scene: &Scene,
        items: &[crate::window::DockItem],
        layout: Layout,
        dragged: Option<usize>,
    ) {
        unsafe {
            self.brush.SetColor(&color(0x171c2a, self.opacity));
            // Preserve the transparent clear outside the antialiased panel.
            let bounds = scene.panel;
            let radius = layout.dock.radius().min(bounds.width.min(bounds.height) / 2.0);
            let panel = D2D1_ROUNDED_RECT {
                rect: rect(bounds.x, bounds.y, bounds.width, bounds.height),
                radiusX: radius,
                radiusY: radius,
            };
            self.surface
                .context
                .FillRoundedRectangle(&panel, &self.brush);
            self.brush.SetColor(&color(0xe3edf9, self.opacity * 0.18));
            self.surface
                .context
                .DrawRoundedRectangle(&panel, &self.brush, 1.0, None);
        }
        for index in (1..items.len()).filter(|_| dragged.is_none()) {
            let in_stack = |item: &crate::window::DockItem| {
                matches!(
                    item.pin.target.as_str(),
                    "shell:Downloads" | "shell:RecycleBinFolder"
                ) && !item.pinned
            };
            if layout.divider == Some(index)
                || (in_stack(&items[index]) && !in_stack(&items[index - 1]))
            {
                let left = scene.icons[index - 1];
                let right = scene.icons[index];
                let x = (((left.x + left.width + right.x) / 2.0) * layout.scale).floor()
                    / layout.scale
                    + 0.5 / layout.scale;
                unsafe {
                    self.brush.SetColor(&color(0xe3edf9, 0.52));
                    self.surface.context.DrawLine(
                        Vector2 {
                            X: x,
                            Y: scene.panel.y + 14.0,
                        },
                        Vector2 {
                            X: x,
                            Y: scene.panel.y + scene.panel.height - 14.0,
                        },
                        &self.brush,
                        1.0 / layout.scale,
                        None,
                    );
                }
            }
        }
        // Paint the held icon last so its neighbours slide underneath it.
        for index in (0..items.len())
            .filter(|index| Some(*index) != dragged)
            .chain(dragged)
        {
            let item = &items[index];
            let bounds = &scene.icons[index];
            let (left, top, size) = (bounds.x, bounds.y, bounds.width);
            if item.pin.target == "dock:Start" {
                unsafe {
                    self.brush.SetColor(&color(0x38a8ff, 1.0));
                    for (x, y) in [(0.0, 0.0), (0.52, 0.0), (0.0, 0.52), (0.52, 0.52)] {
                        self.surface.context.FillRectangle(
                            &rect(
                                left + size * (0.12 + x * 0.76),
                                top + size * (0.12 + y * 0.76),
                                size * 0.35,
                                size * 0.35,
                            ),
                            &self.brush,
                        );
                    }
                }
            } else if item.pin.target == "dock:Search" {
                unsafe {
                    self.brush.SetColor(&color(0xe3edf9, 1.0));
                }
                crate::icons::draw_d2d(
                    &self.surface.context,
                    &self.brush,
                    crate::icons::Symbol::Search,
                    left + size * 0.08,
                    top + size * 0.08,
                    size * 0.84,
                    size * 0.055,
                );
            } else if let Some(icon) = self.icons.get(&item.pin.target) {
                unsafe {
                    let pixels = icon.GetPixelSize();
                    let scale = size / pixels.width.max(pixels.height) as f32;
                    let width = pixels.width as f32 * scale;
                    let height = pixels.height as f32 * scale;
                    let destination = rect(
                        left + (size - width) / 2.0,
                        top + (size - height) / 2.0,
                        width,
                        height,
                    );
                    self.surface.context.DrawBitmap(
                        icon,
                        Some(&destination),
                        1.0,
                        D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                        None,
                        None,
                    );
                }
            } else {
                self.draw_fallback(left, top, size);
            }
            if !item.windows.is_empty() {
                let foreground =
                    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
                let active = item.windows.iter().any(|&window| {
                    window == foreground
                        || window
                            == unsafe {
                                windows::Win32::UI::WindowsAndMessaging::GetAncestor(
                                    foreground,
                                    windows::Win32::UI::WindowsAndMessaging::GA_ROOTOWNER,
                                )
                            }
                });
                let attention = !active
                    && item
                        .windows
                        .iter()
                        .any(|&window| crate::watcher::attention(window));
                let dot = D2D1_ELLIPSE {
                    point: windows_numerics::Vector2 {
                        X: left + size / 2.0,
                        Y: scene.panel.y + scene.panel.height - 7.0,
                    },
                    radiusX: 2.0,
                    radiusY: 2.0,
                };
                unsafe {
                    self.brush
                        .SetColor(&color(if attention { 0xffb74d } else { 0xe3edf9 }, 1.0));
                    self.surface.context.FillEllipse(&dot, &self.brush);
                }
            }
        }
    }

    fn draw_overlay(&self, bounds: crate::layout::Bounds, name: &str, detail: &str) -> Result<()> {
        self.card(
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            12.0,
            0x192437,
            0.98,
        );
        let width = (bounds.width - 24.0).max(1.0);
        self.text(
            name,
            &self.label,
            bounds.x + 12.0,
            bounds.y + 4.0,
            width,
            22.0,
            0xf6f8ff,
            false,
        )?;
        self.text(
            detail,
            &self.detail,
            bounds.x + 12.0,
            bounds.y + 25.0,
            width,
            19.0,
            0xb3c0d5,
            false,
        )
    }

    fn card(&self, x: f32, y: f32, width: f32, height: f32, radius: f32, rgb: u32, opacity: f32) {
        let card = D2D1_ROUNDED_RECT {
            rect: rect(x + 0.5, y + 0.5, width - 1.0, height - 1.0),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe {
            self.brush.SetColor(&color(rgb, opacity));
            self.surface
                .context
                .FillRoundedRectangle(&card, &self.brush);
            self.brush.SetColor(&color(0xa9c7ff, 0.32));
            self.surface
                .context
                .DrawRoundedRectangle(&card, &self.brush, 1.0, None);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        value: &str,
        format: &IDWriteTextFormat,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        rgb: u32,
        centered: bool,
    ) -> Result<()> {
        let value: Vec<_> = value.encode_utf16().collect();
        unsafe {
            let layout = self.write.CreateTextLayout(&value, format, width, height)?;
            if centered {
                layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            }
            self.brush.SetColor(&color(rgb, 1.0));
            self.surface.context.DrawTextLayout(
                Vector2 { X: x, Y: y },
                &layout,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
        }
        Ok(())
    }

    fn draw_fallback(&self, left: f32, top: f32, size: f32) {
        let tile = D2D1_ROUNDED_RECT {
            rect: rect(left, top, size, size),
            radiusX: size * 0.23,
            radiusY: size * 0.23,
        };
        unsafe {
            self.brush.SetColor(&color(0x414b60, 1.0));
            self.surface
                .context
                .FillRoundedRectangle(&tile, &self.brush);
            self.brush.SetColor(&color(0xcbd5e1, 1.0));
            for (x, y) in [(0.29, 0.29), (0.53, 0.29), (0.29, 0.53), (0.53, 0.53)] {
                self.surface.context.FillRectangle(
                    &rect(left + x * size, top + y * size, 0.17 * size, 0.17 * size),
                    &self.brush,
                );
            }
        }
    }
}

pub(crate) fn text_format(
    write: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat> {
    unsafe {
        let text = write.CreateTextFormat(
            crate::popup::UI_FONT,
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!(""),
        )?;
        text.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        text.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        let ellipsis = write.CreateEllipsisTrimmingSign(&text)?;
        text.SetTrimming(
            &DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                ..Default::default()
            },
            &ellipsis,
        )?;
        Ok(text)
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.frame_ready) };
    }
}
fn bind_back_buffer(context: &ID2D1DeviceContext, swap_chain: &IDXGISwapChain1) -> Result<()> {
    unsafe {
        let buffer: IDXGISurface = swap_chain.GetBuffer(0)?;
        let bitmap = context.CreateBitmapFromDxgiSurface(
            &buffer,
            Some(&D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            }),
        )?;
        context.SetTarget(&bitmap);
        Ok(())
    }
}

pub(crate) fn rect(left: f32, top: f32, width: f32, height: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}
pub(crate) fn color(rgb: u32, alpha: f32) -> D2D1_COLOR_F {
    let rgb = crate::theme::ui(rgb);
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 255) as f32 / 255.0,
        g: ((rgb >> 8) & 255) as f32 / 255.0,
        b: (rgb & 255) as f32 / 255.0,
        a: alpha,
    }
}
use std::collections::HashMap;
