use super::{ConduitApp, Store};
use gpui_kit::Focusable as _;
use gpui_kit::{
    AppContext as _, Entity, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, TestAppContext,
    VisualTestContext, point, px, size,
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

/// Opt-in visual evidence from the real renderer, without OS input automation.
/// cargo test --features ui-tests --bin conduit settings_workspace_native_snapshots -- --ignored --nocapture --test-threads=1
#[test]
#[ignore = "requires a real Windows desktop and GPU"]
fn settings_workspace_native_snapshots() {
    use gpui_kit::{Bounds, WindowBounds, WindowOptions};
    use std::time::Duration;
    let _locale = crate::test_support::chinese();
    gpui_kit::application().with_assets(gpui_kit::assets::AllAssets).run(|cx| {
        gpui_kit::init(cx);
        crate::app::install_theme_packs(cx);
        cx.spawn(async move |cx| {
            let mut previous = None;
            let pages = [(0, "general"), (1, "appearance"), (2, "shortcuts"),
                (3, "engines"), (8, "services"), (10, "sessions"),
                (6, "git"), (4, "skills"), (9, "mcp"), (11, "plugins"), (7, "maintenance"),
                (3, "engines-detail"), (8, "services-detail"), (1, "appearance-preview"),
                (3, "engines-advanced"), (10, "sessions-naming"), (6, "git-generation"),
                (8, "service-editor"), (8, "service-new"), (9, "mcp-editor"), (4, "skill-editor")];
            for (width, height, font_size, dark) in [(1200., 800., 16., true), (850., 500., 16., true), (1200., 800., 20., false), (640., 720., 16., true), (1600., 900., 16., true)] {
                for (page, label) in pages {
                    let handle = cx.update(|cx| cx.open_window(WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(40.), px(40.)), size(px(width), px(height))))),
                        focus: false,
                        ..Default::default()
                    }, |window, cx| {
                        let mut store = Store::default();
                        store.ui.language = serde_json::from_value(serde_json::json!("zh-CN")).unwrap();
                        store.ui.ui_font_size = font_size;
                        store.ui.performance_monitor = false;
                        store.ui.appearance = if dark { crate::store::AppearanceMode::Dark } else { crate::store::AppearanceMode::Light };
                        store.providers = serde_json::from_value(serde_json::json!([
                            {"id":"visual-local", "name":"研发模型服务", "engine":"Claude", "base_url":"https://models.example.invalid/anthropic", "api_key_env":"DEMO_KEY", "models":["research-large", "research-fast"]},
                            {"id":"visual-secondary", "name":"测试环境", "engine":"Claude", "base_url":"https://staging.example.invalid/anthropic", "api_key_env":"DEMO_KEY", "models":["evaluation-model"]}
                        ])).unwrap();
                        let app = cx.new(|cx| {
                            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                            app.startup = None;
                            app.writable = true;
                            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
                            app.connection_settings.syncing = true;
                            app.connection_settings.engine = crate::domain::Provider::Claude;
                            app.connection_settings.engine_detail_open = matches!(label, "engines-detail" | "engines-advanced");
                            app.connection_settings.path_help = label == "engines-advanced";
                            app.connection_settings.service_detail_open = label == "services-detail";
                            app.connection_settings.service_selection = Some((crate::domain::Provider::Claude, crate::providers::ModelChannel::Provider("visual-local".into())));
                            app.settings_ui.open = true;
                            app.settings_ui.page = page;
                            app.settings_ui.naming_expanded = label == "sessions-naming";
                            app.settings_ui.git_expanded = label == "git-generation";
                            if label == "sessions-naming" { app.settings_ui.group = Some(3); }
                            app.settings_ui.appearance_preview_open = label == "appearance-preview";
                            app.extensions_ui.mcp = page == 9;
                            app.extensions_ui.engine = crate::domain::Provider::Claude;
                            app.extensions_ui.initialized = true;
                            let context = crate::extensions::Context { engine: crate::domain::Provider::Claude, workspace: None, cli_path: None };
                            let entries = [
                                ("code-review", "检查代码变更、行为边界与回归风险。", crate::extensions::Kind::Skill, false),
                                ("release-notes", "从实际版本变更生成面向用户的更新说明。", crate::extensions::Kind::Skill, false),
                                ("workspace-style", "工作区代码约定与命名规范。", crate::extensions::Kind::Skill, true),
                                ("workspace-files", "", crate::extensions::Kind::Mcp, false),
                                ("documentation-search", "", crate::extensions::Kind::Mcp, false),
                                ("issue-tracker", "", crate::extensions::Kind::Mcp, true),
                            ].into_iter().map(|(name, description, kind, disabled)| crate::extensions::Entry {
                                name: name.into(), description: description.into(),
                                path: std::env::temp_dir().join("conduit-visual-fixtures").join(name).join(if kind == crate::extensions::Kind::Skill { "SKILL.md" } else { "config.json" }),
                                scope: "全局".into(), kind, disabled,
                                transport: "stdio".into(), target: "demo-server".into(),
                            }).collect::<Vec<_>>();
                            app.extensions_ui.catalogs.insert(context.clone(), crate::extensions::Catalog {
                                manual_detection: entries.iter().map(crate::extensions::Entry::id).collect(),
                                entries, ..Default::default()
                            });
                            app.extensions_ui.context = Some(context);
                            app
                        });
                        let root = cx.new(|cx| gpui_kit::component::Root::new(app.clone(), window, cx));
                        window.defer(cx, move |window, cx| {
                            match label {
                                "service-editor" | "service-new" => app.update(cx, |app, cx| {
                                    let provider = (label == "service-editor").then(|| app.store.providers[0].clone());
                                    app.open_provider_editor(provider, window, cx);
                                }),
                                "mcp-editor" => app.update(cx, |app, cx| app.open_mcp_form(window, cx)),
                                "skill-editor" => super::config_editor::show_editor(
                                    app, std::env::temp_dir().join("conduit-visual-fixtures/my-skill/SKILL.md"),
                                    None, "---\nname: my-skill\ndescription: Review changes and verify behavior.\n---\n\n# Review\n\nCheck behavior, readability, and regression risks.".into(), true, window, cx),
                                _ => {}
                            }
                        });
                        root
                    }).unwrap());
                    if let Some(old) = previous.replace(handle) {
                        old.update(cx, |_, window, _| window.remove_window()).unwrap();
                    }
                    // Capture dialog rest states, after the native entrance fade.
                    let settle_ms = if label.ends_with("editor") || label == "service-new" { 450 } else { 150 };
                    cx.background_executor().timer(Duration::from_millis(settle_ms)).await;
                    let saved = std::rc::Rc::new(std::cell::Cell::new(false));
                    let completed = saved.clone();
                    handle.update(cx, |_, window, cx| window.defer(cx, move |window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                        let image = window.render_to_image().unwrap();
                        std::fs::create_dir_all("target/settings-native-snapshots").unwrap();
                        let name = format!("{label}-{width:.0}-{font_size:.0}-{}.png", if dark { "dark" } else { "light" });
                        image.save(format!("target/settings-native-snapshots/{name}")).unwrap();
                        println!("saved {name}: {}x{}", image.width(), image.height());
                        completed.set(true);
                    })).unwrap();
                    cx.background_executor().timer(Duration::from_millis(60)).await;
                    assert!(saved.get(), "snapshot {label} was not saved");
                }
            }
            cx.update(|cx| cx.quit());
        }).detach();
    });
}

