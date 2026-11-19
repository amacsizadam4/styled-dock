use std::ffi::c_void;
use std::ptr::null_mut;

use windows::Win32::Foundation::{
    ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, HANDLE, HLOCAL, HWND, LPARAM, LocalFree, PROPERTYKEY,
    WPARAM,
};
use windows::Win32::Media::Audio::Endpoints::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::NetworkManagement::IpHelper::*;
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::WinSock::AF_UNSPEC;
use windows::Win32::System::Com::*;
use windows::Win32::System::Power::*;
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING,
};
use windows::Win32::UI::WindowsAndMessaging::{DEVICE_NOTIFY_WINDOW_HANDLE, PostMessageW, WM_APP};
use windows::core::{Error, GUID, PCWSTR, Result, implement};

pub const WM_NETWORK: u32 = WM_APP + 9;
pub const WM_AUDIO: u32 = WM_APP + 10;
pub const WM_AUDIO_DEVICE: u32 = WM_APP + 11;

fn notify(hwnd: HWND, message: u32) {
    let _ = unsafe { PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(0)) };
}

pub struct Power(Vec<HPOWERNOTIFY>);

#[derive(Clone, Copy, PartialEq)]
pub enum Battery {
    Present {
        percent: Option<u8>,
        plugged_in: Option<bool>,
        charging: Option<bool>,
    },
    AcPower,
    Unavailable,
}

impl Battery {
    pub fn label(self) -> String {
        match self {
            Self::Present { percent, .. } => percent.map_or_else(
                || crate::i18n::text("Battery").into(),
                |percent| crate::tr!("Battery {percent}%", percent = percent),
            ),
            Self::AcPower => crate::i18n::text("AC power").into(),
            Self::Unavailable => crate::i18n::text("Battery unavailable").into(),
        }
    }

    pub fn percent(self) -> Option<u8> {
        match self {
            Self::Present { percent, .. } => percent,
            _ => None,
        }
    }

    pub fn power_source(self) -> Option<&'static str> {
        match self {
            Self::Present {
                charging: Some(true),
                ..
            } => Some(crate::i18n::text("Plugged in · Charging")),
            Self::Present {
                plugged_in: Some(true),
                charging: Some(false),
                ..
            } => Some(crate::i18n::text("Plugged in · Not charging")),
            Self::Present {
                plugged_in: Some(true),
                ..
            } => Some(crate::i18n::text("Plugged in")),
            Self::Present {
                plugged_in: Some(false),
                ..
            } => Some(crate::i18n::text("On battery")),
            Self::Present { .. } => Some(crate::i18n::text("Power source unknown")),
            Self::AcPower => Some(crate::i18n::text("No battery detected")),
            Self::Unavailable => None,
        }
    }
}

impl Power {
    pub fn new(hwnd: HWND) -> Self {
        Self(
            [GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING]
                .iter()
                .filter_map(|guid| unsafe {
                    RegisterPowerSettingNotification(
                        HANDLE(hwnd.0),
                        guid,
                        DEVICE_NOTIFY_WINDOW_HANDLE,
                    )
                    .ok()
                })
                .collect(),
        )
    }

    pub fn status() -> Battery {
        let mut status = SYSTEM_POWER_STATUS::default();
        if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
            return Battery::Unavailable;
        }
        if status.BatteryFlag != 255 && status.BatteryFlag & 128 != 0 {
            return Battery::AcPower;
        }
        let percent = (status.BatteryLifePercent <= 100).then_some(status.BatteryLifePercent);
        if percent.is_some() || status.BatteryFlag != 255 {
            Battery::Present {
                percent,
                plugged_in: match status.ACLineStatus {
                    0 => Some(false),
                    1 => Some(true),
                    _ => None,
                },
                charging: (status.BatteryFlag != 255).then_some(status.BatteryFlag & 8 != 0),
            }
        } else {
            Battery::Unavailable
        }
    }
}

pub struct PowerPlan {
    pub name: String,
    id: GUID,
}

