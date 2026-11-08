use super::{ConduitApp, Store};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, TestAppContext,
    VisualTestContext, px, size,
};

fn fixture(cx: &mut TestAppContext) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = ConduitApp::with_store(window, cx, Ok(Store::default()), false, false);
            app.startup = None;
            app.writable = false;
            app.connection_settings.syncing = true;
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.simulate_resize(size(px(850.), px(500.)));
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
    draw(cx);
    (app, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

#[gpui_kit::test]
fn settings_engine_management_is_readonly_and_disabled_launches_preserve_history(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use crate::domain::Provider;
    let (app, cx) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.connection_settings.engine = Provider::Codex;
            app.select_settings_page(3, window, cx);
            app.set_engine_enabled(Provider::Codex, false, cx);
            assert!(
                app.store.cli.enabled(Provider::Codex),
                "read-only mutation must be rejected"
            );
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("engine-enabled-1").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            app.store.cli.set_enabled(Provider::Codex, false);
            app.new_session_provider = Provider::Claude;
            app.select_engine(Provider::Codex, window, cx);
            assert_eq!(app.new_session_provider, Provider::Claude);
            app.store.workspaces.push(crate::domain::Workspace {
                path: std::env::temp_dir(),
                sessions: vec![],
            });
            let before = serde_json::to_value(&app.store.workspaces).unwrap();
            let error = app
                .start_turn(0, None, Provider::Codex, "offline prompt".into(), true, cx)
                .unwrap_err();
            assert!(error.contains("已停用"));
            assert_eq!(serde_json::to_value(&app.store.workspaces).unwrap(), before);
            assert!(app.session_control.runs.is_empty());
            app.writable = false;
        })
    });
}

#[gpui_kit::test]
fn settings_extension_filters_keep_the_list_visible_at_small_and_large_sizes(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use crate::{
        domain::Provider,
        extensions::{Catalog, Context, Entry, Kind, management::Filter},
    };
    let (app, cx) = fixture(cx);
    for (width, height) in [(850., 500.), (1280., 900.)] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.settings_ui.page = 4;
                app.extensions_ui.initialized = true;
                app.extensions_ui.probe = None;
                let context = Context {
                    engine: Provider::Codex,
                    workspace: None,
                    cli_path: None,
                };
                app.extensions_ui.context = Some(context.clone());
                app.extensions_ui.catalogs.insert(
                    context,
                    Catalog {
                        entries: vec![Entry {
                            name: "sample".into(),
                            description: "Skill description".into(),
                            path: "C:/test/SKILL.md".into(),
                            scope: "全局".into(),
                            kind: Kind::Skill,
                            disabled: true,
                            transport: String::new(),
                            target: String::new(),
                        }],
                        ..Default::default()
                    },
                );
                app.extensions_ui.filters[0] = Filter::All;
                app.extensions_ui.lists[0].reset(0);
                cx.notify();
            })
        });
        draw(cx);
        let list = cx.debug_bounds("extensions-list").unwrap();
        assert!(
            list.size.height > px(50.),
            "filters must leave room for the list"
        );
        let filter = cx.debug_bounds("extensions-filter-trigger").unwrap();
        cx.simulate_mouse_down(filter.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(filter.center(), MouseButton::Left, Modifiers::default());
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.extensions_ui.filters[0] = Filter::Disabled;
                app.extensions_ui.lists[0].reset(0);
                cx.notify();
            })
        });
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(app.read_with(cx, |app, _| app.extensions_ui.filters[0]
            == Filter::Disabled));
        assert!(cx.debug_bounds("extension-detail-0").is_some());
    }
}