#[gpui_kit::test]
fn independent_settings_restore_workbench_state_and_preview_is_opt_in(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.simulate_resize(size(px(1600.), px(900.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.close_settings(window, cx);
            app.input.update(cx, |input, cx| {
                input.set_value("retained workbench draft", window, cx);
                input.set_selected_range(2..8, cx);
                input.focus_handle(cx).focus(window, cx);
            });
            app.plugins.dock.width = 410.;
            app.open_settings(window, cx);
            app.select_settings_page(1, window, cx);
        })
    });
    draw(cx);
    let before = app.read_with(cx, |app, _| serde_json::to_value(&app.store).unwrap());
    assert!(cx.debug_bounds("appearance-preview-rail").is_none());
    let preview = cx
        .debug_bounds("appearance-toggle-preview")
        .unwrap()
        .center();
    cx.simulate_mouse_down(preview, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(preview, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("appearance-preview-rail").is_some());
    for width in [640., 1600.] {
        cx.simulate_resize(size(px(width), px(900.)));
        draw(cx);
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.plugins.dock.width, 410.,
                "hidden dock retains its width"
            );
            assert_eq!(serde_json::to_value(&app.store).unwrap(), before);
        });
    }
    let back = cx
        .debug_bounds("settings-back-to-workbench")
        .unwrap()
        .center();
    cx.simulate_mouse_down(back, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(back, MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|window, cx| {
        app.read_with(cx, |app, cx| {
            assert!(!app.settings_ui.open);
            assert_eq!(app.input.read(cx).value(), "retained workbench draft");
            assert_eq!(app.input.read(cx).selected_range(), 2..8);
            assert!(app.input.read(cx).focus_handle(cx).is_focused(window));
        })
    });
}

