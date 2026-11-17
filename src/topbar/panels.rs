use std::sync::{
    Mutex,
    atomic::{AtomicIsize, Ordering},
};
use std::time::{Duration, Instant};
use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};

use windows::Win32::Devices::Bluetooth::*;
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{E_FAIL, HANDLE, HWND, LPARAM, WIN32_ERROR, WPARAM};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::NetworkManagement::WiFi::*;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{Error, GUID, HSTRING, Result};

pub const WM_RADIOS: u32 = WM_APP + 15;
static OWNER: AtomicIsize = AtomicIsize::new(0);
static RADIOS: Mutex<[Option<bool>; 2]> = Mutex::new([None, None]);

pub fn radio_state(bluetooth: bool) -> Option<bool> {
    RADIOS.lock().ok().and_then(|s| s[usize::from(bluetooth)])
}
pub fn refresh_radios(hwnd: HWND) {
    OWNER.store(hwnd.0 as isize, Ordering::Relaxed);
    std::thread::spawn(move || {
        let _ = radios(None);
    });
}

// WinRT operations run off the window thread and have a deadline.
fn radios(toggle: Option<bool>) -> Result<()> {
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    macro_rules! bounded {
        ($operation:expr) => {{
            let operation = $operation?;
            let start = Instant::now();
            while operation.Status()?.0 == 0 {
                if start.elapsed() > Duration::from_secs(3) {
                    return Err(Error::new(
                        E_FAIL,
                        crate::i18n::text("Windows did not respond to the radio request."),
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            operation.GetResults()?
        }};
    }
    let devices = bounded!(Radio::GetRadiosAsync());
    let mut states = [None, None];
    let mut changed = false;
    for radio in devices {
        let index = match radio.Kind()? {
            RadioKind::WiFi => 0,
            RadioKind::Bluetooth => 1,
            _ => continue,
        };
        if toggle == Some(index == 1) {
            if bounded!(Radio::RequestAccessAsync()) != RadioAccessStatus::Allowed {
                return Err(Error::new(
                    E_FAIL,
                    crate::i18n::text(
                        "Windows does not allow this app to control the radio. Use Windows Settings.",
                    ),
                ));
            }
            let state = if radio.State()? == RadioState::On {
                RadioState::Off
            } else {
                RadioState::On
            };
            if bounded!(radio.SetStateAsync(state)) != RadioAccessStatus::Allowed {
                return Err(Error::new(
                    E_FAIL,
                    crate::i18n::text("Windows could not change the radio. Use Windows Settings."),
                ));
            }
            changed = true;
        }
        states[index] = match radio.State()? {
            RadioState::On => Some(true),
            RadioState::Off => Some(false),
            _ => None,
        };
    }
    if toggle.is_some() && !changed {
        return Err(Error::new(
            E_FAIL,
            crate::i18n::text("No controllable radio was found."),
        ));
    }
    if let Ok(mut cache) = RADIOS.lock() {
        *cache = states;
    }
    let hwnd = HWND(OWNER.load(Ordering::Relaxed) as *mut _);
    if !hwnd.0.is_null() {
        let _ = unsafe { PostMessageW(Some(hwnd), WM_RADIOS, WPARAM(0), LPARAM(0)) };
    }
    Ok(())
}
pub fn toggle_radio(bluetooth: bool) -> Result<()> {
    // The shell worker is STA; use a dedicated MTA for WinRT radio access.
    std::thread::spawn(move || radios(Some(bluetooth)))
        .join()
        .map_err(|_| Error::new(E_FAIL, crate::i18n::text("The radio request failed.")))?
}

fn wide(text: &[u16]) -> String {
    String::from_utf16_lossy(&text[..text.iter().position(|&c| c == 0).unwrap_or(text.len())])
}

struct Wlan(HANDLE);
impl Wlan {
    fn open() -> Result<Self> {
        let mut handle = HANDLE::default();
        let mut version = 0;
        unsafe {
            WIN32_ERROR(WlanOpenHandle(2, None, &mut version, &mut handle)).ok()?;
        }
        Ok(Self(handle))
    }
}
impl Drop for Wlan {
    fn drop(&mut self) {
        unsafe {
            WlanCloseHandle(self.0, None);
        }
    }
}
struct WlanMemory(*mut std::ffi::c_void);
impl Drop for WlanMemory {
    fn drop(&mut self) {
        unsafe {
            WlanFreeMemory(self.0);
        }
    }
}

pub struct Network {
    pub name: String,
    pub profile: String,
    pub connected: bool,
    pub secure: bool,
    pub signal: u32,
    pub interface: GUID,
}
pub fn networks() -> Result<Vec<Network>> {
    let wlan = Wlan::open()?;
    unsafe {
        let mut list = std::ptr::null_mut();
        WIN32_ERROR(WlanEnumInterfaces(wlan.0, None, &mut list)).ok()?;
        let _interfaces = WlanMemory(list.cast());
        if list.is_null() {
            return Ok(Vec::new());
        }
        let interfaces = std::slice::from_raw_parts(
            (&raw const (*list).InterfaceInfo).cast::<WLAN_INTERFACE_INFO>(),
            (*list).dwNumberOfItems as usize,
        );
        let mut networks = Vec::new();
        for interface in interfaces {
            let mut list = std::ptr::null_mut();
            WIN32_ERROR(WlanGetAvailableNetworkList(
                wlan.0,
                &interface.InterfaceGuid,
                0,
                None,
                &mut list,
            ))
            .ok()?;
            let _list = WlanMemory(list.cast());
            if list.is_null() {
                continue;
            }
            let rows = std::slice::from_raw_parts(
                (&raw const (*list).Network).cast::<WLAN_AVAILABLE_NETWORK>(),
                (*list).dwNumberOfItems as usize,
            );
            for row in rows {
                let length = (row.dot11Ssid.uSSIDLength as usize).min(row.dot11Ssid.ucSSID.len());
                let name = String::from_utf8_lossy(&row.dot11Ssid.ucSSID[..length]).into_owned();
                if name.is_empty() {
                    continue;
                }
                networks.push(Network {
                    name,
                    profile: wide(&row.strProfileName),
                    connected: row.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
                    secure: row.bSecurityEnabled.as_bool(),
                    signal: row.wlanSignalQuality,
                    interface: interface.InterfaceGuid,
                });
            }
        }
        networks.sort_by_key(|n| {
            (
                !n.connected,
                n.profile.is_empty(),
                std::cmp::Reverse(n.signal),
            )
        });
        networks.dedup_by(|a, b| a.name == b.name && a.interface == b.interface);
        Ok(networks)
    }
}
pub fn connect(network: &Network) -> Result<()> {
    let wlan = Wlan::open()?;
    let profile = HSTRING::from(&network.profile);
    let parameters = WLAN_CONNECTION_PARAMETERS {
        wlanConnectionMode: wlan_connection_mode_profile,
        strProfile: windows::core::PCWSTR(profile.as_ptr()),
        dot11BssType: dot11_BSS_type_any,
        ..Default::default()
    };
    unsafe { WIN32_ERROR(WlanConnect(wlan.0, &network.interface, &parameters, None)).ok() }
}

pub fn bluetooth_devices() -> Vec<(String, bool)> {
    unsafe {
        let search = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnAuthenticated: true.into(),
            fReturnRemembered: true.into(),
            fReturnConnected: true.into(),
            fIssueInquiry: false.into(),
            ..Default::default()
        };
        let mut info = BLUETOOTH_DEVICE_INFO {
            dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        let Ok(find) = BluetoothFindFirstDevice(&search, &mut info) else {
            return Vec::new();
        };
        let mut devices = Vec::new();
        loop {
            devices.push((wide(&info.szName), info.fConnected.as_bool()));
            if BluetoothFindNextDevice(find, &mut info).is_err() {
                break;
            }
        }
        let _ = BluetoothFindDeviceClose(find);
        devices.sort_by_key(|(_, connected)| !connected);
        devices
    }
}

pub struct Brightness {
    handle: HANDLE,
    min: u32,
    max: u32,
    pub level: f32,
}
impl Brightness {
    pub fn for_monitor(monitor: HMONITOR) -> Option<Self> {
        unsafe {
            let mut count = 0;
            GetNumberOfPhysicalMonitorsFromHMONITOR(monitor, &mut count).ok()?;
            if count == 0 || count > 32 {
                return None;
            }
            let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
            GetPhysicalMonitorsFromHMONITOR(monitor, &mut monitors).ok()?;
            let mut selected = None;
            for monitor in monitors {
                let (mut min, mut current, mut max) = (0, 0, 0);
                if selected.is_none()
                    && GetMonitorBrightness(
                        monitor.hPhysicalMonitor,
                        &mut min,
                        &mut current,
                        &mut max,
                    ) != 0
                    && max > min
                {
                    selected = Some(Self {
                        handle: monitor.hPhysicalMonitor,
                        min,
                        max,
                        level: (current - min) as f32 / (max - min) as f32,
                    });
                } else {
                    let _ = DestroyPhysicalMonitor(monitor.hPhysicalMonitor);
                }
            }
            selected
        }
    }
    pub fn set(&self, level: f32) -> Result<()> {
        let value =
            self.min + ((self.max - self.min) as f32 * level.clamp(0.0, 1.0)).round() as u32;
        if unsafe { SetMonitorBrightness(self.handle, value) } != 0 {
            Ok(())
        } else {
            Err(Error::from_thread())
        }
    }
}
impl Drop for Brightness {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyPhysicalMonitor(self.handle);
        }
    }
}
