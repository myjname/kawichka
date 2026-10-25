//! Windows behavior behind a narrow handle boundary. No Slint types here.
use kavychka_core::engine::Environment;
#[derive(Clone, Copy, Debug)]
pub enum Overlay {
    Pet,
    PetAlpha,
    Message,
}
#[derive(Debug, Clone, Copy)]
pub struct WindowFacts {
    pub no_activate: bool,
    pub tool_window: bool,
    pub topmost: bool,
    pub clipped_corner: bool,
    pub native_frame: bool,
    pub client_fills_window: bool,
}
#[cfg(windows)]
mod windows {
    use super::*;
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{Power::*, StationsAndDesktops::*},
        UI::{Shell::*, WindowsAndMessaging::*},
    };
    // Winit deliberately retains WS_CAPTION on undecorated top-level windows for
    // Aero behavior. These nonactivating overlays must have no native chrome.
    fn overlay_style(style: u32) -> u32 {
        (style & !(WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX))
            | WS_POPUP
    }
    fn overlay_ex_style(style: u32) -> u32 {
        (style | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW)
            & !(WS_EX_APPWINDOW | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME)
    }
    unsafe extern "system" fn subclass(
        h: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        id: usize,
        _data: usize,
    ) -> LRESULT {
        unsafe {
            if msg == WM_MOUSEACTIVATE {
                return MA_NOACTIVATE as isize;
            }
            // The backend forwards NC activation to DefWindowProc, which can
            // paint a cached caption during clicks even without WS_CAPTION.
            // Keep its focus bookkeeping, but explicitly forbid native repaint.
            if msg == WM_NCACTIVATE {
                DefSubclassProc(h, msg, wp, -1);
                return 1;
            }
            if msg == WM_NCPAINT || msg == WM_NCCALCSIZE {
                return 0;
            }
            if msg == WM_NCHITTEST {
                return HTCLIENT as isize;
            }
            if msg == WM_NCDESTROY {
                RemoveWindowSubclass(h, Some(subclass), id);
            }
            let result = DefSubclassProc(h, msg, wp, lp);
            if msg == WM_STYLECHANGING && lp != 0 {
                let style = &mut *(lp as *mut STYLESTRUCT);
                if wp as isize == GWL_EXSTYLE as isize {
                    style.styleNew = overlay_ex_style(style.styleNew);
                } else if wp as isize == GWL_STYLE as isize {
                    style.styleNew = overlay_style(style.styleNew);
                }
            }
            result
        }
    }
    pub fn overlay(handle: isize, kind: Overlay, scale: f32) -> Result<(), String> {
        if !scale.is_finite() || !(0.5..=8.).contains(&scale) {
            return Err("Invalid overlay DPI scale".into());
        }
        let scale = f64::from(scale);
        unsafe {
            let h = handle as HWND;
            if SetWindowSubclass(h, Some(subclass), 0x4b415659, 0) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let style = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(h, GWL_EXSTYLE, overlay_ex_style(style) as isize);
            let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
            SetWindowLongPtrW(h, GWL_STYLE, overlay_style(style) as isize);
            if SetWindowPos(
                h,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            if matches!(kind, Overlay::PetAlpha) {
                return Ok(());
            }
            let mut rect = RECT::default();
            if GetClientRect(h, &mut rect) == 0 {
                return Err("cannot read overlay dimensions".into());
            }
            // Slint margins/radii are logical pixels. The user-selected pet
            // width is independent of DPI and must not rescale its corners.
            let margin = match kind {
                Overlay::Pet | Overlay::PetAlpha => (8. * scale) as i32,
                Overlay::Message => 0,
            };
            let radius = match kind {
                Overlay::Pet | Overlay::PetAlpha => (52. * scale) as i32,
                Overlay::Message => (28. * scale) as i32,
            };
            let region = CreateRoundRectRgn(
                margin,
                margin,
                rect.right - margin + 1,
                rect.bottom - margin + 1,
                radius,
                radius,
            );
            if region.is_null() {
                return Err("cannot build overlay region".into());
            }
            if SetWindowRgn(h, region, 1) == 0 {
                DeleteObject(region);
                return Err("cannot apply overlay region".into());
            }
            Ok(()) // successful SetWindowRgn transfers ownership to Windows
        }
    }
    /// Current-frame alpha spans, already transformed to client pixels. Windows
    /// owns the region only after a successful SetWindowRgn.
    pub fn alpha_region(handle: isize, spans: &[[i32; 4]]) -> Result<(), String> {
        if spans.len() > 4_000_000
            || spans.iter().any(|r| {
                r[0] < 0 || r[1] < 0 || r[2] > 4096 || r[3] > 4096 || r[2] < r[0] || r[3] < r[1]
            })
        {
            return Err("Invalid alpha region".into());
        }
        unsafe {
            // Aligned RGNDATAHEADER + row-major, disjoint RECT records. One
            // GDI allocation avoids a CombineRgn syscall for every image run.
            let mut data = Vec::<u32>::with_capacity(8 + spans.len() * 4);
            let bounds = spans.iter().fold([4096, 4096, 0, 0], |a, r| {
                [
                    a[0].min(r[0]),
                    a[1].min(r[1]),
                    a[2].max(r[2]),
                    a[3].max(r[3]),
                ]
            });
            data.extend_from_slice(&[
                32,
                RDH_RECTANGLES,
                spans.len() as u32,
                (spans.len() * 16) as u32,
            ]);
            data.extend(if spans.is_empty() { [0; 4] } else { bounds }.map(|v| v as u32));
            for r in spans {
                data.extend(r.map(|v| v as u32));
            }
            let region = if spans.is_empty() {
                CreateRectRgn(0, 0, 0, 0)
            } else {
                ExtCreateRegion(
                    std::ptr::null(),
                    (data.len() * 4) as u32,
                    data.as_ptr().cast(),
                )
            };
            if region.is_null() {
                return Err("Cannot build alpha region".into());
            }
            if SetWindowRgn(handle as HWND, region, 1) == 0 {
                DeleteObject(region);
                return Err("Cannot apply alpha region".into());
            }
            Ok(())
        }
    }
    /// Read back the actual native region of an app-owned test window.
    pub fn region_contains(handle: isize, x: i32, y: i32) -> Option<bool> {
        unsafe {
            let region = CreateRectRgn(0, 0, 0, 0);
            if region.is_null() {
                return None;
            }
            let result =
                (GetWindowRgn(handle as HWND, region) != 0).then(|| PtInRegion(region, x, y) != 0);
            DeleteObject(region);
            result
        }
    }
    fn area(handle: isize) -> RECT {
        unsafe {
            let monitor = MonitorFromWindow(handle as HWND, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info) != 0 {
                info.rcWork
            } else {
                RECT {
                    left: 0,
                    top: 0,
                    right: GetSystemMetrics(SM_CXSCREEN),
                    bottom: GetSystemMetrics(SM_CYSCREEN),
                }
            }
        }
    }
    pub fn place(handle: isize, position: Option<(i32, i32)>) -> Result<(i32, i32), String> {
        unsafe {
            let h = handle as HWND;
            let mut r = RECT::default();
            if GetWindowRect(h, &mut r) == 0 {
                return Err("cannot position window".into());
            }
            if let Some((x, y)) = position {
                SetWindowPos(
                    h,
                    std::ptr::null_mut(),
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            let a = area(handle);
            let w = r.right - r.left;
            let height = r.bottom - r.top;
            let (x, y) = position.unwrap_or((a.right - w - 24, a.bottom - height - 24));
            let x = x.clamp(a.left, (a.right - w).max(a.left));
            let y = y.clamp(a.top, (a.bottom - height).max(a.top));
            if SetWindowPos(
                h,
                std::ptr::null_mut(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok((x, y))
        }
    }
    pub fn beside(message: isize, pet: isize) -> Result<(), String> {
        unsafe {
            let mut r = RECT::default();
            let mut bubble = RECT::default();
            if GetWindowRect(pet as HWND, &mut r) == 0
                || GetWindowRect(message as HWND, &mut bubble) == 0
            {
                return Err("cannot anchor message".into());
            }
            let a = area(pet);
            let width = bubble.right - bubble.left;
            let height = bubble.bottom - bubble.top;
            let x = if r.left - width - 12 >= a.left {
                r.left - width - 12
            } else {
                r.right + 12
            };
            let x = x.clamp(a.left, (a.right - width).max(a.left));
            let y = (r.bottom - height).clamp(a.top, (a.bottom - height).max(a.top));
            if SetWindowPos(
                message as HWND,
                HWND_TOPMOST,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE,
            ) == 0
            {
                return Err("cannot position message".into());
            }
            Ok(())
        }
    }
    pub fn position(handle: isize) -> Option<(i32, i32)> {
        unsafe {
            let mut r = RECT::default();
            (GetWindowRect(handle as HWND, &mut r) != 0).then_some((r.left, r.top))
        }
    }
    pub fn cursor() -> Option<(i32, i32)> {
        unsafe {
            let mut p = POINT::default();
            (GetCursorPos(&mut p) != 0).then_some((p.x, p.y))
        }
    }
    pub fn facts(handle: isize) -> WindowFacts {
        unsafe {
            let h = handle as HWND;
            let style = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            let normal = GetWindowLongPtrW(h, GWL_STYLE) as u32;
            let mut client = RECT::default();
            let mut outer = RECT::default();
            let bounds_valid =
                GetClientRect(h, &mut client) != 0 && GetWindowRect(h, &mut outer) != 0;
            let region = CreateRectRgn(0, 0, 0, 0);
            let present = GetWindowRgn(h, region) > 0;
            let clipped = present && PtInRegion(region, 0, 0) == 0;
            DeleteObject(region);
            WindowFacts {
                no_activate: style & WS_EX_NOACTIVATE != 0,
                tool_window: style & WS_EX_TOOLWINDOW != 0,
                topmost: style & WS_EX_TOPMOST != 0,
                clipped_corner: clipped,
                native_frame: normal & (WS_CAPTION | WS_THICKFRAME) != 0
                    || style & (WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME) != 0,
                client_fills_window: bounds_valid
                    && client.right - client.left == outer.right - outer.left
                    && client.bottom - client.top == outer.bottom - outer.top,
            }
        }
    }
    pub fn foreground() -> isize {
        unsafe { GetForegroundWindow() as isize }
    }
    /// QA readback of pixels actually presented to this process's client DC.
    /// A renderer snapshot would repaint its cache and hide restore failures.
    pub fn client_pixel_for_test(handle: isize, x: i32, y: i32) -> Option<[u8; 3]> {
        unsafe {
            let h = handle as HWND;
            let mut process = 0;
            GetWindowThreadProcessId(h, &mut process);
            if process != std::process::id() {
                return None;
            }
            let dc = GetDC(h);
            if dc.is_null() {
                return None;
            }
            let pixel = GetPixel(dc, x, y);
            ReleaseDC(h, dc);
            (pixel != u32::MAX).then_some([pixel as u8, (pixel >> 8) as u8, (pixel >> 16) as u8])
        }
    }
    /// Simulate Windows losing a client surface; restricted to this process.
    pub fn erase_client_for_test(handle: isize) -> bool {
        unsafe {
            let h = handle as HWND;
            let mut process = 0;
            GetWindowThreadProcessId(h, &mut process);
            if process != std::process::id() {
                return false;
            }
            let mut r = RECT::default();
            if GetClientRect(h, &mut r) == 0 {
                return false;
            }
            let dc = GetDC(h);
            if dc.is_null() {
                return false;
            }
            let cleared = PatBlt(dc, 0, 0, r.right, r.bottom, BLACKNESS) != 0;
            ReleaseDC(h, dc);
            cleared
        }
    }
    pub fn environment() -> Environment {
        unsafe {
            let desktop = OpenInputDesktop(0, 0, DESKTOP_READOBJECTS);
            let locked = if desktop.is_null() {
                true
            } else {
                let mut name = [0u16; 128];
                let mut needed = 0;
                let ok = GetUserObjectInformationW(
                    desktop,
                    UOI_NAME,
                    name.as_mut_ptr().cast(),
                    256,
                    &mut needed,
                );
                CloseDesktop(desktop);
                ok == 0
                    || !String::from_utf16_lossy(
                        &name[..name.iter().position(|n| *n == 0).unwrap_or(name.len())],
                    )
                    .eq_ignore_ascii_case("default")
            };
            let mut state = 0;
            let query = SHQueryUserNotificationState(&mut state);
            let foreground = GetForegroundWindow();
            let mut process = 0;
            GetWindowThreadProcessId(foreground, &mut process);
            let own = process == std::process::id();
            let mut r = RECT::default();
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let mut class = [0u16; 128];
            GetClassNameW(foreground, class.as_mut_ptr(), 128);
            let class = String::from_utf16_lossy(
                &class[..class.iter().position(|v| *v == 0).unwrap_or(128)],
            );
            let desktop_window = matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd");
            let covers = !own
                && !desktop_window
                && GetWindowRect(foreground, &mut r) != 0
                && GetMonitorInfoW(
                    MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST),
                    &mut info,
                ) != 0
                && r.left <= info.rcMonitor.left
                && r.top <= info.rcMonitor.top
                && r.right >= info.rcMonitor.right
                && r.bottom >= info.rcMonitor.bottom;
            Environment {
                locked,
                fullscreen: !own && (covers || matches!(state, 2 | 3)),
                notifications: if query < 0 {
                    kavychka_core::engine::NotificationState::Unknown
                } else {
                    match state {
                        4 | 6 => kavychka_core::engine::NotificationState::Blocked,
                        2 | 3 | 5 | 7 => kavychka_core::engine::NotificationState::Allowed,
                        _ => kavychka_core::engine::NotificationState::Unknown,
                    }
                },
                session_unknown: query >= 0 && state == 1 && !locked,
                suspended: false,
                power: crate::power_source(),
            }
        }
    }
    pub fn animation_allowed() -> bool {
        unsafe {
            let mut animated = 1i32;
            let mut power = SYSTEM_POWER_STATUS::default();
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&mut animated as *mut i32).cast(),
                0,
            ) != 0
                && animated != 0
                && (GetSystemPowerStatus(&mut power) == 0 || power.ACLineStatus != 0)
        }
    }
    pub fn post_close(handle: isize) -> bool {
        unsafe { PostMessageW(handle as HWND, WM_CLOSE, 0, 0) != 0 }
    }
    pub fn activate(handle: isize) {
        unsafe {
            SetForegroundWindow(handle as HWND);
        }
    }
    pub fn tray_window() -> isize {
        unsafe {
            let name: Vec<u16> = "SlintSystemTrayWindow"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let mut found = std::ptr::null_mut();
            loop {
                found = FindWindowExW(HWND_MESSAGE, found, name.as_ptr(), std::ptr::null());
                if found.is_null() {
                    return 0;
                }
                let mut process = 0;
                GetWindowThreadProcessId(found, &mut process);
                if process == std::process::id() {
                    return found as isize;
                }
            }
        }
    }
    pub fn tray_registered() -> bool {
        unsafe {
            let hwnd = tray_window();
            if hwnd == 0 {
                return false;
            }
            let id = NOTIFYICONIDENTIFIER {
                cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: hwnd as HWND,
                uID: 1,
                ..Default::default()
            };
            let mut r = RECT::default();
            Shell_NotifyIconGetRect(&id, &mut r) >= 0
        }
    }
    /// QA actions are confined to this process's Slint tray, pinned to Slint 1.18.
    pub fn tray_click_for_test() -> bool {
        unsafe {
            let hwnd = tray_window();
            if hwnd == 0 {
                return false;
            }
            PostMessageW(hwnd as HWND, WM_APP + 1, 0, WM_LBUTTONUP as isize) != 0
        }
    }
    pub fn lose_tray_for_test() -> bool {
        unsafe {
            let hwnd = tray_window();
            if hwnd == 0 {
                return false;
            }
            let data = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd as HWND,
                uID: 1,
                ..Default::default()
            };
            Shell_NotifyIconW(NIM_DELETE, &data) != 0
        }
    }
}
#[cfg(windows)]
pub use windows::*;
#[cfg(not(windows))]
mod unsupported {
    use super::*;
    pub fn environment() -> Environment {
        Environment {
            notifications: kavychka_core::engine::NotificationState::Unknown,
            session_unknown: true,
            ..Default::default()
        }
    }
    pub fn animation_allowed() -> bool {
        false
    }
    pub fn overlay(_: isize, _: Overlay, _: f32) -> Result<(), String> {
        Err("Windows adapter required".into())
    }
    pub fn alpha_region(_: isize, _: &[[i32; 4]]) -> Result<(), String> {
        Err("Windows adapter required".into())
    }
    pub fn region_contains(_: isize, _: i32, _: i32) -> Option<bool> {
        None
    }
    pub fn place(_: isize, _: Option<(i32, i32)>) -> Result<(i32, i32), String> {
        Err("Windows adapter required".into())
    }
    pub fn beside(_: isize, _: isize) -> Result<(), String> {
        Err("Windows adapter required".into())
    }
    pub fn position(_: isize) -> Option<(i32, i32)> {
        None
    }
    pub fn cursor() -> Option<(i32, i32)> {
        None
    }
    pub fn foreground() -> isize {
        0
    }
    pub fn client_pixel_for_test(_: isize, _: i32, _: i32) -> Option<[u8; 3]> {
        None
    }
    pub fn erase_client_for_test(_: isize) -> bool {
        false
    }
    pub fn facts(_: isize) -> WindowFacts {
        WindowFacts {
            no_activate: false,
            tool_window: false,
            topmost: false,
            clipped_corner: false,
            native_frame: false,
            client_fills_window: false,
        }
    }
    pub fn post_close(_: isize) -> bool {
        false
    }
    pub fn activate(_: isize) {}
    pub fn tray_window() -> isize {
        0
    }
    pub fn tray_registered() -> bool {
        false
    }
    pub fn tray_click_for_test() -> bool {
        false
    }
    pub fn lose_tray_for_test() -> bool {
        false
    }
}
#[cfg(not(windows))]
pub use unsupported::*;