#[gpui_kit::test]
fn appearance_filter_retains_input_across_reflow_and_escape_clears_before_leaving(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui = crate::store::UiSettings::default();
            app.store.ui.code_line_numbers = true;
            app.select_settings_page(1, window, cx);
            app.settings_ui.appearance_filter.update(cx, |input, cx| {
                input.set_value("code", window, cx);
            });
            app.settings_ui
                .search
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        });
    });
    let input = app.read_with(cx, |app, _| app.settings_ui.appearance_filter.entity_id());
    for width in [1600., 640., 1600.] {
        cx.simulate_resize(size(px(width), px(900.)));
        draw(cx);
        assert!(
            cx.debug_bounds("appearance-property-code_font_size")
                .is_some()
        );
        assert!(
            cx.debug_bounds("appearance-property-ui_font_size")
                .is_none()
        );
        cx.update(|window, cx| {
            let app = app.read(cx);
            assert_eq!(app.settings_ui.appearance_filter.entity_id(), input);
            assert!(
                app.settings_ui
                    .search
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        });
    }
    let toggle = cx
        .debug_bounds("appearance-modified-only")
        .unwrap()
        .center();
    cx.simulate_mouse_down(toggle, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(toggle, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(
        cx.debug_bounds("appearance-property-code_line_numbers")
            .is_some()
    );
    assert!(
        cx.debug_bounds("appearance-property-code_font_size")
            .is_none()
    );
    cx.simulate_keystrokes("escape");
    draw(cx);
    app.read_with(cx, |app, cx| {
        assert!(app.settings_ui.open);
        assert!(
            app.settings_ui
                .appearance_filter
                .read(cx)
                .value()
                .is_empty()
        );
        assert!(!app.settings_ui.appearance_modified_only);
        assert!(
            app.store.ui.code_line_numbers,
            "browsing read-only filters cannot change values"
        );
    });
    // A group reset covers its hidden modified properties too.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.settings_ui.appearance_filter.update(cx, |input, cx| {
                input.set_value("code_font_size", window, cx)
            });
        });
    });
    draw(cx);
    assert!(
        cx.debug_bounds("appearance-property-code_line_numbers")
            .is_none()
    );
    let reset = cx
        .debug_bounds("appearance-reset-group-3")
        .unwrap()
        .center();
    cx.simulate_mouse_down(reset, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(reset, MouseButton::Left, Modifiers::default());
    draw(cx);
    app.read_with(cx, |app, _| assert!(!app.store.ui.code_line_numbers));
}