#[gpui_kit::test]
fn settings_keyboard_search_reveals_deep_section_and_escape_clears_before_leaving(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.simulate_keystrokes("ctrl-f");
    draw(cx);
    cx.update(|window, cx| {
        assert!(
            app.read(cx)
                .settings_ui
                .search
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        )
    });
    cx.simulate_input("提示词");
    draw(cx);
    assert!(cx.debug_bounds("settings-search-result-0").is_some());
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.settings_ui.page, 6);
        assert_eq!(app.settings_ui.group, Some(2));
        assert!(app.settings_ui.prompt_expanded);
        assert!(app.settings_ui.search.read(cx).value().is_empty());
    });
    let body = cx.debug_bounds("settings-workspace-body").unwrap();
    let prompt = cx
        .debug_bounds("git-prompt-expand")
        .expect("search opens the prompt section");
    assert!(prompt.top() >= body.top() && prompt.bottom() <= body.bottom());
    let full = cx.debug_bounds("settings-show-full-page").unwrap();
    cx.simulate_mouse_down(full.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(full.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
    assert_eq!(app.read_with(cx, |app, _| app.settings_ui.group), None);
    assert!(cx.debug_bounds("settings-show-full-page").is_none());
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("unmatched-setting-xyz");
    draw(cx);
    assert!(cx.debug_bounds("settings-search-result-0").is_none());
    cx.simulate_keystrokes("escape");
    draw(cx);
    cx.update(|_, cx| {
        assert!(app.read(cx).settings_ui.open);
        assert!(app.read(cx).settings_ui.search.read(cx).value().is_empty());
        assert_eq!(app.read(cx).settings_ui.page, 6);
    });
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!app.read_with(cx, |app, _| app.settings_ui.open));
}

#[gpui_kit::test]
fn settings_child_dialog_escape_preserves_workspace_and_navigation(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_settings_page(1, window, cx);
            app.open_about_dialog(window, cx);
        })
    });
    draw(cx);
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    cx.simulate_keystrokes("escape");
    draw(cx);
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert!(app.read(cx).settings_ui.open);
        assert_eq!(app.read(cx).settings_ui.page, 1);
    });
    assert!(cx.debug_bounds("appearance-theme-system").is_some());
}

#[gpui_kit::test]
fn settings_management_actions_fit_and_model_search_can_return_to_full_page(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use crate::domain::Provider;
    use gpui_kit::component::{Theme, ThemeMode};
    let (app, cx) = fixture(cx);
    for (width, height, mode) in [
        (850., 500., ThemeMode::Light),
        (1280., 900., ThemeMode::Dark),
        (1920., 1080., ThemeMode::Light),
        (1920., 1080., ThemeMode::Dark),
    ] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.update(|window, cx| Theme::change(mode, Some(window), cx));
        for page in [3, 8, 4, 9] {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.connection_settings.engine = Provider::Claude;
                    app.select_settings_page(page, window, cx);
                    app.extensions_ui.probe = None;
                    cx.notify();
                })
            });
            draw(cx);
            let body = cx.debug_bounds("settings-workspace-body").unwrap();
            let content = cx.debug_bounds("settings-content").unwrap();
            let actions = cx.debug_bounds("settings-page-actions");
            let engines = cx.debug_bounds("management-engines").unwrap();
            assert_eq!(content.right(), body.right());
            assert!(engines.left() >= content.left() && engines.right() <= content.right());
            let engine_selectors = if page == 4 || page == 9 {
                [
                    "extensions-engine-0",
                    "extensions-engine-1",
                    "extensions-engine-2",
                ]
            } else {
                [
                    "settings-engine-0",
                    "settings-engine-1",
                    "settings-engine-2",
                ]
            };
            for selector in engine_selectors {
                let tab = cx.debug_bounds(selector).unwrap();
                assert!(tab.left() >= engines.left() && tab.right() <= engines.right());
                assert!(
                    tab.size.width >= px(72.),
                    "engine choices must remain usable"
                );
            }
            if page == 3 {
                assert!(
                    actions.is_none(),
                    "CLI page must not reserve an empty title or actions row"
                );
                assert!(engines.top() - content.top() <= px(12.));
            } else {
                let actions = actions.unwrap();
                assert!(content.right() - actions.right() <= px(24.));
                assert!(actions.left() >= body.left() && actions.right() <= body.right());
                assert!(
                    actions.size.height <= px(48.),
                    "actions must not reserve a title row"
                );
                assert!(
                    engines.right() <= actions.left()
                        || actions.right() <= engines.left()
                        || engines.bottom() <= actions.top()
                        || actions.bottom() <= engines.top(),
                    "engine choices and page actions must not overlap in either layout"
                );
                let action = cx
                    .debug_bounds(if page == 8 {
                        "settings-add-service"
                    } else {
                        "extensions-add"
                    })
                    .unwrap();
                assert!(action.top() >= actions.top() && action.bottom() <= actions.bottom());
            }
            if page == 4 || page == 9 {
                let list = cx.debug_bounds("extensions-list").unwrap();
                assert!(
                    list.size.height > px(60.),
                    "fixed controls must leave space for extensions"
                );
                assert!(list.top() >= engines.bottom() && list.bottom() <= body.bottom());
                let scope = cx.debug_bounds("extensions-scope").unwrap();
                let filter = cx.debug_bounds("extensions-filter-trigger").unwrap();
                let refresh = cx.debug_bounds("extensions-refresh").unwrap();
                assert!(scope.right() <= filter.left() && filter.right() <= refresh.left());
                assert!(refresh.right() <= content.right());
            }
        }
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let result = app
                    .settings_search_matches("管理 CLI 模型")
                    .into_iter()
                    .find(|result| result.page == 8)
                    .unwrap();
                app.open_settings_result(result, window, cx);
            })
        });
        draw(cx);
        assert!(app.read_with(cx, |app, _| app.connection_settings.models_expanded));
        let full = cx.debug_bounds("settings-show-full-page").unwrap();
        cx.simulate_mouse_down(full.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(full.center(), MouseButton::Left, Modifiers::default());
        draw(cx);
        assert_eq!(app.read_with(cx, |app, _| app.settings_ui.group), None);
        assert!(cx.debug_bounds("settings-add-service").is_some());
    }
}