#[cfg(all(test, windows))]
mod overlay_tests {
    use super::*;
    use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use windows_sys::Win32::{Foundation::*, UI::Shell::*};
    static PAINT_CALLS: AtomicU32 = AtomicU32::new(0);
    static NC_LPARAM: AtomicIsize = AtomicIsize::new(0);
    unsafe extern "system" fn paint_spy(
        h: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        if msg == WM_NCPAINT {
            PAINT_CALLS.fetch_add(1, Ordering::Relaxed);
        }
        if msg == WM_NCACTIVATE {
            NC_LPARAM.store(lp, Ordering::Relaxed);
        }
        unsafe { DefSubclassProc(h, msg, wp, lp) }
    }

    #[test]
    fn overlays_keep_chrome_and_regions_correct_across_size_dpi_and_style_updates() {
        struct OwnedWindow(windows_sys::Win32::Foundation::HWND);
        impl Drop for OwnedWindow {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        unsafe {
            let class: Vec<u16> = "STATIC".encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "Kavychka owned invisible overlay test"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            for kind in [Overlay::Pet, Overlay::Message] {
                let before = GetForegroundWindow();
                let h = CreateWindowExW(
                    WS_EX_WINDOWEDGE,
                    class.as_ptr(),
                    title.as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    400,
                    280,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                );
                assert!(!h.is_null());
                let _window = OwnedWindow(h);
                assert!(facts(h as isize).native_frame);
                assert_ne!(SetWindowSubclass(h, Some(paint_spy), 0x51414652, 0), 0);
                overlay(h as isize, kind, 1.).unwrap();
                for _ in 0..3 {
                    PAINT_CALLS.store(0, Ordering::Relaxed);
                    for active in [0, 1, 0] {
                        assert_eq!(SendMessageW(h, WM_NCACTIVATE, active, 0), 1);
                        assert_eq!(NC_LPARAM.load(Ordering::Relaxed), -1);
                        assert_eq!(SendMessageW(h, WM_NCPAINT, 1, 0), 0);
                    }
                    assert_eq!(
                        PAINT_CALLS.load(Ordering::Relaxed),
                        0,
                        "Native caption paint reached the backend"
                    );
                    assert_eq!(
                        SendMessageW(h, WM_NCHITTEST, 0, 80 | (80 << 16)),
                        HTCLIENT as isize
                    );
                    let f = facts(h as isize);
                    assert!(!f.native_frame && f.client_fills_window, "{f:?}");
                    assert!(f.no_activate && f.tool_window && f.topmost && f.clipped_corner);
                    // Model a backend updating styles during hide/show/theme changes.
                    let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
                    SetWindowLongPtrW(
                        h,
                        GWL_STYLE,
                        (style | WS_CAPTION | WS_THICKFRAME | WS_SYSMENU) as isize,
                    );
                    let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
                    SetWindowLongPtrW(
                        h,
                        GWL_EXSTYLE,
                        ((ex & !(WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW))
                            | WS_EX_APPWINDOW
                            | WS_EX_WINDOWEDGE) as isize,
                    );
                    assert_ne!(
                        SetWindowPos(
                            h,
                            HWND_TOPMOST,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED
                        ),
                        0
                    );
                }
                let f = facts(h as isize);
                assert!(!f.native_frame && f.client_fills_window && f.no_activate && f.tool_window);
                assert_eq!(GetForegroundWindow(), before);
            }
            for logical in [116., 130., 200., 240.] {
                for scale in [1., 1.25, 1.5, 2.] {
                    let size = (logical * scale) as i32;
                    let h = CreateWindowExW(
                        0,
                        class.as_ptr(),
                        title.as_ptr(),
                        WS_OVERLAPPEDWINDOW,
                        0,
                        0,
                        size,
                        size,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    );
                    assert!(!h.is_null());
                    let _window = OwnedWindow(h);
                    overlay(h as isize, Overlay::Pet, scale).unwrap();
                    // These points belong to Slint's 26 logical-pixel corner.
                    // Scaling that radius by size/116 cuts them off at size130+.
                    let p = (16. * scale) as i32;
                    for (x, y) in [
                        (p, p),
                        (size - 1 - p, p),
                        (p, size - 1 - p),
                        (size - 1 - p, size - 1 - p),
                    ] {
                        assert_eq!(
                            region_contains(h as isize, x, y),
                            Some(true),
                            "size={logical}, dpi={scale}, point=({x},{y})"
                        );
                    }
                    assert_eq!(region_contains(h as isize, 0, 0), Some(false));
                    assert_eq!(region_contains(h as isize, size / 2, size / 2), Some(true));
                }
            }
        }
    }
}
