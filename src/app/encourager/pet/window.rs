#[cfg(target_os = "windows")]
use super::super::native_window::handle as native_handle;
use super::*;

const EDGE_INSET: f32 = 24.;
pub(super) const WITH_BUBBLE: (f32, f32) = (320., 312.);

pub(super) fn cell_size(scale: f32) -> Pixels {
    px((6. * scale).round().max(1.) / scale)
}
pub(super) fn compact_size(scale: f32) -> gpui_kit::Size<Pixels> {
    let cell = cell_size(scale);
    size(
        cell * pet_core::CANVAS_W as f32,
        cell * pet_core::CANVAS_H as f32,
    )
}
pub(super) fn surface_size(scale: f32) -> gpui_kit::Size<Pixels> {
    let compact = compact_size(scale);
    size(
        px(WITH_BUBBLE.0).max(compact.width),
        px(WITH_BUBBLE.1).max(compact.height),
    )
}
pub(in crate::app::encourager) fn position_from_window(
    window: &Window,
) -> crate::store::EncouragerPetPosition {
    let bounds = window.bounds();
    let compact = compact_size(window.scale_factor());
    crate::store::EncouragerPetPosition {
        x: f32::from(bounds.origin.x + bounds.size.width - compact.width).round() as i32,
        y: f32::from(bounds.origin.y + bounds.size.height - compact.height).round() as i32,
    }
}
fn visible_bounds(anchor: Point<Pixels>, cx: &App) -> Option<Bounds<Pixels>> {
    cx.displays()
        .into_iter()
        .find(|display| display.visible_bounds().contains(&anchor))
        .or_else(|| cx.primary_display())
        .map(|display| display.visible_bounds())
}
pub(in crate::app::encourager) fn pet_window_bounds(
    saved: Option<crate::store::EncouragerPetPosition>,
    cx: &App,
) -> Bounds<Pixels> {
    let anchor = saved
        .map(|p| point(px(p.x as f32), px(p.y as f32)))
        .unwrap_or_default();
    let pet_size = compact_size(1.);
    let surface = surface_size(1.);
    let Some(visible) = visible_bounds(anchor, cx) else {
        return Bounds::centered(None, surface, cx);
    };
    let min = visible.origin + point(px(EDGE_INSET), px(EDGE_INSET));
    let max = visible.origin + visible.size.into()
        - pet_size.into()
        - point(px(EDGE_INSET), px(EDGE_INSET));
    // A missing monitor goes to the primary work area's bottom right.
    let origin = if saved.is_some() && visible.contains(&anchor) {
        point(
            anchor.x.clamp(min.x, max.x.max(min.x)),
            anchor.y.clamp(min.y, max.y.max(min.y)),
        )
    } else {
        point(max.x.max(min.x), max.y.max(min.y))
    };
    let surface_origin = origin + pet_size.into() - surface.into();
    let max = visible.origin + visible.size.into() - surface.into();
    Bounds::new(
        point(
            surface_origin
                .x
                .clamp(visible.origin.x, max.x.max(visible.origin.x)),
            surface_origin
                .y
                .clamp(visible.origin.y, max.y.max(visible.origin.y)),
        ),
        surface,
    )
}
pub(super) fn resize_window(window: &mut Window, cx: &mut App) {
    let next_size = surface_size(window.scale_factor());
    let old = window.bounds();
    if old.size == next_size {
        return;
    }
    let mut origin = old.origin + old.size.into() - next_size.into();
    if let Some(visible) = visible_bounds(old.center(), cx) {
        let max = visible.origin + visible.size.into() - next_size.into();
        origin.x = origin
            .x
            .clamp(visible.origin.x, max.x.max(visible.origin.x));
        origin.y = origin
            .y
            .clamp(visible.origin.y, max.y.max(visible.origin.y));
    }
    #[cfg(target_os = "windows")]
    if let Some(hwnd) = native_handle(window) {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let scale = window.scale_factor();
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                (f32::from(origin.x) * scale).round() as i32,
                (f32::from(origin.y) * scale).round() as i32,
                (f32::from(next_size.width) * scale).round() as i32,
                (f32::from(next_size.height) * scale).round() as i32,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
        window.bounds_changed(cx);
        return;
    }
    window.resize(next_size);
}