#[gpui_kit::test]
fn settings_extensions_receive_scan_results_in_workspace_without_dialog(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{
        domain::Provider,
        extensions::{Catalog, Context, Probe},
    };
    let (app, cx) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!window.has_active_dialog(cx));
            app.settings_ui.page = 4;
            let context = Context {
                engine: Provider::Codex,
                workspace: None,
                cli_path: None,
            };
            app.extensions_ui.context = Some(context.clone());
            app.extensions_ui.probe = Some(Probe::ready(
                context.clone(),
                Catalog {
                    updated: Some(std::time::SystemTime::now()),
                    errors: vec!["synthetic source diagnostic".into()],
                    ..Default::default()
                },
            ));
            app.poll_extensions(window, cx);
            assert!(app.extensions_ui.probe.is_none());
            assert_eq!(
                app.extensions_ui
                    .catalogs
                    .get(&context)
                    .expect("workspace must receive completed discovery")
                    .errors,
                ["synthetic source diagnostic"]
            );
            app.extensions_ui.probe = Some(Probe::ready(context.clone(), Catalog::default()));
            app.settings_ui.page = 3;
            app.poll_extensions(window, cx);
            assert!(
                app.extensions_ui.probe.is_none(),
                "leaving extension pages cancels discovery"
            );
            assert_eq!(
                app.extensions_ui.catalogs[&context].errors,
                ["synthetic source diagnostic"]
            );
            app.settings_ui.page = 4;
            app.extensions_ui.probe = Some(Probe::ready(context, Catalog::default()));
            app.close_settings(window, cx);
            assert!(app.extensions_ui.probe.is_none());
        })
    });
}

#[gpui_kit::test]
fn settings_tab_preserves_workspace_sidebar_and_detail_dock(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use conduit::plugins::{builtins, ids::ViewId};
    let (app, cx) = fixture(cx);
    cx.simulate_resize(size(px(1600.), px(900.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.open_right_view(
                &ViewId::new(builtins::NOTIFICATIONS).unwrap(),
                window,
                cx
            ));
            app.close_settings(window, cx);
        });
    });
    draw(cx);
    let sidebar = cx.debug_bounds("sidebar").unwrap();
    let dock = cx.debug_bounds("detail-panel").unwrap();
    let dock_width = app.read_with(cx, |app, _| app.plugins.dock.width);
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
    for page in [3, 8, 4, 9] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.select_settings_page(page, window, cx);
                app.extensions_ui.probe = None;
            });
        });
        draw(cx);
        let settings = cx.debug_bounds("settings-workspace").unwrap();
        let tab = cx.debug_bounds("settings-tab").unwrap();
        assert_eq!(cx.debug_bounds("sidebar"), Some(sidebar));
        assert_eq!(cx.debug_bounds("detail-panel"), Some(dock));
        assert!(settings.left() >= sidebar.right() && settings.right() <= dock.left());
        assert!(tab.left() >= sidebar.right() && tab.right() <= dock.left());
        assert!(tab.bottom() <= settings.top());
        assert_eq!(
            app.read_with(cx, |app, _| app.plugins.dock.width),
            dock_width
        );
        assert_eq!(
            app.read_with(cx, |app, _| app.active_document()),
            Some(crate::app::DocumentTab::Settings)
        );
    }
}

