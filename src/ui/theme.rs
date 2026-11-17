use serde::Deserialize;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::w;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Auto,
    Light,
    Dark,
}
impl ThemeMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}
static MODE: AtomicU8 = AtomicU8::new(0);
static DARK: AtomicBool = AtomicBool::new(true);
pub fn configure(mode: ThemeMode) {
    MODE.store(
        match mode {
            ThemeMode::Auto => 0,
            ThemeMode::Light => 1,
            ThemeMode::Dark => 2,
        },
        Ordering::Relaxed,
    );
    refresh();
}
pub fn refresh() {
    let mut light = 0u32;
    let mut size = 4u32;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut light).cast()),
            Some(&mut size),
        );
    }
    DARK.store(
        match MODE.load(Ordering::Relaxed) {
            1 => false,
            2 => true,
            _ => light == 0,
        },
        Ordering::Relaxed,
    );
}
pub fn dark() -> bool {
    DARK.load(Ordering::Relaxed)
}
pub fn ui(rgb: u32) -> u32 {
    if dark() {
        return rgb;
    }
    match rgb {
        0x171c2a | 0x18212b => 0xf0f3f8,
        0x192437 => 0xf8f9fc,
        0x41536f => 0xc3cbd7,
        0x202e44 => 0xeef2f7,
        0x354967 | 0x293952 | 0x263447 => 0xd9e3f2,
        0xf6f8ff | 0xe8eefb | 0xf7f9ff | 0xe3edf9 | 0xf4f7ff => 0x202b3c,
        0xb3c0d5 | 0x8291a8 => 0x58667a,
        0xa9c7ff => 0x2469d8,
        _ => rgb,
    }
}
pub fn settings(rgb: u32) -> u32 {
    if !dark() {
        return rgb;
    }
    match rgb {
        0xf7f8fb => 0x192437,
        0xeef1f6 => 0x141d2c,
        0x1c2739 => 0xe8eefb,
        0x526078 => 0xb3c0d5,
        0xdbe1eb => 0x41536f,
        0x235bc3 => 0xa9c7ff,
        0xdce8fd => 0x293952,
        _ => rgb,
    }
}
