//! Windows adapter for a bare, alpha-composited desktop character.
//!
//! gpui-pre-windows 0.3.9 creates PopUp with style 0 (WS_OVERLAPPED), which
//! Windows turns into a captioned window. Transparent GPUI pixels alone cannot
//! remove that non-client frame. Keep this workaround local to the pet window.
use gpui_kit::{App, Bounds, Pixels, Window};

#[cfg(target_os = "windows")]
pub(super) fn handle(window: &Window) -> Option<windows_sys::Win32::Foundation::HWND> {
    let raw_window_handle::RawWindowHandle::Win32(handle) =
        raw_window_handle::HasWindowHandle::window_handle(window)
            .ok()?
            .as_raw()
    else {
        return None;
    };
    Some(handle.hwnd.get() as windows_sys::Win32::Foundation::HWND)
}

pub(super) fn prepare(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    cx: &mut App,
) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    if let Some(hwnd) = handle(window) {
        use windows_sys::Win32::{Graphics::Dwm::*, UI::WindowsAndMessaging::*};
        unsafe {
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            anyhow::ensure!(
                ex_style & WS_EX_NOREDIRECTIONBITMAP != 0,
                "Desktop pet transparency requires DirectComposition; remove GPUI_DISABLE_DIRECT_COMPOSITION and restart."
            );
            SetWindowLongW(
                hwnd,
                GWL_STYLE,
                ((style
                    & !(WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX))
                    | WS_POPUP) as i32,
            );
            SetWindowLongW(
                hwnd,
                GWL_EXSTYLE,
                ((ex_style
                    & !(WS_EX_APPWINDOW
                        | WS_EX_WINDOWEDGE
                        | WS_EX_CLIENTEDGE
                        | WS_EX_DLGMODALFRAME
                        | WS_EX_STATICEDGE))
                    | WS_EX_TOOLWINDOW) as i32,
            );
            // Disabling non-client rendering removes DWM's frame and shadow.
            let policy = DWMNCRP_DISABLED;
            let result = DwmSetWindowAttribute(
                hwnd,
                DWMWA_NCRENDERING_POLICY as u32,
                (&policy as *const i32).cast(),
                std::mem::size_of_val(&policy) as u32,
            );
            anyhow::ensure!(
                result >= 0,
                "Cannot disable desktop pet window decoration: {result:#x}"
            );
            // These Windows 11 attributes are optional on Windows 10.
            for (attribute, value) in [
                (DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND as u32),
                (DWMWA_BORDER_COLOR, 0xfffffffe), // DWMWA_COLOR_NONE
                (DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE as u32),
            ] {
                DwmSetWindowAttribute(hwnd, attribute as u32, (&value as *const u32).cast(), 4);
            }
            let scale = window.scale_factor();
            // SetWindowPos uses the actual frame-free outer/client size. Do not
            // call GPUI resize here: its old border offsets are still cached.
            let result = SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                (f32::from(bounds.origin.x) * scale).round() as i32,
                (f32::from(bounds.origin.y) * scale).round() as i32,
                (f32::from(bounds.size.width) * scale).round() as i32,
                (f32::from(bounds.size.height) * scale).round() as i32,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOZORDER,
            );
            anyhow::ensure!(
                result != 0,
                "Cannot position desktop pet: {}",
                std::io::Error::last_os_error()
            );
        }
        // Native callbacks cannot update a Window while its GPUI lease is held.
        // Sync explicitly or layout keeps the previous viewport and clips art.
        window.bounds_changed(cx);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (window, bounds, cx);
    Ok(())
}

/// Called after preparing the first scene, without activating the app.
pub(super) fn show(window: &Window) {
    #[cfg(target_os = "windows")]
    if let Some(hwnd) = handle(window) {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE,
            );
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = window;
}
