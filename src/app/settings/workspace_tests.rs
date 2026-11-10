use super::{ConduitApp, Store};
use gpui_kit::{
    AppContext as _, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size,
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
        let bounds = cx.debug_bounds("settings-content").unwrap();
        for selector in ["plugin-install", "plugin-refresh", "plugin-default-icons"] {
            let control = cx.debug_bounds(selector).unwrap();
            assert!(control.left() >= bounds.left() && control.right() <= bounds.right());
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
    let select = cx
        .debug_bounds("plugin-use-example.outline-icons")
        .unwrap()
        .center();
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
