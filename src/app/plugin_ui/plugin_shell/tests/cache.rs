//! Cache behavior through the actual desktop ShellRoot, in dev and release.
use super::*;

fn metrics(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) -> (u64, u64) {
    let runtime = runtime(app, cx).upgrade().unwrap();
    let metrics = runtime.read_metrics();
    (metrics.script_renders(), metrics.materializations())
}

#[gpui_kit::test]
fn desktop_plugin_cache_reuses_host_input_frames(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    command(&app, "example.workspace-overview.increment", cx);
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
    });
    cx.simulate_input("warmup");
    draw(cx);
    let before = metrics(&app, cx);
    assert!(before.0 > 0 && before.1 > 0, "plugin must first render");
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("G5 中文😀 input A");
    cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
    draw(cx);
    let after = metrics(&app, cx);
    assert_eq!(
        after, before,
        "host-only input/notify rematerialized the plugin"
    );
    assert!(text(&app, true, cx).contains("Command count: 1"));
    cx.update(|window, cx| {
        let input = app.read(cx).input.read(cx);
        assert_eq!(input.value().as_str(), "G5 中文😀 input A");
        assert!(input.focus_handle(cx).is_focused(window));
    });
    // Script invalidation still rebuilds exactly once after cached input frames.
    command(&app, "example.workspace-overview.increment", cx);
    let changed = metrics(&app, cx);
    assert_eq!(changed.0, after.0 + 1);
    assert!(changed.1 > after.1);
    assert!(text(&app, true, cx).contains("Command count: 2"));
    draw(cx);
    assert_eq!(metrics(&app, cx), changed);
}

#[gpui_kit::test]
fn desktop_plugin_cache_invalidates_for_metadata_theme_and_bounds(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    command(&app, "example.workspace-overview.increment", cx);
    draw(cx);
    let before = metrics(&app, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut session = crate::app::tests::session();
            session.id = 77;
            app.store.workspaces[0].sessions.push(session);
            app.refresh_shell_context(cx);
            cx.notify();
        });
    });
    draw(cx);
    let updated = metrics(&app, cx);
    assert_eq!(updated.0, before.0 + 1);
    assert!(updated.1 > before.1);
    assert!(text(&app, true, cx).contains("Command count: 1"));
    for (mode, label) in [
        (crate::store::AppearanceMode::Light, "light"),
        (crate::store::AppearanceMode::Dark, "dark"),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.store.ui.appearance = mode;
                app.apply_appearance(window, cx);
            });
        });
        draw(cx);
        let snapshot = text(&app, true, cx);
        assert!(snapshot.contains(&format!("Theme: {label}")), "{snapshot}");
        assert!(snapshot.contains("Command count: 1"));
    }
    let before = metrics(&app, cx);
    cx.simulate_resize(size(px(1400.), px(1000.)));
    draw(cx);
    let resized = metrics(&app, cx);
    assert_eq!(
        resized.0, before.0,
        "resize should reuse the script snapshot"
    );
    assert!(resized.1 > before.1, "new bounds must relayout the plugin");
    draw(cx);
    assert_eq!(metrics(&app, cx), resized);
}

#[gpui_kit::test]
fn desktop_plugin_cache_keeps_native_input_interactive(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    command(&app, "example.workspace-overview.increment", cx);
    let group = cx.element_bounds("plugin-summary-input").unwrap();
    assert!(
        group.size.width > px(100.),
        "cached script root must fill its container: {group:?}"
    );
    click(cx, "plugin-summary-input");
    cx.simulate_input("中文😀 cache");
    draw(cx);
    assert!(text(&app, true, cx).contains("Input: 中文😀 cache"));
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("替换🦀");
    draw(cx);
    assert!(text(&app, true, cx).contains("Input: 替换🦀"));
}