fn settle_ai_save(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) {
    for _ in 0..200 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(20));
        draw(cx);
        if !app.read_with(cx, |app, _| app.connection_settings.saving) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    panic!("AI settings save did not complete");
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
            app.connection_settings.engine_detail_open = true;
            app.set_engine_enabled(Provider::Codex, false, cx);
            assert!(
                app.store.cli.enabled(Provider::Codex),
                "read-only mutation must be rejected"
            );
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("engine-enabled-1").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            app.store.cli.set_enabled(Provider::Codex, false);
            app.new_session_provider = Provider::Claude;
            app.select_composer_engine(Provider::Codex, cx);
            assert_eq!(app.new_session_provider, Provider::Claude);
            app.store.workspaces.push(crate::domain::Workspace {
                path: std::env::temp_dir(),
                sessions: vec![],
            });
            let before = serde_json::to_value(&app.store.workspaces).unwrap();
            let error = app
                .start_turn(
                    0,
                    None,
                    Provider::Codex,
                    "offline prompt".to_string(),
                    true,
                    cx,
                )
                .unwrap_err();
            assert!(error.contains("已停用"));
            assert_eq!(serde_json::to_value(&app.store.workspaces).unwrap(), before);
            assert!(app.session_control.runs.is_empty());
            app.writable = false;
        })
    });
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
                app.connection_settings.engine_detail_open = true;
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
                Provider::Qoder => "settings-check-updates-3",
                Provider::Gemini => "settings-check-updates-4",
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
fn settings_plugins_manage_icon_theme_lifecycle_and_readonly_state(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use conduit::plugin_packages as packages;
    let (app, cx) = fixture(cx);
    let root = crate::app::plugin_ui::management::installation_root();
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins/outline-icons");
    let id = packages::install(&source, &root).unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.refresh_installed_plugins(cx);
            app.select_settings_page(11, window, cx);
        })
    });
    draw(cx);
    cx.update(|_, cx| {
        app.read_with(cx, |app, _| {
            assert_eq!(app.plugins.packages.entries.len(), 1);
            assert!(app.plugins.packages.entries[0].registration.is_some());
            assert!(
                app.settings_search_matches("plugin")
                    .iter()
                    .any(|r| r.page == 11)
            );
        })
    });
    for (width, height) in [(850., 500.), (1280., 800.)] {
        cx.simulate_resize(size(px(width), px(height)));
        draw(cx);
        assert!(cx.debug_bounds("settings-search-bar").is_some());
        for selector in ["plugin-install", "plugin-refresh", "plugin-default-icons"] {
            assert!(cx.debug_bounds(selector).is_some());
        }
    }
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.set_plugin_enabled(&id, false, cx);
            app.select_file_icon_theme(Some(id.clone()), cx);
            assert!(app.store.ui.plugins.file_icon_theme.is_none());
            assert!(
                app.plugins.packages.entries[0].registration.is_some(),
                "read-only toggle"
            );
            app.writable = true;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            cx.notify();
        })
    });
    draw(cx);
    let viewport = cx.debug_bounds("settings-plugins-list").unwrap();
    let search = cx.debug_bounds("settings-search-bar").unwrap();
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-1200.))),
        ..Default::default()
    });
    draw(cx);
    assert_eq!(cx.debug_bounds("settings-search-bar"), Some(search));
    let select = cx
        .debug_bounds("plugin-use-example.outline-icons")
        .unwrap()
        .center();
    assert!(
        viewport.contains(&select),
        "scroll the installed plugin into view"
    );
    cx.simulate_mouse_down(select, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(select, MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(
                app.store.ui.plugins.file_icon_theme.as_deref(),
                Some(id.as_str())
            );
            let before = app.plugins.revision();
            app.set_plugin_enabled(&id, false, cx);
            assert!(app.store.ui.plugins.disabled.contains(&id));
            assert!(app.store.ui.plugins.file_icon_theme.is_none());
            assert!(app.plugins.packages.entries[0].registration.is_none());
            assert!(
                app.plugins.revision() != before,
                "icon changes invalidate navigation cache"
            );
            app.select_file_icon_theme(Some(id.clone()), cx);
            assert!(
                app.store.ui.plugins.file_icon_theme.is_none(),
                "disabled themes cannot activate"
            );
            app.refresh_installed_plugins(cx);
        })
    });
    draw(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(
                app.plugins.packages.entries[0].registration.is_none(),
                "refresh preserves disabled preference"
            );
            app.set_plugin_enabled(&id, true, cx);
            app.select_file_icon_theme(Some(id.clone()), cx);
            app.refresh_installed_plugins(cx);
        })
    });
    draw(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(
                app.store.ui.plugins.file_icon_theme.as_deref(),
                Some(id.as_str())
            );
            app.uninstall_plugin(id.clone(), cx);
        })
    });
    draw(cx);
    cx.update(|_, cx| {
        app.read_with(cx, |app, _| {
            assert!(app.plugins.packages.entries.is_empty());
            assert!(app.store.ui.plugins.file_icon_theme.is_none());
            assert!(!app.store.ui.plugins.disabled.contains(&id));
        })
    });
    assert!(!root.join(&id).exists());
    assert!(source.join(packages::MANIFEST).exists());
}

#[gpui_kit::test]
fn plugin_discovery_installs_official_package_and_respects_readonly_mode(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    let official = conduit::plugin_packages::catalog::bundled_entry();
    let id = official.plugin.id.clone();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            // Seed a completed discovery result: this UI test must never contact GitHub.
            app.plugins.packages.catalog.queried = true;
            app.select_settings_page(11, window, cx);
        })
    });
    draw(cx);
    let discover = cx.debug_bounds("plugins-discover-tab").unwrap().center();
    cx.simulate_mouse_down(discover, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(discover, MouseButton::Left, Modifiers::default());
    draw(cx);
    for (width, height) in [(850., 500.), (1280., 800.)] {
        cx.simulate_resize(size(px(width), px(height)));
        draw(cx);
        let content = cx.debug_bounds("settings-content").unwrap();
        let search = cx.debug_bounds("plugin-catalog-search").unwrap();
        assert!(search.left() >= content.left() && search.right() <= content.right());
    }
    let button = cx
        .debug_bounds("catalog-install-conduit-plugins.outline-icons")
        .unwrap()
        .center();
    cx.simulate_mouse_down(button, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(button, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(app.read_with(cx, |app, _| app.plugins.packages.entries.is_empty()));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            cx.notify();
        })
    });
    draw(cx);
    cx.simulate_mouse_down(button, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(button, MouseButton::Left, Modifiers::default());
    draw(cx);
    app.read_with(cx, |app, _| {
        assert_eq!(app.plugins.packages.entries.len(), 1);
        let package = &app.plugins.packages.entries[0].package;
        assert_eq!(package.manifest.id, id);
        assert!(matches!(
            package.source,
            Some(conduit::plugin_packages::catalog::Source::Bundled { .. })
        ));
        assert!(app.plugins.packages.entries[0].registration.is_some());
    });
    // The installed tab uses the same manager and persisted icon preference.
    let installed = cx.debug_bounds("plugins-installed-tab").unwrap().center();
    cx.simulate_mouse_down(installed, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(installed, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(
        cx.debug_bounds("plugin-conduit-plugins.outline-icons")
            .is_some()
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.select_file_icon_theme(Some(id.clone()), cx);
            assert_eq!(
                app.store.ui.plugins.file_icon_theme.as_deref(),
                Some(id.as_str())
            );
            app.uninstall_plugin(id.clone(), cx);
        })
    });
    draw(cx);
    assert!(
        app.read_with(cx, |app, _| app.plugins.packages.entries.is_empty()
            && app.store.ui.plugins.file_icon_theme.is_none())
    );
}