impl PowerPlan {
    pub fn list() -> Result<Vec<Self>> {
        let mut plans = Vec::new();
        for index in 0.. {
            let mut id = GUID::default();
            let mut size = size_of::<GUID>() as u32;
            let status = unsafe {
                PowerEnumerate(
                    None,
                    None,
                    None,
                    ACCESS_SCHEME,
                    index,
                    Some((&raw mut id).cast()),
                    &mut size,
                )
            };
            if status == ERROR_NO_MORE_ITEMS {
                break;
            }
            status.ok()?;
            let name = Self::read_name(&id).unwrap_or_else(|_| format!("{id:?}"));
            plans.push(Self { name, id });
        }
        Ok(plans)
    }

    pub fn active() -> Result<GUID> {
        let mut id = null_mut();
        unsafe {
            PowerGetActiveScheme(None, &mut id).ok()?;
            let active = id.as_ref().copied().ok_or_else(Error::empty)?;
            let _ = LocalFree(Some(HLOCAL(id.cast())));
            Ok(active)
        }
    }

    pub fn is_active(&self, active: Option<GUID>) -> bool {
        active == Some(self.id)
    }

    pub fn activate(&self) -> Result<()> {
        unsafe { PowerSetActiveScheme(None, Some(&self.id)).ok() }
    }

    fn read_name(id: &GUID) -> Result<String> {
        let mut size = 0;
        unsafe {
            let status = PowerReadFriendlyName(None, Some(id), None, None, None, &mut size);
            if status != ERROR_MORE_DATA {
                status.ok()?;
            }
            let mut name = vec![0u16; (size as usize).div_ceil(2)];
            PowerReadFriendlyName(
                None,
                Some(id),
                None,
                None,
                Some(name.as_mut_ptr().cast()),
                &mut size,
            )
            .ok()?;
            let length = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            Ok(String::from_utf16_lossy(&name[..length]))
        }
    }
}