/// Keep the composition surface stable while exposing only the actual children.
/// A native region also excludes unused transparent canvas from mouse hit testing.
pub(super) fn visible_region(bounds: &[Bounds<Pixels>], scale: f32) -> Vec<[i32; 4]> {
    bounds
        .iter()
        .map(|bounds| {
            [
                (f32::from(bounds.origin.x) * scale).round() as i32,
                (f32::from(bounds.origin.y) * scale).round() as i32,
                (f32::from(bounds.right()) * scale).round() as i32,
                (f32::from(bounds.bottom()) * scale).round() as i32,
            ]
        })
        .collect()
}
pub(super) fn set_visible_region(window: &Window, rects: &[[i32; 4]]) {
    #[cfg(target_os = "windows")]
    if let Some(hwnd) = native_handle(window) {
        use windows_sys::Win32::Graphics::Gdi::*;
        unsafe {
            let region = CreateRectRgn(0, 0, 0, 0);
            if region.is_null() {
                return;
            }
            for &[left, top, right, bottom] in rects {
                let part = CreateRectRgn(left, top, right, bottom);
                if part.is_null() {
                    DeleteObject(region);
                    return;
                }
                let combined = CombineRgn(region, region, part, RGN_OR);
                DeleteObject(part);
                if combined == 0 {
                    DeleteObject(region);
                    return;
                }
            }
            // The OS owns the region only after success. The new scene has already
            // been drawn; do not request an extra erase/repaint during this change.
            if SetWindowRgn(hwnd, region, 0) == 0 {
                DeleteObject(region);
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (window, rects);
}

#[derive(Clone, Copy)]
pub(super) struct PetPress {
    pub(super) local: Point<Pixels>,
    pub(super) dragged: bool,
    #[cfg(target_os = "windows")]
    pub(super) native: Option<(i32, i32, i32, i32)>,
}
impl PetPress {
    pub(super) fn new(local: Point<Pixels>, _window: &Window) -> Self {
        Self {
            local,
            dragged: false,
            #[cfg(target_os = "windows")]
            native: native_press(_window),
        }
    }
}
#[cfg(target_os = "windows")]
fn native_press(window: &Window) -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::{
        Foundation::{POINT, RECT},
        UI::WindowsAndMessaging::*,
    };
    let hwnd = native_handle(window)?;
    let mut point = POINT::default();
    let mut rect = RECT::default();
    unsafe {
        if GetCursorPos(&mut point) == 0 || GetWindowRect(hwnd, &mut rect) == 0 {
            return None;
        }
    }
    Some((point.x, point.y, rect.left, rect.top))
}
#[cfg(target_os = "windows")]
pub(super) fn native_motion(press: PetPress) -> Option<(bool, bool)> {
    use windows_sys::Win32::{
        Foundation::POINT,
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
            WindowsAndMessaging::GetCursorPos,
        },
    };
    let (x, y, _, _) = press.native?;
    let mut point = POINT::default();
    unsafe {
        if GetCursorPos(&mut point) == 0 {
            return Some((false, false));
        }
        Some((
            GetAsyncKeyState(VK_LBUTTON as i32) < 0,
            (point.x - x).abs().max((point.y - y).abs()) >= 4,
        ))
    }
}
#[cfg(target_os = "windows")]
pub(super) fn native_drag(
    window: &mut Window,
    (x, y, left, top): (i32, i32, i32, i32),
    cx: &mut App,
) {
    use windows_sys::Win32::{
        Foundation::{POINT, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
        UI::WindowsAndMessaging::*,
    };
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    let mut point = POINT::default();
    unsafe {
        if GetCursorPos(&mut point) != 0 {
            let mut next_left = left + point.x - x;
            let mut next_top = top + point.y - y;
            let mut frame = RECT::default();
            let mut monitor = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetWindowRect(hwnd, &mut frame) != 0
                && GetMonitorInfoW(
                    MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST),
                    &mut monitor,
                ) != 0
            {
                // Keep the reserved popup area on-screen while dragging, so opening
                // it later neither clips the reply nor moves the character.
                let work = monitor.rcWork;
                next_left = next_left.clamp(
                    work.left,
                    (work.right - (frame.right - frame.left)).max(work.left),
                );
                next_top = next_top.clamp(
                    work.top,
                    (work.bottom - (frame.bottom - frame.top)).max(work.top),
                );
            }
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                next_left,
                next_top,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE,
            );
        }
    }
    window.bounds_changed(cx);
}