#[gpui_kit::test]
fn settings_service_actions_keep_history_and_revalidate_routes(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{
        domain::Provider,
        providers::{ModelChannel, ModelProvider, ProviderSource},
    };
    let (app, cx) = fixture(cx);
    cx.simulate_resize(size(px(1500.), px(850.)));
    let provider = ModelProvider {
        id: "ui-service".into(),
        name: "UI service".into(),
        engine: Provider::Codex,
        base_url: "https://example.invalid/v1".into(),
        api_key_env: String::new(),
        protected_api_key: None,
        models: vec!["model-a".into(), "model-b".into()],
        source: ProviderSource::Local,
        model_metadata: vec![],
        claude_models: None,
    };
    let route = ModelChannel::Provider(provider.id.clone());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.connection_settings.engine = Provider::Codex;
            app.store.providers = vec![provider.clone()];
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.select_settings_page(8, window, cx);
            assert!(!app.set_service_default(Provider::Codex, route.clone(), None, window, cx));
            assert!(!app.delete_model_service(&provider.id, cx));
            app.writable = true;
            let mut session = crate::app::tests::session();
            session.provider = Provider::Codex;
            session.title = "Existing session".into();
            session.channel = route.clone();
            session.model = Some("model-b".into());
            app.store.workspaces.push(crate::domain::Workspace {
                path: std::env::temp_dir(),
                sessions: vec![session],
            });
            cx.notify();
        })
    });
    draw(cx);
    let row = cx
        .debug_bounds("service-row-0")
        .expect("service row")
        .center();
    cx.simulate_mouse_down(row, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(row, MouseButton::Left, Modifiers::default());
    draw(cx);
    let control = cx
        .debug_bounds("service-detail-default")
        .expect("explicit new-session default action")
        .center();
    cx.simulate_mouse_down(control, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(control, MouseButton::Left, Modifiers::default());
    draw(cx);
    settle_ai_save(&app, cx);
    cx.update(|_, cx| {
        app.read_with(cx, |app, _| {
            assert_eq!(app.store.session_defaults[&Provider::Codex].channel, route);
            assert!(app.store.session_defaults[&Provider::Codex].model.is_none());
            assert_eq!(
                app.store.workspaces[0].sessions[0].model.as_deref(),
                Some("model-b")
            );
            assert_eq!(app.provider_configuration_references(&provider.id).len(), 1);
            assert_eq!(app.provider_references(&provider.id).len(), 2);
        })
    });
    assert!(cx.debug_bounds("service-details").is_some());
    cx.update(|window, cx| {
        use gpui_kit::component::WindowExt as _;
        assert!(!window.has_active_sheet(cx), "service details are inline");
        app.update(cx, |app, cx| {
            let history = serde_json::to_value(&app.store.workspaces).unwrap();
            assert!(!app.set_service_default(Provider::Claude, route.clone(), None, window, cx));
            assert!(!app.set_service_default(
                Provider::Codex,
                ModelChannel::Provider("missing".into()),
                None,
                window,
                cx
            ));
            assert!(!app.set_service_default(
                Provider::Codex,
                route.clone(),
                Some("missing-model".into()),
                window,
                cx
            ));
            assert!(app.set_service_default(
                Provider::Codex,
                route.clone(),
                Some("model-a".into()),
                window,
                cx
            ));
            assert_eq!(
                serde_json::to_value(&app.store.workspaces).unwrap(),
                history
            );
            assert!(!app.delete_model_service(&provider.id, cx));
            assert!(
                !app.set_service_default(Provider::Codex, ModelChannel::Cli, None, window, cx),
                "overlapping saves are blocked"
            );
        })
    });
    settle_ai_save(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.set_service_default(Provider::Codex, ModelChannel::Cli, None, window, cx));
        })
    });
    settle_ai_save(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(
                !app.delete_model_service(&provider.id, cx),
                "history still references the service"
            );
            app.store.workspaces.clear();
            app.store.git.generation_channel = Some(route.clone());
            assert!(
                !app.delete_model_service(&provider.id, cx),
                "automation references block deletion"
            );
            app.store.git.generation_channel = None;
            app.store.providers[0].source = ProviderSource::Ccs {
                id: "external".into(),
                protected_route: None,
            };
            assert!(
                !app.delete_model_service(&provider.id, cx),
                "imported services remain externally managed"
            );
            app.store.providers[0].source = ProviderSource::Local;
            app.writable = false;
            assert!(!app.set_service_default(Provider::Codex, route.clone(), None, window, cx));
            assert!(!app.delete_model_service(&provider.id, cx));
            app.writable = true;
            assert!(app.delete_model_service(&provider.id, cx));
            assert!(
                !app.set_service_default(Provider::Codex, route.clone(), None, window, cx),
                "stale controls cannot restore a removed route"
            );
            app.persistence.flush();
            app.writable = false;
        });
    });
    draw(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.connection_settings.service_detail_open = false;
            cx.notify();
        })
    });
    draw(cx);
    let control = cx
        .debug_bounds("services-default-catalog")
        .unwrap()
        .center();
    cx.simulate_mouse_down(control, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(control, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(
        cx.debug_bounds("default-channel-1").is_some(),
        "default controls remain reachable"
    );
}

#[gpui_kit::test]
fn settings_services_inline_selection_resizes_without_changing_defaults(cx: &mut TestAppContext) {
    use crate::domain::Provider;
    use crate::providers::{ModelChannel, ModelProvider, ProviderSource};
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    let first: ModelProvider = serde_json::from_value(serde_json::json!({
        "id":"inline-local", "name":"Local service", "engine":"Codex",
        "base_url":"https://example.invalid/v1", "api_key_env":"",
        "models":["model-a", "model-b"]
    }))
    .unwrap();
    let mut second = first.clone();
    second.id = "inline-imported".into();
    second.name = "Imported service".into();
    second.source = ProviderSource::Ccs {
        id: "external".into(),
        protected_route: None,
    };
    let route = ModelChannel::Provider(first.id.clone());
    cx.simulate_resize(size(px(1500.), px(850.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.connection_settings.engine = Provider::Codex;
            app.store.providers = vec![first.clone(), second.clone()];
            app.writable = true;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            assert!(app.set_service_default(
                Provider::Codex,
                route.clone(),
                Some("model-a".into()),
                window,
                cx
            ));
            app.select_settings_page(8, window, cx);
        })
    });
    draw(cx);
    settle_ai_save(&app, cx);
    let list = cx.debug_bounds("service-list").expect("inline list");
    let detail = cx.debug_bounds("service-details").expect("inline detail");
    assert!(
        list.right() <= detail.left(),
        "details are beside, not over the list"
    );
    assert!(cx.debug_bounds("service-sessions-section").is_some());
    assert!(cx.debug_bounds("service-detail-back").is_none());
    assert!(cx.debug_bounds("service-default-summary").is_some());
    assert!(cx.debug_bounds("service-models-heading").is_some());
    assert!(cx.debug_bounds("service-sessions-heading").is_some());
    let before = cx.update(|_, cx| {
        app.read_with(cx, |app, _| {
            serde_json::to_value(&app.store.session_defaults).unwrap()
        })
    });
    let click = |cx: &mut VisualTestContext, selector: &'static str| {
        super::redesign_tests::open_engine_picker_if_needed(cx, selector);
        let center = cx.debug_bounds(selector).expect(selector).center();
        cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
        draw(cx);
    };
    click(cx, "service-row-1");
    cx.update(|_, cx| {
        app.read_with(cx, |app, _| {
            assert_eq!(
                serde_json::to_value(&app.store.session_defaults).unwrap(),
                before
            );
            assert_eq!(
                app.connection_settings
                    .service_selection
                    .as_ref()
                    .unwrap()
                    .1,
                ModelChannel::Provider(second.id.clone())
            );
        })
    });
    assert!(
        cx.debug_bounds("service-edit-local").is_none(),
        "imported services stay read-only"
    );
    cx.simulate_resize(size(px(850.), px(700.)));
    draw(cx);
    assert!(cx.debug_bounds("service-list").is_none());
    assert!(cx.debug_bounds("service-detail-back").is_some());
    click(cx, "service-detail-back");
    assert!(cx.debug_bounds("service-list").is_some());
    assert!(cx.debug_bounds("service-details").is_none());
    click(cx, "service-row-0");
    assert!(cx.debug_bounds("service-edit-local").is_some());
    click(cx, "service-model-default-1");
    settle_ai_save(&app, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.store.session_defaults[&Provider::Codex].channel, route);
            assert_eq!(
                app.store.session_defaults[&Provider::Codex]
                    .model
                    .as_deref(),
                Some("model-b")
            );
            app.writable = false;
            cx.notify();
        })
    });
    draw(cx);
    click(cx, "service-model-default-0");
    cx.update(|_, cx| {
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.store.session_defaults[&Provider::Codex]
                    .model
                    .as_deref(),
                Some("model-b")
            )
        })
    });
    // Esc backs out of the inline detail before leaving the settings workspace.
    cx.update(|window, cx| app.update(cx, |app, cx| app.settings_ui.focus.focus(window, cx)));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(cx.debug_bounds("service-list").is_some());
    cx.update(|_, cx| app.read_with(cx, |app, _| assert!(app.settings_ui.open)));
    // Engine changes must not leave another engine's service in the detail pane.
    click(cx, "settings-engine-0");
    cx.update(|_, cx| {
        app.read_with(cx, |app, _| {
            assert!(app.connection_settings.service_selection.is_none());
            assert!(!app.connection_settings.service_detail_open);
        })
    });
    assert!(cx.debug_bounds("service-list").is_some());
    assert!(cx.debug_bounds("service-details").is_none());
}

