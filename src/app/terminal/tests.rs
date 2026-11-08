use super::{RunState, Runtime, ShellProfile};
use gpui_kit::{Focusable as _, TestAppContext, VisualTestContext};
use std::time::{Duration, Instant};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.run_until_parked();
}

fn wait(runtime: &Runtime, active: bool) {
    let start = Instant::now();
    while if active {
        runtime.state() != RunState::Running
    } else {
        runtime.active()
    } {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "{:?}",
            runtime.state()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[gpui_kit::test]
fn terminal_workspace_tabs_hide_and_cleanup(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "terminal-workspaces");
    std::fs::create_dir_all(root.join("other")).unwrap();
    let first = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.new_terminal(ShellProfile::Cmd, window, cx);
            assert_eq!(app.terminals.sessions[0].workspace.path(), root);
            app.terminals.sessions[0].runtime.clone()
        })
    });
    wait(&first, true);
    draw(cx);
    assert!(cx.debug_bounds("terminal-grid").is_some());
    cx.update(|window, cx| app.update(cx, |app, cx| app.hide_terminal(window, cx)));
    draw(cx);
    assert!(cx.debug_bounds("terminal-grid").is_none());
    assert!(first.active());
    let second = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.selected_workspace = Some(1);
            app.selected_session = Some(2);
            app.new_terminal(ShellProfile::Cmd, window, cx);
            let second = app.terminals.sessions[1].runtime.clone();
            assert_eq!(
                app.terminals.sessions[1].workspace.path(),
                root.join("other")
            );
            assert!(app.workspace_busy(0));
            assert!(app.workspace_busy(1));
            app.remove_workspace(0, cx);
            assert_eq!(app.store.workspaces.len(), 2);
            second
        })
    });
    wait(&second, true);
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.poll_terminals(window, cx);
            assert!(!app.terminals.sessions[1].view.read(cx).input_enabled);
            assert!(app.terminals.sessions[0].view.read(cx).input_enabled);
            app.terminals.stop_all();
        })
    });
    wait(&first, false);
    wait(&second, false);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.workspace_busy(0));
            app.remove_workspace(0, cx);
            assert_eq!(app.terminals.sessions.len(), 1);
            assert_eq!(
                app.terminals.sessions[0].workspace.path(),
                root.join("other")
            );
        })
    });
}

#[gpui_kit::test]
fn terminal_keys_stay_out_of_chat_and_return_focus(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "terminal-keys");
    let runtime = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.input.update(cx, |input, cx| {
                input.set_value("保留的聊天草稿", window, cx)
            });
            app.input.read(cx).focus_handle(cx).focus(window, cx);
            app.new_terminal(ShellProfile::Cmd, window, cx);
            app.terminals.sessions[0].runtime.clone()
        })
    });
    wait(&runtime, true);
    draw(cx);
    for key in [
        "ctrl-e", "ctrl-p", "ctrl-k", "ctrl-m", "ctrl-,", "ctrl-w", "tab", "escape",
    ] {
        cx.simulate_keystrokes(key);
        draw(cx);
        cx.update(|window, cx| {
            let app = app.read(cx);
            assert!(!app.settings_ui.open, "{key}");
            assert!(app.terminal_focused(window, cx), "{key}");
            assert_eq!(app.input.read(cx).value().as_str(), "保留的聊天草稿");
            assert!(app.session_control.runs.is_empty());
        });
    }
    cx.simulate_keystrokes("ctrl-shift-f6");
    draw(cx);
    cx.update(|window, cx| {
        assert!(
            app.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        )
    });
    runtime.stop();
    wait(&runtime, false);
}