#[gpui_kit::test]
fn settings_cli_updates_report_versions_errors_and_discard_changed_paths(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{
        cli::{
            self, VersionInfo, VersionProbe,
            updates::{UpdateInfo, UpdateStatus},
        },
        domain::Provider,
    };
    let (app, cx) = fixture(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            for provider in Provider::ALL {
                app.store
                    .cli
                    .set(provider, Some(std::env::current_exe().unwrap()));
            }
        })
    });
    let original = app.read_with(cx, |app, _| serde_json::to_value(&app.store.cli).unwrap());
    for (provider, current, latest, status, expected) in [
        (
            Provider::Claude,
            "2.1.9",
            "2.1.10",
            UpdateStatus::Available,
            "发现可用更新",
        ),
        (
            Provider::Codex,
            "0.10.0",
            "0.10.0",
            UpdateStatus::Current,
            "已是最新版本",
        ),
        (
            Provider::OpenCode,
            "1.11.0",
            "1.10.0",
            UpdateStatus::Ahead,
            "当前版本高于官方稳定版",
        ),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.connection_settings.engine = provider;
                app.select_settings_page(3, window, cx);
                let path = cli::resolve(provider, &app.store.cli).unwrap().path;
                let (probe, sender) = VersionProbe::test_pending(path, true);
                app.probes.insert(provider, probe);
                app.check_cli_updates(provider, cx);
                app.probe_cli(provider, cx);
                assert!(
                    app.probes[&provider].checking_updates,
                    "duplicate actions must not replace a running check"
                );
                sender
                    .send(Ok(VersionInfo {
                        version: current.into(),
                        verified_version: false,
                        update: Some(Ok(UpdateInfo {
                            latest_version: latest.into(),
                            status,
                        })),
                    }))
                    .unwrap();
                app.poll_cli_probes(cx);
                let message = &app.diagnostics[&provider];
                assert!(
                    message.contains(current)
                        && message.contains(latest)
                        && message.contains(expected)
                );
                assert!(!app.probes.contains_key(&provider));
            })
        });
        draw(cx);
        assert!(
            cx.debug_bounds(match provider {
                Provider::Claude => "settings-check-updates-0",
                Provider::Codex => "settings-check-updates-1",
                Provider::OpenCode => "settings-check-updates-2",
            })
            .is_some()
        );
    }
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(app.diagnostics[&Provider::Claude].contains("发现可用更新"));
            assert!(app.diagnostics[&Provider::OpenCode].contains("高于官方稳定版"));
            let path = cli::resolve(Provider::Codex, &app.store.cli).unwrap().path;
            let (probe, sender) = VersionProbe::test_pending(path.clone(), true);
            app.probes.insert(Provider::Codex, probe);
            sender
                .send(Ok(VersionInfo {
                    version: "0.10.0".into(),
                    verified_version: false,
                    update: Some(Err("网络暂不可用".into())),
                }))
                .unwrap();
            app.poll_cli_probes(cx);
            assert!(app.diagnostics[&Provider::Codex].contains("当前版本 0.10.0"));
            assert!(app.diagnostics[&Provider::Codex].contains("更新检查失败：网络暂不可用"));
            let (probe, sender) =
                VersionProbe::test_pending(path.with_file_name("previous-cli.exe"), true);
            app.probes.insert(Provider::Codex, probe);
            sender
                .send(Ok(VersionInfo {
                    version: "0.9.0".into(),
                    verified_version: false,
                    update: Some(Ok(UpdateInfo {
                        latest_version: "0.10.0".into(),
                        status: UpdateStatus::Available,
                    })),
                }))
                .unwrap();
            app.poll_cli_probes(cx);
            assert_eq!(
                app.diagnostics[&Provider::Codex],
                "配置已变更，请重新检测。"
            );
            assert_eq!(
                serde_json::to_value(&app.store.cli).unwrap(),
                original,
                "read-only checks must not modify CLI settings"
            );
        })
    });
}