fn ai_service(id: &str) -> crate::providers::ModelProvider {
    crate::providers::ModelProvider {
        id: id.into(),
        name: id.into(),
        engine: crate::domain::Provider::Codex,
        base_url: "https://example.invalid/v1".into(),
        api_key_env: String::new(),
        protected_api_key: None,
        models: vec!["exact-model".into(), "other-model".into()],
        source: Default::default(),
        model_metadata: vec![],
        claude_models: None,
    }
}

#[gpui_kit::test]
fn settings_reselecting_default_service_preserves_model_and_does_not_write(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use crate::{domain::Provider, providers::ModelChannel, store::SessionDefaults};
    let (app, cx) = fixture(cx);
    let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let recorded = writes.clone();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            app.persistence = crate::persistence::Persistence::with_writer(move |_| {
                recorded.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            });
            app.store.providers.push(ai_service("retain-default"));
            let channel = ModelChannel::Provider("retain-default".into());
            app.store.session_defaults.insert(
                Provider::Codex,
                SessionDefaults {
                    channel: channel.clone(),
                    model: Some("exact-model".into()),
                },
            );
            assert!(!app.set_default_channel(Provider::Codex, channel, window, cx));
            assert_eq!(
                app.store.session_defaults[&Provider::Codex]
                    .model
                    .as_deref(),
                Some("exact-model")
            );
            assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 0);
            assert!(app.set_default_channel(Provider::Codex, ModelChannel::Cli, window, cx));
            assert!(app.connection_settings.saving);
        })
    });
    settle_ai_save(&app, cx);
    app.read_with(cx, |app, _| {
        let default = &app.store.session_defaults[&Provider::Codex];
        assert_eq!(default.channel, ModelChannel::Cli);
        assert!(default.model.is_none());
    });
    assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[gpui_kit::test]
