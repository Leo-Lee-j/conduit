#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod approval;
mod ccs;
mod chat_runtime;
mod child;
mod cli;
mod domain;
mod models;
mod protocol;
mod store;

use app::ConduitApp;
use gpui_kit::*;

fn main() {
    let start_in_settings = std::env::args_os().any(|arg| arg == "--settings");
    let _instance = match single_instance() {
        Ok(instance) => instance,
        Err(error) => {
            eprintln!("{error:#}");
            let text: Vec<u16> = format!("{error:#}").encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "Conduit".encode_utf16().chain(Some(0)).collect();
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    std::ptr::null_mut(),
                    text.as_ptr(),
                    title.as_ptr(),
                    windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                        | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONINFORMATION,
                );
            }
            return;
        }
    };
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1200.), px(800.)), cx)),
                window_min_size: Some(size(px(850.), px(500.))),

                ..Default::default()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                window.set_window_title("Conduit");
                apply_dark_titlebar(window);
                cx.new(|cx| ConduitApp::new(window, cx, start_in_settings))
            })
            .expect("failed to open Conduit window");
        });
}

#[cfg(windows)]
fn apply_dark_titlebar(window: &gpui_kit::Window) {
    use raw_window_handle::RawWindowHandle;
    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let enabled: i32 = 1;
    unsafe {
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            win32.hwnd.get() as *mut std::ffi::c_void,
            windows_sys::Win32::Graphics::Dwm::DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &enabled as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

#[cfg(not(windows))]
fn apply_dark_titlebar(_: &gpui_kit::Window) {}

fn single_instance() -> anyhow::Result<std::os::windows::io::OwnedHandle> {
    use std::{
        hash::{Hash, Hasher},
        os::windows::io::{FromRawHandle, OwnedHandle},
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
        System::Threading::CreateMutexW,
    };
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let path = store::Store::path();
    let parent = path.parent().unwrap();
    let parent = parent.canonicalize().unwrap_or_else(|_| {
        if parent.is_absolute() {
            parent.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(parent)
        }
    });
    parent
        .join("state.json")
        .to_string_lossy()
        .to_lowercase()
        .hash(&mut hash);
    let name: Vec<u16> = format!("Local\\Conduit-{:x}", hash.finish())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let existing = GetLastError() == ERROR_ALREADY_EXISTS;
        let handle = OwnedHandle::from_raw_handle(handle);
        if existing {
            anyhow::bail!("Conduit 已在运行，请使用已有窗口");
        }
        Ok(handle)
    }
}