#[gpui_kit::test]
fn settings_cli_updates_cancel_and_keep_controls_inside_the_tab(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{
        cli::{self, VersionProbe},
        domain::Provider,
    };
    let (app, cx) = fixture(cx);
    for (width, height) in [(850., 500.), (1280., 900.)] {
        cx.simulate_resize(size(px(width), px(height)));
        let sender = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(!app.writable);
                app.store
                    .cli
                    .set(Provider::Codex, Some(std::env::current_exe().unwrap()));
                app.connection_settings.engine = Provider::Codex;
                app.select_settings_page(3, window, cx);
                let path = cli::resolve(Provider::Codex, &app.store.cli).unwrap().path;
                let (probe, sender) = VersionProbe::test_pending(path, true);
                app.probes.insert(Provider::Codex, probe);
                cx.notify();
                sender
            })
        });
        draw(cx);
        let body = cx.debug_bounds("settings-workspace-body").unwrap();
        let button = cx.debug_bounds("settings-check-updates-1").unwrap();
        assert!(button.left() >= body.left() && button.right() <= body.right());
        assert!(
            button.top() >= body.top() && button.bottom() <= body.bottom(),
            "update action must be visible at {width}x{height}: {button:?}, {body:?}"
        );
        cx.simulate_mouse_down(button.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(button.center(), MouseButton::Left, Modifiers::default());
        draw(cx);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                assert!(app.probes[&Provider::Codex].is_cancelled());
                assert!(app.diagnostics[&Provider::Codex].contains("正在取消更新检查"));
                sender.send(Err("late network failure".into())).unwrap();
                app.poll_cli_probes(cx);
                assert!(!app.probes.contains_key(&Provider::Codex));
                assert_eq!(app.diagnostics[&Provider::Codex], "更新检查已取消。");
            })
        });
    }
}

#[gpui_kit::test]
fn settings_cli_install_progress_results_and_task_interlocks(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::cli::{
        self, VersionInfo, VersionProbe,
        updates::{UpdateInfo, UpdateStatus},
    };
    use crate::domain::Provider;
    let (app, cx) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store
                .cli
                .set(Provider::Codex, Some(std::env::current_exe().unwrap()));
            app.connection_settings.engine = Provider::Codex;
            app.select_settings_page(3, window, cx);
            // A read-only settings window may check versions, but cannot install.
            assert!(!app.writable);
            app.install_cli_update(Provider::Codex, cx);
            assert!(app.probes.is_empty());
            app.writable = true;
            let path = cli::resolve(Provider::Codex, &app.store.cli).unwrap().path;
            let (mut probe, sender) = VersionProbe::test_pending(path.clone(), false);
            probe.installing_update = true;
            let (progress, receiver) = std::sync::mpsc::channel();
            probe.progress = Some(receiver);
            app.probes.insert(Provider::Codex, probe);
            progress.send("正在下载并安装…".into()).unwrap();
            app.poll_cli_probes(cx);
            assert_eq!(app.diagnostics[&Provider::Codex], "正在下载并安装…");
            assert!(app.cli_installing());
            assert!(app.send_blocked(true).unwrap().contains("正在更新"));
            assert!(
                app.generation_request(super::super::generation::GenerationKind::Naming, None)
                    .is_err()
            );
            app.probe_cli(Provider::Claude, cx);
            app.check_cli_updates(Provider::Claude, cx);
            app.install_cli_update(Provider::Claude, cx);
            assert_eq!(
                app.probes.len(),
                1,
                "installers and new probes must not overlap"
            );
            let settings = app.store.cli.clone();
            app.set_cli_path(Provider::Codex, None, cx);
            assert_eq!(
                app.store.cli, settings,
                "path cannot change during installation"
            );
            sender
                .send(Ok(VersionInfo {
                    version: "0.11.0".into(),
                    verified_version: false,
                    update: Some(Ok(UpdateInfo {
                        latest_version: "0.11.0".into(),
                        status: UpdateStatus::Current,
                    })),
                }))
                .unwrap();
            app.poll_cli_probes(cx);
            assert!(!app.cli_installing());
            assert!(app.diagnostics[&Provider::Codex].contains("更新完成，已重新检测"));
            assert!(app.diagnostics[&Provider::Codex].contains("当前版本 0.11.0"));
            let (mut probe, sender) = VersionProbe::test_pending(path, false);
            probe.installing_update = true;
            app.probes.insert(Provider::Codex, probe);
            sender
                .send(Err("更新退出码 7：permission denied".into()))
                .unwrap();
            app.poll_cli_probes(cx);
            assert!(app.diagnostics[&Provider::Codex].contains("一键更新失败"));
            assert!(app.diagnostics[&Provider::Codex].contains("permission denied"));
            assert!(!app.cli_installing());
        })
    });
}