fn settings_failed_default_save_restores_prior_selection_and_allows_retry(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{domain::Provider, providers::ModelChannel, store::SessionDefaults};
    let (app, cx) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            app.store.providers.push(ai_service("failed-default"));
            app.store.session_defaults.insert(
                Provider::Codex,
                SessionDefaults {
                    channel: ModelChannel::Cli,
                    model: Some("original-model".into()),
                },
            );
            app.persistence = crate::persistence::Persistence::with_writer(|_| {
                Err("synthetic disk failure".into())
            });
            assert!(app.set_service_default(
                Provider::Codex,
                ModelChannel::Provider("failed-default".into()),
                Some("exact-model".into()),
                window,
                cx
            ));
            assert!(app.connection_settings.saving);
            // Independent settings must survive the rollback.
            app.store.ui.default_session_permission = crate::domain::PermissionLevel::ReadOnly;
        })
    });
    settle_ai_save(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(
                app.store.session_defaults[&Provider::Codex].channel,
                ModelChannel::Cli
            );
            assert_eq!(
                app.store.session_defaults[&Provider::Codex]
                    .model
                    .as_deref(),
                Some("original-model")
            );
            assert_eq!(
                app.store.ui.default_session_permission,
                crate::domain::PermissionLevel::ReadOnly
            );
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            assert!(app.set_service_default(
                Provider::Codex,
                ModelChannel::Provider("failed-default".into()),
                Some("exact-model".into()),
                window,
                cx
            ));
        })
    });
    settle_ai_save(&app, cx);
    assert_eq!(
        app.read_with(cx, |app, _| app.store.session_defaults[&Provider::Codex]
            .model
            .clone()),
        Some("exact-model".into())
    );
}

