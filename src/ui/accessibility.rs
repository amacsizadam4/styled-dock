//! Native button providers expose custom-drawn items to UI Automation/MSAA.
//! Buttons forward pointer gestures to their custom-drawn owner while native
//! providers handle accessibility, names, geometry, focus and Invoke.
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
};
use windows::Win32::{
    Foundation::*,
    UI::{
        Controls::WM_MOUSELEAVE,
        Input::KeyboardAndMouse::*,
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
use windows::core::{HSTRING, Result, w};

pub const WM_ACTION: u32 = WM_APP + 40;
pub const WM_FOCUS: u32 = WM_APP + 41;
const FIRST_ID: usize = 0x4000;
pub struct Entry {
    pub key: String,
    pub name: String,
    pub bounds: RECT,
}
struct Button {
    key: String,
    name: String,
    bounds: RECT,
    hwnd: HWND,
    id: usize,
}
impl Drop for Button {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}
struct Model {
    buttons: Vec<Button>,
    next_id: usize,
    actions: VecDeque<(String, bool)>,
    previous: HWND,
}
thread_local! { static MODELS: RefCell<HashMap<isize, Rc<RefCell<Model>>>> = RefCell::new(HashMap::new()); }
pub struct Surface {
    hwnd: HWND,
    model: Rc<RefCell<Model>>,
}
impl Surface {
    pub fn new(hwnd: HWND) -> Self {
        let model = Rc::new(RefCell::new(Model {
            buttons: Vec::new(),
            next_id: FIRST_ID,
            actions: VecDeque::new(),
            previous: HWND::default(),
        }));
        MODELS.with_borrow_mut(|models| {
            models.insert(hwnd.0 as isize, model.clone());
        });
        Self { hwnd, model }
    }
    pub fn update(&self, entries: Vec<Entry>) -> Result<()> {
        let mut model = self.model.borrow_mut();
        let old_order: Vec<_> = model
            .buttons
            .iter()
            .map(|button| button.key.clone())
            .collect();
        let mut previous = std::mem::take(&mut model.buttons);
        for entry in entries {
            let mut button =
                if let Some(index) = previous.iter().position(|button| button.key == entry.key) {
                    previous.remove(index)
                } else {
                    let start = model.next_id;
                    let id = loop {
                        let candidate = model.next_id;
                        model.next_id = if candidate == 0xffff {
                            FIRST_ID
                        } else {
                            candidate + 1
                        };
                        if !previous
                            .iter()
                            .chain(model.buttons.iter())
                            .any(|button| button.id == candidate)
                        {
                            break candidate;
                        }
                        if model.next_id == start {
                            return Err(windows::core::Error::from_hresult(E_OUTOFMEMORY));
                        }
                    };
                    let hwnd = unsafe {
                        CreateWindowExW(
                            WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
                            w!("BUTTON"),
                            &HSTRING::from(&entry.name),
                            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                            0,
                            0,
                            1,
                            1,
                            Some(self.hwnd),
                            Some(HMENU(id as *mut _)),
                            None,
                            None,
                        )?
                    };
                    unsafe {
                        if let Err(error) = SetWindowSubclass(hwnd, Some(button_proc), 1, 0).ok() {
                            let _ = DestroyWindow(hwnd);
                            return Err(error);
                        }
                    }
                    Button {
                        key: entry.key,
                        name: String::new(),
                        bounds: RECT::default(),
                        hwnd,
                        id,
                    }
                };
            if button.name != entry.name {
                unsafe {
                    SetWindowTextW(button.hwnd, &HSTRING::from(&entry.name))?;
                }
                button.name = entry.name;
            }
            if button.bounds != entry.bounds {
                let r = entry.bounds;
                unsafe {
                    SetWindowPos(
                        button.hwnd,
                        None,
                        r.left,
                        r.top,
                        (r.right - r.left).max(1),
                        (r.bottom - r.top).max(1),
                        SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOREDRAW,
                    )?;
                }
                button.bounds = r;
            }
            model.buttons.push(button);
        }
        let reordered = old_order
            != model
                .buttons
                .iter()
                .map(|button| button.key.clone())
                .collect::<Vec<_>>();
        let order: Vec<_> = model.buttons.iter().map(|button| button.hwnd).collect();
        drop(model);
        drop(previous);
        if reordered {
            for hwnd in order.into_iter().rev() {
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        Some(HWND_TOP),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOREDRAW,
                    )
                };
            }
        }
        Ok(())
    }
    pub fn take_action(&self) -> Option<(String, bool)> {
        self.model.borrow_mut().actions.pop_front()
    }
    pub fn focused(&self) -> Option<usize> {
        let focused = unsafe { GetFocus() };
        self.model
            .borrow()
            .buttons
            .iter()
            .position(|button| button.hwnd == focused)
    }
    pub fn focus_first(&self) {
        let mut model = self.model.borrow_mut();
        model.previous = unsafe { GetForegroundWindow() };
        let focused = model.buttons.first().map(|button| button.hwnd);
        drop(model);
        if let Some(focused) = focused {
            unsafe {
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(Some(focused));
            }
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        MODELS.with_borrow_mut(|models| {
            models.remove(&(self.hwnd.0 as isize));
        });
        self.model.borrow_mut().buttons.clear();
    }
}

fn model(hwnd: HWND) -> Option<Rc<RefCell<Model>>> {
    MODELS.with_borrow(|models| models.get(&(hwnd.0 as isize)).cloned())
}
pub fn command(owner: HWND, wparam: WPARAM, lparam: LPARAM) -> bool {
    if wparam.0 >> 16 != BN_CLICKED as usize {
        return false;
    }
    let Some(model) = model(owner) else {
        return false;
    };
    let Ok(mut state) = model.try_borrow_mut() else {
        let _ = unsafe { PostMessageW(Some(owner), WM_COMMAND, wparam, lparam) };
        return true;
    };
    if let Some(button) = state
        .buttons
        .iter()
        .find(|button| button.id == wparam.0 & 0xffff && button.hwnd.0 as isize == lparam.0)
    {
        let key = button.key.clone();
        state.actions.push_back((key, false));
        let _ = unsafe { PostMessageW(Some(owner), WM_ACTION, WPARAM(0), LPARAM(0)) };
        return true;
    }
    false
}
pub fn owner_draw(owner: HWND, wparam: WPARAM) -> bool {
    model(owner).is_some_and(|model| {
        model
            .try_borrow()
            .is_ok_and(|state| state.buttons.iter().any(|button| button.id == wparam.0))
    })
}
pub fn pointer_inside(owner: HWND) -> bool {
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point).is_err() } {
        return false;
    }
    let target = unsafe { WindowFromPoint(point) };
    if target == owner {
        let _ = unsafe {
            TrackMouseEvent(&mut TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: owner,
                dwHoverTime: 0,
            })
        };
        return true;
    }
    unsafe { IsChild(owner, target).as_bool() }
}
pub fn activated(owner: HWND, previous: HWND) {
    if previous != owner && !previous.is_invalid() && unsafe { IsWindow(Some(previous)).as_bool() }
    {
        if let Some(model) = model(owner) {
            if let Ok(mut model) = model.try_borrow_mut() {
                model.previous = previous;
            }
        }
    }
}
pub fn route(message: &MSG) -> bool {
    if message.message != WM_KEYDOWN {
        return false;
    }
    let Ok(owner) = (unsafe { GetParent(message.hwnd) }) else {
        return false;
    };
    let Some(model) = model(owner) else {
        return false;
    };
    let mut state = model.borrow_mut();
    let Some(index) = state
        .buttons
        .iter()
        .position(|button| button.hwnd == message.hwnd)
    else {
        return false;
    };
    let key = message.wParam.0 as u16;
    let last = state.buttons.len() - 1;
    let reverse = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    let next = match key {
        0x24 => Some(0),
        0x23 => Some(last),
        0x25 | 0x26 => Some(if index == 0 { last } else { index - 1 }),
        0x27 | 0x28 => Some(if index == last { 0 } else { index + 1 }),
        0x09 => Some(if reverse {
            if index == 0 { last } else { index - 1 }
        } else if index == last {
            0
        } else {
            index + 1
        }),
        0x0d | 0x20 | 0x5d | 0x79 => {
            let context = key == 0x5d || (key == 0x79 && reverse);
            if key == 0x79 && !context {
                return false;
            }
            let key = state.buttons[index].key.clone();
            state.actions.push_back((key, context));
            let _ = unsafe { PostMessageW(Some(owner), WM_ACTION, WPARAM(0), LPARAM(0)) };
            None
        }
        0x1b => {
            let previous = state.previous;
            drop(state);
            if !previous.is_invalid() && unsafe { IsWindow(Some(previous)).as_bool() } {
                let _ = unsafe { SetForegroundWindow(previous) };
            }
            return true;
        }
        _ => return false,
    };
    let focus = next.map(|index| state.buttons[index].hwnd);
    drop(state);
    if let Some(focus) = focus {
        let _ = unsafe { SetFocus(Some(focus)) };
    }
    true
}