#[gpui_kit::test]
fn settings_cli_install_button_fits_tabs_and_cancels_without_false_success(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use crate::{
        cli::{self, VersionProbe},
        domain::Provider,
    };
    let (app, cx) = fixture(cx);
    for (width, height) in [(850., 500.), (1280., 900.)] {
        cx.simulate_resize(size(px(width), px(height)));
        let sender = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.writable = true;
                app.store
                    .cli
                    .set(Provider::Codex, Some(std::env::current_exe().unwrap()));
                app.connection_settings.engine = Provider::Codex;
                app.select_settings_page(3, window, cx);
                let path = cli::resolve(Provider::Codex, &app.store.cli).unwrap().path;
                let (mut probe, sender) = VersionProbe::test_pending(path, false);
                probe.installing_update = true;
                app.probes.insert(Provider::Codex, probe);
                cx.notify();
                sender
            })
        });
        draw(cx);
        let body = cx.debug_bounds("settings-workspace-body").unwrap();
        let button = cx.debug_bounds("settings-check-updates-1").unwrap();
        assert!(button.left() >= body.left() && button.right() <= body.right());
        assert!(
            button.top() >= body.top() && button.bottom() <= body.bottom(),
            "install action must fit at {width}x{height}"
        );
        cx.simulate_mouse_down(button.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(button.center(), MouseButton::Left, Modifiers::default());
        draw(cx);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                assert!(app.probes[&Provider::Codex].is_cancelled());
                sender.send(Err("late installer error".into())).unwrap();
                app.poll_cli_probes(cx);
                assert!(!app.probes.contains_key(&Provider::Codex));
                assert!(app.diagnostics[&Provider::Codex].contains("更新已停止"));
                assert!(!app.diagnostics[&Provider::Codex].contains("更新完成"));
            })
        });
    }
}

