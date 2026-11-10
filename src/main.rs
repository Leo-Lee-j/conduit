#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(all(test, feature = "allocation-probe"))]
mod allocation_probe;
mod app;
mod approval;
mod ccs;
mod chat_runtime;
mod child;
mod cli;
mod cli_adapter;
mod commands;
mod domain;
mod engines;
mod event_queue;
mod extensions;
mod failure;
mod file_ref;
mod file_search;
mod git;
mod history;
mod message_status;
mod models;
mod notifications;
mod performance;
mod persistence;
mod protocol;
mod providers;
mod store;
mod terminal;
mod transcript;
mod wallpaper;
mod workspace_files;

#[cfg(conduit_shell_desktop)]
#[path = "../tools/shell-prototype/desktop/args.rs"]
mod development_args;

use app::ConduitApp;
use gpui_kit::component::TitleBar;
use gpui_kit::*;

fn main() {
    let _installation_lock = match std::env::current_exe()
        .map_err(anyhow::Error::from)
        .and_then(|exe| conduit::update::install::running_lock(&exe))
    {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("{error:#}");
            return;
        }
    };
    let start_in_settings = std::env::args_os().any(|arg| arg == "--settings");
    let start_in_git = std::env::args_os().any(|arg| arg == "--git");
    if std::env::args_os().any(|arg| arg == "--history") {
        return print_history();
    }
    #[cfg(conduit_shell_desktop)]
    let development_package = match development_args::parse(std::env::args_os().skip(1)) {
        Ok(package) => package,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(2);
        }
    };
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
    // Load once: the constructor would otherwise parse the same state file a
    // second time after the window already exists.
    let loaded = store::Store::load().map_err(|error| format!("{error:#}"));
    let appearance = loaded
        .as_ref()
        .map(|store| store.ui.appearance)
        .unwrap_or_default();
    let language = loaded
        .as_ref()
        .map(|store| store.ui.language.clone())
        .unwrap_or_default();
    conduit::i18n::set_locale(&language);
    gpui_kit::application()
        // The full icon catalog keeps the UI glyphs that the default component
        // bundle does not embed (e.g. the composer Pin and the settings CLI
        // terminal icon) rendering.
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            #[cfg(conduit_shell_desktop)]
            gpui_shell::init(cx);
            conduit::i18n::set_locale(&language);
            app::install_theme_packs(cx);
            let initial_mode = match appearance {
                store::AppearanceMode::System => {
                    gpui_kit::component::ThemeMode::from(cx.window_appearance())
                }
                store::AppearanceMode::Light => gpui_kit::component::ThemeMode::Light,
                store::AppearanceMode::Dark => gpui_kit::component::ThemeMode::Dark,
            };
            app::set_palette(initial_mode);
            gpui_kit::component::Theme::change(initial_mode, None, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1200.), px(800.)), cx)),
                window_min_size: Some(size(px(850.), px(500.))),
                // Draw our own title bar (brand row + window controls) instead
                // of the native Windows caption.
                //
                // Stay hidden until the first frame exists. GPUI's composition
                // surface is empty before that, so showing at creation exposes
                // a transparent window while startup work blocks the thread.
                show: false,
                ..TitleBar::window_options()
            };
            #[cfg(conduit_shell_desktop)]
            let app_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
            #[cfg(conduit_shell_desktop)]
            let app_output = app_slot.clone();
            let (window, _content) = gpui_kit::open_window(options, cx, move |window, cx| {
                window.set_window_title("Conduit");
                let app = cx.new(|cx| {
                    let mut app =
                        ConduitApp::with_store(window, cx, loaded, start_in_settings, start_in_git);
                    #[cfg(conduit_shell_desktop)]
                    if let Some(packages) = development_package
                        && let Err(error) = app.install_development_packages(packages, cx)
                    {
                        eprintln!(
                            "{}",
                            conduit::i18n::format(
                                "main.failed_to_load_development_plugin",
                                &[("error", format!("{:#}", error))]
                            )
                        );
                    }
                    // The loading page is the window's very first frame; the
                    // initial history import below releases it once merged.
                    app.begin_startup_loading();
                    app
                });
                #[cfg(conduit_shell_desktop)]
                {
                    *app_output.borrow_mut() = Some(app.clone());
                    cx.new(|cx| {
                        gpui_shell::ShellRoot::new(app.into(), window, cx).with_uncached_content()
                    })
                }
                #[cfg(not(conduit_shell_desktop))]
                app
            })
            .expect("failed to open Conduit window");
            #[cfg(conduit_shell_desktop)]
            let app = app_slot.borrow_mut().take().expect("desktop app created");
            #[cfg(not(conduit_shell_desktop))]
            let app = _content;
            // Ask to activate only after launch; the request is dropped while
            // the application is still starting. The first frame has already
            // been drawn, so the window appears with content instead of a
            // blank surface.
            cx.spawn(async move |cx| {
                cx.update(|cx| {
                    cx.activate(true);
                    let _ = window.update(cx, |_, window, cx| {
                        app.update(cx, |app, cx| {
                            app.start_initial_history_imports(window, cx);
                        });
                        window.activate_window();
                    });
                });
            })
            .detach();
        });
}

/// Debug entry: `conduit --history [workspace]` prints CLI-side sessions
/// discovered for a workspace without opening the GUI. Defaults to the
/// current directory.
fn print_history() {
    let workspace = std::env::args()
        .nth(
            std::env::args()
                .position(|a| a == "--history")
                .map(|i| i + 1)
                .unwrap_or(usize::MAX),
        )
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let workspace = workspace.canonicalize().unwrap_or(workspace);
    let sessions = history::scan_workspace(&workspace);
    if sessions.is_empty() {
        println!(
            "{}",
            conduit::i18n::format(
                "main.no_sessions_for_found_in_cli_history",
                &[("arg0", format!("{}", workspace.display()))]
            )
        );
        return;
    }
    println!(
        "{}",
        conduit::i18n::format(
            "main.found_cli_history_sessions_in_workspace",
            &[
                ("arg0", format!("{}", workspace.display())),
                ("arg1", format!("{}", sessions.len()))
            ]
        )
    );
    for session in &sessions {
        println!(
            "{}",
            conduit::i18n::format(
                "main.native_id_messages",
                &[
                    ("arg0", format!("{:?}", session.provider)),
                    ("arg1", chrono_like(session.created_at).to_string()),
                    ("arg2", session.native_id.to_string()),
                    ("arg3", session.title.to_string()),
                    ("arg4", format!("{}", session.messages.len())),
                    ("arg5", session.model.as_deref().unwrap_or("-").to_string())
                ]
            )
        );
    }
}

fn chrono_like(unix: u64) -> String {
    // Small local formatter; avoids pulling a date crate into the binary.
    let days = unix / 86_400;
    let secs = unix % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u64, u64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u64;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u64;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

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
            anyhow::bail!(conduit::i18n::text(
                "main.conduit_is_already_running_use_the_existing"
            ));
        }
        Ok(handle)
    }
}

#[cfg(test)]
mod test_support;