#[gpui_kit::test]
fn settings_search_opens_exact_service_and_model_with_keyboard_focus(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{domain::Provider, providers::ModelChannel};
    let (app, cx) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.providers = vec![ai_service("search-first"), ai_service("search-second")];
            let before = serde_json::to_value(&app.store.session_defaults).unwrap();
            let qualified = app.settings_search_matches("search-second exact-model");
            assert_eq!(
                qualified.len(),
                1,
                "service-qualified model search remains supported"
            );
            let matches = app.settings_search_matches("exact-model");
            let result = matches
                .into_iter()
                .find(|r| {
                    r.target.as_ref().is_some_and(|target| {
                        target.channel == ModelChannel::Provider("search-second".into())
                    })
                })
                .unwrap();
            app.open_settings_result(result, window, cx);
            assert_eq!(
                app.connection_settings.service_selection,
                Some((
                    Provider::Codex,
                    ModelChannel::Provider("search-second".into())
                ))
            );
            assert_eq!(
                app.connection_settings
                    .model_search
                    .read(cx)
                    .value()
                    .as_str(),
                "exact-model"
            );
            assert!(
                app.connection_settings
                    .model_search
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
            assert_eq!(
                serde_json::to_value(&app.store.session_defaults).unwrap(),
                before
            );
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("service-model-search").is_some());
    assert!(cx.debug_bounds("service-model-row-0").is_some());
    assert!(cx.debug_bounds("service-model-row-1").is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_ui
                .search
                .update(cx, |input, cx| input.set_value("search-first", window, cx));
            app.settings_ui
                .search
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        })
    });
    cx.simulate_keystrokes("enter");
    draw(cx);
    app.read_with(cx, |app, cx| {
        assert_eq!(
            app.connection_settings.service_selection,
            Some((
                Provider::Codex,
                ModelChannel::Provider("search-first".into())
            ))
        );
        assert_eq!(
            app.connection_settings
                .model_search
                .read(cx)
                .value()
                .as_str(),
            ""
        );
    });
}

#[gpui_kit::test]
fn settings_change_defaults_opens_new_session_controls(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| app.select_settings_page(8, window, cx)));
    draw(cx);
    let button = cx
        .debug_bounds("services-default-catalog")
        .unwrap()
        .center();
    cx.simulate_mouse_down(button, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(button, MouseButton::Left, Modifiers::default());
    draw(cx);
    app.read_with(cx, |app, _| {
        assert_eq!((app.settings_ui.page, app.settings_ui.group), (10, Some(0)))
    });
    assert!(cx.debug_bounds("management-engines").is_some());
    assert!(
        cx.debug_bounds("default-channel-0").is_some()
            || cx.debug_bounds("default-channel-1").is_some()
    );

    // Search and confirm through the real picker: the model ID is persisted for
    // new sessions, and an unavailable saved ID survives merely opening it.
    cx.simulate_resize(size(px(1200.), px(800.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.store.providers.push(ai_service("search-default"));
            app.store.session_defaults.insert(
                crate::domain::Provider::Codex,
                crate::store::SessionDefaults {
                    channel: crate::providers::ModelChannel::Provider("search-default".into()),
                    model: Some("unlisted-saved-model".into()),
                },
            );
            app.select_settings_engine(crate::domain::Provider::Codex, window, cx);
        })
    });
    draw(cx);
    let picker = cx.debug_bounds("default-model-1").unwrap().center();
    cx.simulate_mouse_down(picker, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(picker, MouseButton::Left, Modifiers::default());
    draw(cx);
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.store.session_defaults[&crate::domain::Provider::Codex]
                .model
                .as_deref(),
            Some("unlisted-saved-model")
        );
    });
    cx.simulate_input("other-model");
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    settle_ai_save(&app, cx);
    app.read_with(cx, |app, _| {
        let defaults = &app.store.session_defaults[&crate::domain::Provider::Codex];
        assert_eq!(
            defaults.channel,
            crate::providers::ModelChannel::Provider("search-default".into())
        );
        assert_eq!(defaults.model.as_deref(), Some("other-model"));
    });
}
