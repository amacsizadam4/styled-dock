//! A single shared snapshot for both panels. WinRT work stays off the UI thread.
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicIsize, Ordering},
};
use std::time::{Duration, Instant};
use windows::Networking::{Connectivity::NetworkInformation, NetworkOperators::*};
use windows::Win32::{
    Foundation::{E_FAIL, HWND, LPARAM, WPARAM},
    System::WinRT::*,
    UI::WindowsAndMessaging::*,
};
use windows::core::{Error, Result};

pub const WM_HOTSPOT: u32 = WM_APP + 16;
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub on: Option<bool>,
    pub busy: bool,
    pub detail: String,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            on: None,
            busy: false,
            detail: "Checking mobile hotspot…".into(),
        }
    }
}
static STATUS: Mutex<Option<Status>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);
static OWNER: AtomicIsize = AtomicIsize::new(0);

pub fn status() -> Status {
    STATUS
        .lock()
        .ok()
        .and_then(|state| state.clone())
        .unwrap_or_default()
}
pub fn detach(hwnd: HWND) {
    let _ = OWNER.compare_exchange(hwnd.0 as isize, 0, Ordering::Relaxed, Ordering::Relaxed);
}
fn publish(state: Status) {
    if let Ok(mut cache) = STATUS.lock() {
        if cache.as_ref() == Some(&state) {
            return;
        }
        *cache = Some(state);
    }
    let hwnd = HWND(OWNER.load(Ordering::Relaxed) as *mut _);
    if !hwnd.is_invalid() {
        let _ = unsafe { PostMessageW(Some(hwnd), WM_HOTSPOT, WPARAM(0), LPARAM(0)) };
    }
}
pub fn refresh(hwnd: HWND) {
    OWNER.store(hwnd.0 as isize, Ordering::Relaxed);
    if BUSY
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
    {
        std::thread::spawn(|| {
            let _ = run(false);
            BUSY.store(false, Ordering::Release);
        });
    }
}
pub fn toggle() -> Result<()> {
    if BUSY
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return Ok(());
    }
    let mut pending = status();
    pending.busy = true;
    pending.detail = "Changing mobile hotspot…".into();
    publish(pending);
    let result = std::thread::spawn(|| run(true)).join().unwrap_or_else(|_| {
        Err(Error::new(
            E_FAIL,
            crate::i18n::text("Mobile hotspot request failed."),
        ))
    });
    BUSY.store(false, Ordering::Release);
    result
}
fn run(toggle: bool) -> Result<()> {
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }
    let operation = (|| {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        let _apartment = Apartment;
        let profile = NetworkInformation::GetInternetConnectionProfile().map_err(|_| {
            Error::new(
                E_FAIL,
                crate::i18n::text("Connect to the internet to use mobile hotspot."),
            )
        })?;
        let manager = NetworkOperatorTetheringManager::CreateFromConnectionProfile(&profile)
            .map_err(|error| {
                Error::new(
                    error.code(),
                    crate::i18n::text(
                        "This app cannot control mobile hotspot. Use Windows settings.",
                    ),
                )
            })?;
        let state = manager.TetheringOperationalState()?;
        let capability =
            NetworkOperatorTetheringManager::GetTetheringCapabilityFromConnectionProfile(&profile)?;
        let on = match state {
            TetheringOperationalState::On => Some(true),
            TetheringOperationalState::Off => Some(false),
            _ => None,
        };
        if capability != TetheringCapability::Enabled && on != Some(true) {
            return Err(Error::new(
                E_FAIL,
                crate::i18n::text(match capability {
                    TetheringCapability::DisabledByGroupPolicy => {
                        "Mobile hotspot is disabled by your organization."
                    }
                    TetheringCapability::DisabledByHardwareLimitation => {
                        "This hardware cannot provide a mobile hotspot."
                    }
                    TetheringCapability::DisabledByOperator => {
                        "Your network operator does not allow mobile hotspot."
                    }
                    TetheringCapability::DisabledBySystemCapability => {
                        "Windows does not allow hotspot control for this app."
                    }
                    _ => {
                        "Mobile hotspot is unavailable for this connection. Open Windows settings for details."
                    }
                }),
            ));
        }
        if toggle {
            let Some(on) = on else {
                return Err(Error::new(
                    E_FAIL,
                    crate::i18n::text("Mobile hotspot is changing. Try again shortly."),
                ));
            };
            publish(Status {
                on: Some(on),
                busy: true,
                detail: "Changing mobile hotspot…".into(),
            });
            let request = if on {
                manager.StopTetheringAsync()?
            } else {
                manager.StartTetheringAsync()?
            };
            let start = Instant::now();
            while request.Status()?.0 == 0 {
                if start.elapsed() > Duration::from_secs(10) {
                    let _ = request.Cancel();
                    return Err(Error::new(
                        E_FAIL,
                        crate::i18n::text(
                            "Windows did not finish the hotspot request. Check Windows settings.",
                        ),
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let result = request.GetResults()?;
            if result.Status()? != TetheringOperationStatus::Success {
                return Err(Error::new(
                    E_FAIL,
                    crate::i18n::text(
                        "Windows could not change mobile hotspot. Open Windows settings for details.",
                    ),
                ));
            }
        }
        let state = manager.TetheringOperationalState()?;
        let on = match state {
            TetheringOperationalState::On => Some(true),
            TetheringOperationalState::Off => Some(false),
            _ => None,
        };
        publish(Status {
            on,
            busy: on.is_none(),
            detail: if on == Some(true) {
                "On"
            } else if on == Some(false) {
                "Off"
            } else {
                "Mobile hotspot is changing. Try again shortly."
            }
            .into(),
        });
        Ok(())
    })();
    if let Err(error) = &operation {
        publish(Status {
            on: None,
            busy: false,
            detail: if toggle {
                error.message()
            } else {
                error.message()
            },
        });
    }
    operation
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_initial_state_disables_toggling_without_touching_hardware() {
        let state = Status::default();
        assert!(!state.busy);
        assert!(state.on.is_none());
    }
}
