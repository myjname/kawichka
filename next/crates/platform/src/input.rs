//! Raw input is reduced immediately to counts/motion/timestamps. Key identifiers
//! and text are neither copied into events nor stored/logged.
use kavychka_core::activity::Input;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
#[derive(Clone, Copy)]
pub enum Observation {
    Input(u64, Input),
    Suspension(u64, u64),
    Uncertain(u64),
    SessionChanged(u64),
    Suspending(u64),
    PowerChanged(u64),
}
struct Mailbox {
    events: Mutex<Vec<Observation>>,
    overflow: AtomicBool,
    session: AtomicU32,
    sleeping: AtomicBool,
    session_registered: AtomicBool,
    power_registered: AtomicBool,
}
pub struct Monitor {
    mailbox: Arc<Mailbox>,
    #[cfg(windows)]
    hwnd: isize,
    #[cfg(windows)]
    join: Option<std::thread::JoinHandle<()>>,
}
impl Monitor {
    pub fn start() -> Result<Self, String> {
        let mailbox = Arc::new(Mailbox {
            events: Mutex::new(vec![]),
            overflow: AtomicBool::new(false),
            session: AtomicU32::new(2),
            sleeping: AtomicBool::new(false),
            session_registered: AtomicBool::new(false),
            power_registered: AtomicBool::new(false),
        });
        #[cfg(windows)]
        {
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            let shared = mailbox.clone();
            let join = std::thread::spawn(move || windows::run(shared, sender));
            let hwnd = receiver.recv().map_err(|e| e.to_string())??;
            Ok(Self {
                mailbox,
                hwnd,
                join: Some(join),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = mailbox;
            Err("input adapter is Windows-only in this release".into())
        }
    }
    pub fn update_environment(&self, env: &mut kavychka_core::engine::Environment) {
        let session = self.mailbox.session.load(Ordering::Acquire);
        env.locked |= session & 1 != 0;
        env.session_unknown |=
            session & 2 != 0 || !self.mailbox.session_registered.load(Ordering::Acquire);
        env.suspended |= self.mailbox.sleeping.load(Ordering::Acquire);
    }
    pub fn registrations(&self) -> (bool, bool) {
        (
            self.mailbox.session_registered.load(Ordering::Acquire),
            self.mailbox.power_registered.load(Ordering::Acquire),
        )
    }
    /// QA targets only this monitor's hidden HWND; no global session/power action.
    #[cfg(windows)]
    pub fn post_session_for_test(&self, event: u32) -> bool {
        windows::post_session(self.hwnd, event)
    }
    #[cfg(windows)]
    pub fn post_power_for_test(&self, event: u32) -> bool {
        windows::post_power(self.hwnd, event)
    }
    pub fn drain(&self) -> Vec<Observation> {
        let mut events = std::mem::take(
            &mut *self
                .mailbox
                .events
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        if self.mailbox.overflow.swap(false, Ordering::Relaxed) {
            events.push(Observation::Uncertain(crate::time::monotonic_ms()));
        }
        events
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    self.hwnd as _,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                    0,
                    0,
                );
            }
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }
}
#[cfg(windows)]
mod windows {
    use super::*;
    use windows_sys::Win32::{
        Foundation::*,
        System::{
            LibraryLoader::GetModuleHandleW, Power::*, RemoteDesktop::*,
            SystemServices::GUID_ACDC_POWER_SOURCE,
        },
        UI::{Input::*, WindowsAndMessaging::*},
    };
    struct Context {
        mailbox: Arc<Mailbox>,
        suspended: Option<u64>,
        absolute: Option<(i32, i32)>,
        session_id: u32,
        power_handle: isize,
    }
    fn push(context: &Context, event: Observation) {
        let mut events = context
            .mailbox
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if events.len() < 8192 {
            events.push(event);
        } else {
            context.mailbox.overflow.store(true, Ordering::Relaxed);
        }
    }
    unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        unsafe {
            if message == WM_NCCREATE {
                let create = &*(lp as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Context;
            if !ptr.is_null() {
                let context = &mut *ptr;
                let now = crate::time::monotonic_ms();
                if message == WM_TIMER && wp == 0x4b415659 {
                    // TermService can still be starting at sign-in. Retry only
                    // unavailable registrations; do not replace real lock flags.
                    if !context.mailbox.session_registered.load(Ordering::Acquire)
                        && WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) != 0
                    {
                        context
                            .mailbox
                            .session_registered
                            .store(true, Ordering::Release);
                        context
                            .mailbox
                            .session
                            .store(current_session_state(), Ordering::Release);
                        push(context, Observation::SessionChanged(now));
                    }
                    if context.power_handle == 0 {
                        context.power_handle = RegisterPowerSettingNotification(
                            hwnd,
                            &GUID_ACDC_POWER_SOURCE,
                            DEVICE_NOTIFY_WINDOW_HANDLE,
                        );
                        context
                            .mailbox
                            .power_registered
                            .store(context.power_handle != 0, Ordering::Release);
                    }
                }
                if message == WM_INPUT
                    && context.mailbox.session.load(Ordering::Acquire) == 0
                    && !context.mailbox.sleeping.load(Ordering::Acquire)
                {
                    let mut raw = RAWINPUT::default();
                    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
                    let read = GetRawInputData(
                        lp as HRAWINPUT,
                        RID_INPUT,
                        (&mut raw as *mut RAWINPUT).cast(),
                        &mut size,
                        std::mem::size_of::<RAWINPUTHEADER>() as u32,
                    );
                    if read != u32::MAX && read >= std::mem::size_of::<RAWINPUTHEADER>() as u32 {
                        match raw.header.dwType {
                            RIM_TYPEKEYBOARD if raw.data.keyboard.Flags & 1 == 0 => {
                                push(context, Observation::Input(now, Input::Keyboard))
                            }
                            RIM_TYPEMOUSE => {
                                let mouse = raw.data.mouse;
                                let flags = mouse.Anonymous.Anonymous.usButtonFlags;
                                if flags & (0x0001 | 0x0004 | 0x0010 | 0x0040 | 0x0100) != 0 {
                                    push(context, Observation::Input(now, Input::Click));
                                }
                                if flags & (0x0400 | 0x0800) != 0 {
                                    push(context, Observation::Input(now, Input::Scroll));
                                }
                                if mouse.usFlags & 1 == 0 {
                                    let pixels = mouse
                                        .lLastX
                                        .unsigned_abs()
                                        .saturating_add(mouse.lLastY.unsigned_abs());
                                    if pixels > 0 {
                                        push(
                                            context,
                                            Observation::Input(now, Input::Move { pixels }),
                                        );
                                    }
                                } else {
                                    let point = (mouse.lLastX, mouse.lLastY);
                                    let previous = context.absolute.replace(point);
                                    let pixels = absolute_distance(
                                        previous,
                                        point,
                                        GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as u32,
                                        GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as u32,
                                    );
                                    if pixels > 0 {
                                        push(
                                            context,
                                            Observation::Input(now, Input::Move { pixels }),
                                        );
                                    }
                                }
                            }
                            _ => (),
                        }
                    }
                }
                if message == WM_WTSSESSION_CHANGE && lp as u32 == context.session_id {
                    match wp as u32 {
                        WTS_SESSION_LOCK => {context.mailbox.session.fetch_or(1,Ordering::AcqRel);}
                        WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT | WTS_SESSION_LOGOFF => {context.mailbox.session.fetch_or(2,Ordering::AcqRel);}
                        WTS_SESSION_UNLOCK | 0xF /* WTS_SESSION_DESKTOP_READY */ => {context.mailbox.session.store(current_session_state(),Ordering::Release);}
                        WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT | WTS_SESSION_LOGON => {context.mailbox.session.store(current_session_state(),Ordering::Release);}
                        _=>{}
                    }
                    context.absolute = None;
                    push(context, Observation::SessionChanged(now));
                }
                if message == WM_POWERBROADCAST {
                    match wp as u32 {
                        PBT_APMSUSPEND => {
                            context.suspended = Some(now);
                            context.mailbox.sleeping.store(true, Ordering::Release);
                            push(context, Observation::Suspending(now));
                        }
                        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                            context.mailbox.sleeping.store(false, Ordering::Release);
                            if let Some(start) = context.suspended.take() {
                                push(
                                    context,
                                    Observation::Suspension(now, now.saturating_sub(start)),
                                );
                            } else {
                                push(context, Observation::Uncertain(now));
                            }
                        }
                        PBT_APMPOWERSTATUSCHANGE | PBT_POWERSETTINGCHANGE => {
                            push(context, Observation::PowerChanged(now));
                        }
                        _ => {}
                    }
                }
            }
            if message == WM_CLOSE {
                if let Some(context) = ptr.as_mut() {
                    if context
                        .mailbox
                        .session_registered
                        .swap(false, Ordering::AcqRel)
                    {
                        WTSUnRegisterSessionNotification(hwnd);
                    }
                    if context.power_handle != 0 {
                        UnregisterPowerSettingNotification(context.power_handle);
                        context.power_handle = 0;
                    }
                    context
                        .mailbox
                        .power_registered
                        .store(false, Ordering::Release);
                }
                KillTimer(hwnd, 0x4b415659);
                DestroyWindow(hwnd);
                return 0;
            }
            if message == WM_DESTROY {
                PostQuitMessage(0);
                return 0;
            }
            DefWindowProcW(hwnd, message, wp, lp)
        }
    }
    fn current_session_state() -> u32 {
        unsafe {
            let mut buffer = std::ptr::null_mut();
            let mut bytes = 0;
            if WTSQuerySessionInformationW(
                std::ptr::null_mut(),
                WTS_CURRENT_SESSION,
                WTSConnectState,
                &mut buffer,
                &mut bytes,
            ) == 0
            {
                return 2;
            }
            let active = !buffer.is_null() && bytes >= 4 && *(buffer as *const i32) == WTSActive;
            WTSFreeMemory(buffer.cast());
            if active { 0 } else { 2 }
        }
    }
    pub fn post_session(hwnd: isize, event: u32) -> bool {
        unsafe {
            let mut session = 0;
            if ProcessIdToSessionId(std::process::id(), &mut session) == 0 {
                return false;
            }
            SendMessageW(
                hwnd as HWND,
                WM_WTSSESSION_CHANGE,
                event as usize,
                session as isize,
            );
            true
        }
    }
    pub fn post_power(hwnd: isize, event: u32) -> bool {
        unsafe {
            SendMessageW(hwnd as HWND, WM_POWERBROADCAST, event as usize, 0);
            true
        }
    }
    pub fn run(mailbox: Arc<Mailbox>, ready: std::sync::mpsc::SyncSender<Result<isize, String>>) {
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let name: Vec<u16> = "KavychkaNextRawInput"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: name.as_ptr(),
                ..Default::default()
            };
            RegisterClassW(&class);
            let mut context = Box::new(Context {
                mailbox,
                suspended: None,
                absolute: None,
                session_id: 0,
                power_handle: 0,
            });
            if ProcessIdToSessionId(std::process::id(), &mut context.session_id) == 0 {
                let _ = ready.send(Err(std::io::Error::last_os_error().to_string()));
                return;
            }
            context
                .mailbox
                .session
                .store(current_session_state(), Ordering::Release);
            // Hidden top-level window, so power broadcasts reach it as well.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                name.as_ptr(),
                name.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                (&mut *context as *mut Context).cast(),
            );
            if hwnd.is_null() {
                let _ = ready.send(Err(std::io::Error::last_os_error().to_string()));
                return;
            }
            let devices = [
                RAWINPUTDEVICE {
                    usUsagePage: 1,
                    usUsage: 6,
                    dwFlags: RIDEV_INPUTSINK,
                    hwndTarget: hwnd,
                },
                RAWINPUTDEVICE {
                    usUsagePage: 1,
                    usUsage: 2,
                    dwFlags: RIDEV_INPUTSINK,
                    hwndTarget: hwnd,
                },
            ];
            if RegisterRawInputDevices(
                devices.as_ptr(),
                2,
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            ) == 0
            {
                let _ = ready.send(Err(std::io::Error::last_os_error().to_string()));
                DestroyWindow(hwnd);
                return;
            }
            let session_registered =
                WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) != 0;
            context
                .mailbox
                .session_registered
                .store(session_registered, Ordering::Release);
            context.power_handle = RegisterPowerSettingNotification(
                hwnd,
                &GUID_ACDC_POWER_SOURCE,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            );
            context
                .mailbox
                .power_registered
                .store(context.power_handle != 0, Ordering::Release);
            SetTimer(hwnd, 0x4b415659, 1000, None);
            let _ = ready.send(Ok(hwnd as isize));
            let mut message = MSG::default();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if context
                .mailbox
                .session_registered
                .swap(false, Ordering::AcqRel)
            {
                WTSUnRegisterSessionNotification(hwnd);
            }
            if context.power_handle != 0 {
                UnregisterPowerSettingNotification(context.power_handle);
            }
            context
                .mailbox
                .power_registered
                .store(false, Ordering::Release);
            let remove = [
                RAWINPUTDEVICE {
                    usUsagePage: 1,
                    usUsage: 6,
                    dwFlags: RIDEV_REMOVE,
                    hwndTarget: std::ptr::null_mut(),
                },
                RAWINPUTDEVICE {
                    usUsagePage: 1,
                    usUsage: 2,
                    dwFlags: RIDEV_REMOVE,
                    hwndTarget: std::ptr::null_mut(),
                },
            ];
            RegisterRawInputDevices(
                remove.as_ptr(),
                2,
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            );
        }
    }
}
#[cfg(windows)]
fn absolute_distance(
    previous: Option<(i32, i32)>,
    point: (i32, i32),
    width: u32,
    height: u32,
) -> u32 {
    let Some(old) = previous else {
        return 0;
    };
    ((u64::from(old.0.abs_diff(point.0)) * u64::from(width)
        + u64::from(old.1.abs_diff(point.1)) * u64::from(height))
        / 65535)
        .min(u64::from(u32::MAX)) as u32
}
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn absolute_device_click_at_an_unchanged_position_does_not_cancel_rest() {
        assert_eq!(absolute_distance(None, (30000, 20000), 1920, 1080), 0);
        assert_eq!(
            absolute_distance(Some((30000, 20000)), (30000, 20000), 1920, 1080),
            0
        );
        assert!(absolute_distance(Some((0, 0)), (65535, 0), 1920, 1080) >= 1920);
    }
}
