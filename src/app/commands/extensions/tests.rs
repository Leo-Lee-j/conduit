use super::{Catalog, Entry, ExtensionContext, HostAction, Kind, Probe, Provider, Row, show};
use crate::app::queue::tests::fixture;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{Focusable as _, TestAppContext, VisualTestContext, px, size};
use std::path::PathBuf;

fn context() -> ExtensionContext {
    ExtensionContext {
        engine: Provider::Codex,
        workspace: Some("C:/fixture/project".into()),
        cli_path: None,
    }
}

fn entry(name: &str, kind: Kind, scope: &str, disabled: bool) -> Entry {
    Entry {
        name: name.into(),
        description: "Review 中文 changes".into(),
        path: PathBuf::from(format!("C:/fixture/{scope}/{name}/SKILL.md")),
        scope: scope.into(),
        kind,
        disabled,
        transport: "stdio".into(),
        target: "fixture-executable".into(),
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn extension_dialog_filters_kind_names_descriptions_and_paths_without_hiding_errors(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (_, cx, _) = fixture(cx, "extensions-filter");
    let target = context();
    let catalog = Catalog {
        entries: vec![
            entry("review", Kind::Skill, "全局", false),
            entry("review", Kind::Skill, "项目", true),
            entry("database", Kind::Mcp, "全局", false),
        ],
        errors: vec!["fixture scan error".into()],
        ..Default::default()
    };
    let view = cx.update(|window, cx| {
        show(
            target.clone(),
            Kind::Skill,
            Probe::ready(target, catalog.clone()),
            window,
            cx,
        )
    });
    draw(cx);
    view.update(cx, |dialog, cx| {
        assert!(dialog.poll(cx));
        assert_eq!(dialog.count, 2);
        assert_eq!(dialog.rows.len(), 3);
        assert!(matches!(&dialog.rows[0], Row::Error(error) if error == "fixture scan error"));
        assert!(
            dialog
                .rows
                .iter()
                .any(|row| matches!(row, Row::Entry(entry) if entry.disabled))
        );
        assert!(
            dialog
                .rows
                .iter()
                .all(|row| !matches!(row, Row::Entry(entry) if entry.kind == Kind::Mcp))
        );
    });
    for (query, count) in [
        ("REVIEW", 2),
        ("中文", 2),
        ("项目", 1),
        ("SKILL.md", 2),
        ("missing", 0),
    ] {
        cx.update(|window, cx| {
            view.update(cx, |dialog, cx| {
                dialog
                    .search
                    .update(cx, |input, cx| input.set_value(query, window, cx));
                dialog.filter(cx);
                assert_eq!(dialog.count, count, "{query}");
                assert!(matches!(&dialog.rows[0], Row::Error(_)));
                if count == 0 {
                    assert!(matches!(
                        dialog.rows.last(),
                        Some(Row::Empty("commands.extensions.no_matches"))
                    ));
                }
            });
        });
    }
    cx.update(|window, cx| {
        view.update(cx, |dialog, cx| {
            dialog.search.update(cx, |input, cx| input.set_value("", window, cx));
            dialog.kind = Kind::Mcp;
            dialog.filter(cx);
            assert_eq!(dialog.count, 1);
            assert!(matches!(&dialog.rows[1], Row::Entry(entry) if entry.status() == conduit::i18n::text_static("extensions.configured_connection_not_tested")));
            dialog.close();
        });
        window.close_dialog(cx);
    });
}

#[gpui_kit::test]
fn extension_dialog_replaces_pending_discovery_and_rejects_wrong_target_or_closed_results(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (_, cx, _) = fixture(cx, "extensions-lifecycle");
    let target = context();
    let (probe, old_sender) = Probe::test_pending(target.clone());
    let view = cx.update(|window, cx| show(target.clone(), Kind::Mcp, probe, window, cx));
    view.update(cx, |dialog, cx| {
        assert!(!dialog.poll(cx));
        let mut other = target.clone();
        other.engine = Provider::Claude;
        dialog.load(Probe::ready(other, Catalog::default()), cx);
        assert!(
            !dialog.poll(cx),
            "a different target must not replace pending discovery"
        );
        dialog.load(Probe::ready(target.clone(), Catalog::default()), cx);
        assert!(old_sender.send(Catalog::default()).is_err());
        assert!(dialog.poll(cx));
        assert!(matches!(
            dialog.rows.as_slice(),
            [Row::Empty("commands.extensions.no_mcp")]
        ));

        let (probe, disconnected) = Probe::test_pending(target.clone());
        dialog.load(probe, cx);
        drop(disconnected);
        assert!(dialog.poll(cx));
        assert!(matches!(
            dialog.rows.as_slice(),
            [Row::Error(_), Row::Empty("commands.extensions.incomplete")]
        ));

        let (probe, closed_sender) = Probe::test_pending(target.clone());
        dialog.load(probe, cx);
        dialog.close();
        assert!(closed_sender.send(Catalog::default()).is_err());
        dialog.load(Probe::ready(target.clone(), Catalog::default()), cx);
        assert!(dialog.probe.is_none());
        assert!(dialog.task.is_none());
    });
}

#[gpui_kit::test]
fn extension_dialog_keyboard_close_cancels_discovery_and_restores_composer_focus(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "extensions-keyboard");
    let target = context();
    let (probe, sender) = Probe::test_pending(target.clone());
    let view = cx.update(|window, cx| {
        window.resize(size(px(720.), px(600.)));
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        show(target, Kind::Skill, probe, window, cx)
    });
    draw(cx);
    assert!(cx.debug_bounds("chat-extension-dialog").is_some());
    cx.update(|window, cx| {
        assert!(
            view.read(cx)
                .search
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        assert!(!app.read(cx).settings_ui.open);
    });
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    cx.simulate_keystrokes("escape");
    draw(cx);
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert!(view.read(cx).closed);
        assert!(view.read(cx).probe.is_none());
        assert!(
            app.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    });
    assert!(sender.send(Catalog::default()).is_err());
}

#[gpui_kit::test]
fn extension_context_tracks_reference_composer_and_selected_engine_without_a_session(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "extensions-composer-target");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            std::fs::create_dir_all(root.join("other")).unwrap();
            app.session_mut(2).unwrap().provider = Provider::Codex;
            app.show_chat_beside(2, window, cx);
            let input = app.reference_chat_view(2).unwrap().read(cx).input.clone();
            let binding = app.composer_binding_for_session(2, input).unwrap();
            app.with_composer(binding, cx, |app, _| {
                let command = app.chat_command_context().unwrap();
                let captured = app.chat_extension_context(&command);
                assert_eq!(captured.engine, Provider::Codex);
                assert_eq!(captured.workspace, Some(root.join("other")));
            });
            let command = app.chat_command_context().unwrap();
            let captured = app.chat_extension_context(&command);
            assert_eq!(captured.engine, Provider::Claude);
            assert_eq!(captured.workspace, Some(root.clone()));
            app.new_session_provider = Provider::OpenCode;
            let command = app.chat_command_context().unwrap();
            assert_eq!(command.session, None);
            assert_eq!(
                app.chat_extension_context(&command).engine,
                Provider::OpenCode
            );
            for action in [HostAction::Skill, HostAction::Mcp] {
                assert!(
                    app.host_chat_command_unavailable(action, &command)
                        .is_none()
                );
            }
            assert_eq!(app.selected_session, Some(1));
            assert!(!app.settings_ui.open);
        });
    });
}