impl Drop for Power {
    fn drop(&mut self) {
        for subscription in self.0.drain(..) {
            let _ = unsafe { UnregisterPowerSettingNotification(subscription) };
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Wifi,
    Ethernet,
    Cellular,
    Other,
}

impl Connection {
    pub fn name(self) -> &'static str {
        crate::i18n::text(match self {
            Self::Wifi => "Wi-Fi",
            Self::Ethernet => "Ethernet",
            Self::Cellular => "Cellular",
            Self::Other => "Network",
        })
    }

    pub fn symbol(self) -> crate::icons::Symbol {
        use crate::icons::Symbol;
        match self {
            Self::Wifi => Symbol::Wifi,
            Self::Ethernet => Symbol::Ethernet,
            Self::Cellular => Symbol::Cellular,
            Self::Other => Symbol::Network,
        }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct NetworkStatus {
    // None means no matching adapter, false means present with no active link.
    pub wifi: Option<bool>,
    pub ethernet: Option<bool>,
    pub cellular: Option<bool>,
    other: bool,
    available: bool,
}

impl NetworkStatus {
    pub fn primary(self) -> Option<Connection> {
        if self.ethernet == Some(true) {
            Some(Connection::Ethernet)
        } else if self.wifi == Some(true) {
            Some(Connection::Wifi)
        } else if self.cellular == Some(true) {
            Some(Connection::Cellular)
        } else {
            self.other.then_some(Connection::Other)
        }
    }

    pub fn label(self) -> &'static str {
        crate::i18n::text(match self.primary() {
            Some(Connection::Wifi) => "Wi-Fi connected",
            Some(Connection::Ethernet) => "Ethernet connected",
            Some(Connection::Cellular) => "Cellular connected",
            Some(Connection::Other) => "Network connected",
            None if self.available => "No network connection",
            None => "Network status unavailable",
        })
    }

    fn from_interfaces(rows: &[MIB_IF_ROW2], default_routes: &[u32]) -> Self {
        let mut status = Self {
            available: true,
            ..Self::default()
        };
        for row in rows {
            let up = row.OperStatus == IfOperStatusUp;
            // HardwareInterface is bit 0 of InterfaceAndOperStatusFlags.
            let hardware = row.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
            let link = match row.Type {
                IF_TYPE_IEEE80211 if hardware => &mut status.wifi,
                IF_TYPE_ETHERNET_CSMACD if hardware => &mut status.ethernet,
                IF_TYPE_WWANPP | IF_TYPE_WWANPP2 => &mut status.cellular,
                IF_TYPE_SOFTWARE_LOOPBACK => continue,
                _ => {
                    // An always-up virtual adapter alone does not indicate connectivity.
                    status.other |= up && default_routes.contains(&row.InterfaceIndex);
                    continue;
                }
            };
            *link = Some(link.unwrap_or(false) || up);
        }
        status
    }

    pub fn read() -> Self {
        let mut table = null_mut();
        if unsafe { GetIfTable2(&mut table) }.0 != 0 || table.is_null() {
            return Self::default();
        }
        unsafe {
            let rows = std::slice::from_raw_parts(
                (&raw const (*table).Table).cast::<MIB_IF_ROW2>(),
                (*table).NumEntries as usize,
            );
            let status = Self::from_interfaces(rows, &default_route_interfaces());
            FreeMibTable(table.cast());
            status
        }
    }
}

fn default_route_interfaces() -> Vec<u32> {
    let mut table = null_mut();
    if unsafe { GetIpForwardTable2(AF_UNSPEC, &mut table) }.0 != 0 || table.is_null() {
        return Vec::new();
    }
    unsafe {
        let routes = std::slice::from_raw_parts(
            (&raw const (*table).Table).cast::<MIB_IPFORWARD_ROW2>(),
            (*table).NumEntries as usize,
        );
        let interfaces = routes
            .iter()
            .filter(|route| route.DestinationPrefix.PrefixLength == 0 && route.ValidLifetime != 0)
            .map(|route| route.InterfaceIndex)
            .collect();
        FreeMibTable(table.cast());
        interfaces
    }
}

pub struct NetworkChanges(Vec<HANDLE>);

impl NetworkChanges {
    pub fn new(hwnd: HWND) -> Option<Self> {
        let mut interfaces = HANDLE::default();
        let mut routes = HANDLE::default();
        let subscriptions = unsafe {
            [
                NotifyIpInterfaceChange(
                    AF_UNSPEC,
                    Some(interface_changed),
                    Some(hwnd.0.cast_const()),
                    false,
                    &mut interfaces,
                )
                .is_ok()
                .then_some(interfaces),
                NotifyRouteChange2(
                    AF_UNSPEC,
                    Some(route_changed),
                    hwnd.0.cast_const(),
                    false,
                    &mut routes,
                )
                .is_ok()
                .then_some(routes),
            ]
        }
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        (!subscriptions.is_empty()).then_some(Self(subscriptions))
    }
}

unsafe extern "system" fn interface_changed(
    context: *const c_void,
    _: *const MIB_IPINTERFACE_ROW,
    _: MIB_NOTIFICATION_TYPE,
) {
    notify(HWND(context.cast_mut()), WM_NETWORK);
}

unsafe extern "system" fn route_changed(
    context: *const c_void,
    _: *const MIB_IPFORWARD_ROW2,
    _: MIB_NOTIFICATION_TYPE,
) {
    notify(HWND(context.cast_mut()), WM_NETWORK);
}

impl Drop for NetworkChanges {
    fn drop(&mut self) {
        for subscription in self.0.drain(..) {
            let _ = unsafe { CancelMibChangeNotify2(subscription) };
        }
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    fn interface(index: u32, kind: u32, hardware: bool, up: bool) -> MIB_IF_ROW2 {
        MIB_IF_ROW2 {
            InterfaceIndex: index,
            Type: kind,
            InterfaceAndOperStatusFlags: MIB_IF_ROW2_0 {
                _bitfield: u8::from(hardware),
            },
            OperStatus: if up {
                IfOperStatusUp
            } else {
                windows::Win32::NetworkManagement::Ndis::IfOperStatusDown
            },
            ..Default::default()
        }
    }

    #[test]
    fn wired_wifi_and_cellular_links_are_reported_independently() {
        let radios = [
            interface(1, IF_TYPE_IEEE80211, true, false),
            interface(2, IF_TYPE_IEEE80211, true, true),
            interface(3, IF_TYPE_WWANPP, true, true),
        ];
        let status = NetworkStatus::from_interfaces(&radios, &[]);
        assert_eq!(status.primary(), Some(Connection::Wifi));
        assert_eq!(status.wifi, Some(true));
        assert_eq!(status.cellular, Some(true));
        let status = NetworkStatus::from_interfaces(
            &[
                radios[0],
                radios[1],
                radios[2],
                interface(4, IF_TYPE_ETHERNET_CSMACD, true, true),
            ],
            &[],
        );
        assert_eq!(status.primary(), Some(Connection::Ethernet));
        assert_eq!(status.wifi, Some(true));
        assert_eq!(
            NetworkStatus::from_interfaces(&radios[2..], &[]).primary(),
            Some(Connection::Cellular)
        );
    }

    #[test]
    fn virtual_adapters_without_a_default_route_do_not_look_connected() {
        let rows = [
            interface(1, IF_TYPE_ETHERNET_CSMACD, true, false),
            interface(2, IF_TYPE_ETHERNET_CSMACD, false, true),
            interface(3, IF_TYPE_SOFTWARE_LOOPBACK, false, true),
        ];
        let status = NetworkStatus::from_interfaces(&rows, &[3]);
        assert_eq!(status.primary(), None);
        assert_eq!(status.ethernet, Some(false));
        assert_eq!(
            NetworkStatus::from_interfaces(&rows, &[2]).primary(),
            Some(Connection::Other)
        );
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeChanged(isize);

impl IAudioEndpointVolumeCallback_Impl for VolumeChanged_Impl {
    fn OnNotify(&self, _: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        notify(HWND(self.0 as *mut _), WM_AUDIO);
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DefaultDeviceChanged(isize);

impl IMMNotificationClient_Impl for DefaultDeviceChanged_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eMultimedia {
            notify(HWND(self.0 as *mut _), WM_AUDIO_DEVICE);
        }
        Ok(())
    }
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

pub struct Audio {
    endpoint: Option<IAudioEndpointVolume>,
    enumerator: IMMDeviceEnumerator,
    device_callback: IMMNotificationClient,
    volume_callback: IAudioEndpointVolumeCallback,
    _apartment: Apartment,
}

impl Audio {
    pub fn new(hwnd: HWND) -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()?;
            let apartment = Apartment;
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)?;
            let device_callback = DefaultDeviceChanged(hwnd.0 as isize).into();
            enumerator.RegisterEndpointNotificationCallback(&device_callback)?;
            let mut audio = Self {
                endpoint: None,
                enumerator,
                device_callback,
                volume_callback: VolumeChanged(hwnd.0 as isize).into(),
                _apartment: apartment,
            };
            audio.reconnect();
            Ok(audio)
        }
    }

    pub fn reconnect(&mut self) {
        self.disconnect();
        self.endpoint = (|| unsafe {
            let device = self
                .enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_INPROC_SERVER, None)?;
            endpoint.RegisterControlChangeNotify(&self.volume_callback)?;
            Ok::<_, windows::core::Error>(endpoint)
        })()
        .ok();
    }

    pub fn status(&self) -> Option<(f32, bool)> {
        let endpoint = self.endpoint.as_ref()?;
        unsafe {
            Some((
                endpoint.GetMasterVolumeLevelScalar().ok()?,
                endpoint.GetMute().ok()?.as_bool(),
            ))
        }
    }

    pub fn set_volume(&self, level: f32) -> Result<()> {
        if let Some(endpoint) = &self.endpoint {
            unsafe {
                endpoint.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null())?;
            }
        }
        Ok(())
    }

    pub fn toggle_mute(&self) -> Result<()> {
        if let Some(endpoint) = &self.endpoint {
            unsafe {
                endpoint.SetMute(!endpoint.GetMute()?.as_bool(), std::ptr::null())?;
            }
        }
        Ok(())
    }

    fn disconnect(&mut self) {
        if let Some(endpoint) = self.endpoint.take() {
            let _ = unsafe { endpoint.UnregisterControlChangeNotify(&self.volume_callback) };
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.disconnect();
        let _ = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.device_callback)
        };
    }
}