#[gpui_kit::test]
fn settings_cli_single_update_action_tracks_results_and_resets_latest_feedback(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use super::cli_settings::CliUpdateAction;
    use crate::{
        cli::{
            self, VersionInfo, VersionProbe,
            updates::{UpdateInfo, UpdateStatus},
        },
        domain::Provider,
    };
    let (app, cx) = fixture(cx);
    for provider in Provider::ALL {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.store
                    .cli
                    .set(provider, Some(std::env::current_exe().unwrap()));
                app.connection_settings.engine = provider;
                app.select_settings_page(3, window, cx);
            })
        });
        draw(cx);
        let (probe_selector, install_selector, check_selector) = match provider {
            Provider::Claude => (
                "settings-probe-0",
                "settings-install-update-0",
                "settings-check-updates-0",
            ),
            Provider::Codex => (
                "settings-probe-1",
                "settings-install-update-1",
                "settings-check-updates-1",
            ),
            Provider::OpenCode => (
                "settings-probe-2",
                "settings-install-update-2",
                "settings-check-updates-2",
            ),
        };
        assert!(
            cx.debug_bounds(probe_selector).is_none(),
            "version-only action must be removed"
        );
        assert!(
            cx.debug_bounds(install_selector).is_none(),
            "install shares the check action"
        );
        assert!(cx.debug_bounds(check_selector).is_some());
        cx.update(|_, cx| app.update(cx, |app, cx| {
            let path = cli::resolve(provider, &app.store.cli).unwrap().path;
            let result = |status| Ok(VersionInfo {
                version: "1.0.0".into(), verified_version: false,
                update: Some(Ok(UpdateInfo { latest_version: "1.1.0".into(), status })),
            });
            let (probe, sender) = VersionProbe::test_pending(path.clone(), true);
            app.probes.insert(provider, probe);
            sender.send(result(UpdateStatus::Available)).unwrap();
            app.poll_cli_probes(cx);
            assert!(matches!(app.connection_settings.cli_updates.get(&provider), Some(CliUpdateAction::Available { path: target }) if target == &path));
            let available = app.connection_settings.cli_updates.len();
            app.poll_cli_probes(cx);
            assert_eq!(app.connection_settings.cli_updates.len(), available, "available updates stay actionable");
            let (probe, sender) = VersionProbe::test_pending(path.clone(), true);
            app.probes.insert(provider, probe);
            sender.send(result(UpdateStatus::Current)).unwrap();
            app.poll_cli_probes(cx);
            match app.connection_settings.cli_updates.get_mut(&provider).unwrap() {
                CliUpdateAction::Latest { until, .. } => {
                    assert!(*until > std::time::Instant::now());
                    *until = std::time::Instant::now() - std::time::Duration::from_millis(1);
                },
                _ => panic!("up-to-date result must show temporary feedback"),
            }
            app.poll_cli_probes(cx);
            assert!(!app.connection_settings.cli_updates.contains_key(&provider), "latest feedback must restore the check action");
            for outcome in [Err("connection failed".into()), Ok(VersionInfo {
                version: "1.0.0".into(), verified_version: false,
                update: Some(Err("request timed out".into())),
            })] {
                let (probe, sender) = VersionProbe::test_pending(path.clone(), true);
                app.probes.insert(provider, probe);
                sender.send(outcome).unwrap();
                app.poll_cli_probes(cx);
                assert!(!app.connection_settings.cli_updates.contains_key(&provider), "failed checks must allow retry");
            }
            let (probe, sender) = VersionProbe::test_pending(path.with_file_name("old-cli.exe"), true);
            app.probes.insert(provider, probe);
            sender.send(result(UpdateStatus::Available)).unwrap();
            app.poll_cli_probes(cx);
            assert!(!app.connection_settings.cli_updates.contains_key(&provider), "stale-path results cannot offer installation");
        }));
    }
}

#[gpui_kit::test]
fn settings_mcp_tool_counts_are_visible_without_expanding_and_clear_on_refresh(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use crate::{
        domain::Provider,
        extensions::{Catalog, Context, Entry, Kind, connection::ToolReport},
    };
    let (app, cx) = fixture(cx);
    let entry = Entry {
        name: "tool-count-fixture".into(),
        description: String::new(),
        path: "fixture-mcp.json".into(),
        scope: "全局".into(),
        kind: Kind::Mcp,
        disabled: false,
        transport: "stdio".into(),
        target: "fixture-server".into(),
    };
    let id = entry.id();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.settings_ui.page = 9;
            app.extensions_ui.mcp = true;
            app.extensions_ui.initialized = true;
            let context = Context {
                engine: Provider::Codex,
                workspace: None,
                cli_path: None,
            };
            app.extensions_ui.context = Some(context.clone());
            app.extensions_ui.catalogs.insert(
                context,
                Catalog {
                    entries: vec![entry],
                    ..Default::default()
                },
            );
            app.extensions_ui.tool_reports.insert(
                id.clone(),
                ToolReport {
                    names: vec!["one".into(), "two".into(), "three".into()],
                },
            );
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("mcp-tool-count-0").is_some());
    assert!(cx.debug_bounds("extension-source-0").is_none());
    assert_eq!(
        app.read_with(cx, |app, _| app.extensions_ui.tool_reports[&id].count()),
        3
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.extensions_ui.invalidate();
            cx.notify();
        })
    });
    assert!(app.read_with(cx, |app, _| app.extensions_ui.tool_reports.is_empty()));
}