unsafe extern "system" fn button_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        BM_CLICK => {
            if let Ok(parent) = unsafe { GetParent(hwnd) } {
                unsafe {
                    SendMessageW(
                        parent,
                        WM_COMMAND,
                        Some(WPARAM(GetDlgCtrlID(hwnd) as usize)),
                        Some(LPARAM(hwnd.0 as isize)),
                    );
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP
        | WM_LBUTTONDBLCLK => {
            if let Ok(parent) = unsafe { GetParent(hwnd) } {
                let mut point = POINT {
                    x: lparam.0 as i16 as i32,
                    y: (lparam.0 >> 16) as i16 as i32,
                };
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut point);
                    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(parent, &mut point);
                }
                let packed = (point.x as u16 as u32) | ((point.y as u16 as u32) << 16);
                let _ =
                    unsafe { PostMessageW(Some(parent), message, wparam, LPARAM(packed as isize)) };
                if message == WM_MOUSEMOVE {
                    let _ = unsafe {
                        TrackMouseEvent(&mut TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        })
                    };
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Ok(parent) = unsafe { GetParent(hwnd) } {
                let _ = unsafe { PostMessageW(Some(parent), WM_MOUSELEAVE, WPARAM(0), LPARAM(0)) };
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SETFOCUS | WM_KILLFOCUS => {
            if let Ok(parent) = unsafe { GetParent(hwnd) } {
                let _ = unsafe { PostMessageW(Some(parent), WM_FOCUS, WPARAM(0), LPARAM(0)) };
            }
            unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use windows::Win32::{
        System::{Com::*, LibraryLoader::GetModuleHandleW},
        UI::Accessibility::*,
    };
    use windows::core::Interface;

    unsafe extern "system" fn owner_proc(
        hwnd: HWND,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        if message == WM_COMMAND && command(hwnd, wp, lp) {
            return LRESULT(0);
        }
        if message == WM_DRAWITEM && owner_draw(hwnd, wp) {
            return LRESULT(1);
        }
        unsafe { DefWindowProcW(hwnd, message, wp, lp) }
    }

    #[test]
    fn native_uia_names_bounds_invoke_and_reordered_identity() -> Result<()> {
        let _ole = crate::files::Ole::new()?;
        let instance = unsafe { GetModuleHandleW(None)? }.into();
        let class = WNDCLASSW {
            hInstance: instance,
            lpfnWndProc: Some(owner_proc),
            lpszClassName: w!("StyledDockAccessibleTest"),
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&class);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                class.lpszClassName,
                w!("Accessible dock test"),
                WS_POPUP,
                100,
                100,
                240,
                100,
                None,
                None,
                Some(instance),
                None,
            )?
        };
        let surface = Surface::new(hwnd);
        let a = || Entry {
            key: "a".into(),
            name: "Editor · 2 windows".into(),
            bounds: RECT {
                left: 10,
                top: 10,
                right: 60,
                bottom: 60,
            },
        };
        let b = || Entry {
            key: "b".into(),
            name: "Browser".into(),
            bounds: RECT {
                left: 80,
                top: 10,
                right: 130,
                bottom: 60,
            },
        };
        surface.update(vec![a(), b()])?;
        let original = surface.model.borrow().buttons[0].hwnd;
        surface.update(vec![b(), a()])?;
        assert_eq!(surface.model.borrow().buttons[1].hwnd, original);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        let child = original.0 as usize;
        let parent = hwnd.0 as usize;
        let (tx, rx) = std::sync::mpsc::channel();
        let client = std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                unsafe {
                    CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
                    let automation: IUIAutomation =
                        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
                    let root = automation.ElementFromHandle(HWND(parent as *mut _))?;
                    let first = automation
                        .ControlViewWalker()?
                        .GetFirstChildElement(&root)?;
                    assert_eq!(first.CurrentName()?.to_string(), "Browser");
                    let element = automation.ElementFromHandle(HWND(child as *mut _))?;
                    assert_eq!(element.CurrentName()?.to_string(), "Editor · 2 windows");
                    assert_eq!(element.CurrentControlType()?, UIA_ButtonControlTypeId);
                    let bounds = element.CurrentBoundingRectangle()?;
                    assert!(bounds.right > bounds.left && bounds.bottom > bounds.top);
                    let at_point = automation.ElementFromPoint(POINT {
                        x: (bounds.left + bounds.right) / 2,
                        y: (bounds.top + bounds.bottom) / 2,
                    })?;
                    assert_eq!(at_point.CurrentName()?.to_string(), "Editor · 2 windows");
                    let invoke: IUIAutomationInvokePattern =
                        element.GetCurrentPattern(UIA_InvokePatternId)?.cast()?;
                    invoke.Invoke()?;
                    drop(invoke);
                    drop(element);
                    drop(first);
                    drop(root);
                    drop(automation);
                    CoUninitialize();
                    Ok(())
                }
            })();
            let _ = tx.send(result);
        });
        let start = Instant::now();
        let result = loop {
            if let Ok(result) = rx.try_recv() {
                break result;
            }
            if start.elapsed() > Duration::from_secs(5) {
                break Err(windows::core::Error::from_hresult(E_FAIL));
            }
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() } {
                unsafe {
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        // Native Invoke may post BM_CLICK after the client call returns.
        let mut action = surface.take_action();
        while action.is_none() && start.elapsed() < Duration::from_secs(5) {
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() } {
                unsafe {
                    DispatchMessageW(&message);
                }
            }
            action = surface.take_action();
            std::thread::sleep(Duration::from_millis(5));
        }
        if result.is_ok() {
            assert!(route(&MSG {
                hwnd: original,
                message: WM_KEYDOWN,
                wParam: WPARAM(0x25),
                ..Default::default()
            }));
            assert_eq!(surface.focused(), Some(0));
            let focused = surface.model.borrow().buttons[0].hwnd;
            assert!(route(&MSG {
                hwnd: focused,
                message: WM_KEYDOWN,
                wParam: WPARAM(0x5d),
                ..Default::default()
            }));
            assert_eq!(surface.take_action(), Some(("b".into(), true)));
        }
        drop(surface);
        let _ = unsafe { DestroyWindow(hwnd) };
        client.join().unwrap();
        result?;
        assert_eq!(action, Some(("a".into(), false)));
        Ok(())
    }
}
