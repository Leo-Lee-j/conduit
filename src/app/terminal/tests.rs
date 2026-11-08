use super::{RunState, Runtime, ShellProfile};
use gpui_kit::{
    Focusable as _, Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size,
};
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

#[gpui_kit::test]
fn terminal_tab_close_requires_second_click_and_small_layout_fits(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "terminal-close");
    let (id, runtime, current_id, current) = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.new_terminal(ShellProfile::Cmd, window, cx);
            let first = &app.terminals.sessions[0];
            let (id, runtime) = (first.id, first.runtime.clone());
            app.new_terminal(ShellProfile::Cmd, window, cx);
            let current = &app.terminals.sessions[1];
            (id, runtime, current.id, current.runtime.clone())
        })
    });
    wait(&runtime, true);
    wait(&current, true);
    for (width, height) in [(900., 480.), (1200., 800.)] {
        cx.simulate_resize(size(px(width), px(height)));
        draw(cx);
        let panel = cx.debug_bounds("terminal-panel").unwrap();
        let grid = cx.debug_bounds("terminal-grid").unwrap();
        assert!(grid.size.height > px(40.), "{grid:?}");
        assert!(grid.bottom() <= panel.bottom() && panel.bottom() <= px(height));
        assert!(cx.debug_bounds("terminal-copy-visible").is_none());
        assert!(cx.debug_bounds("terminal-close").is_none());
        assert!(cx.debug_bounds("toggle-terminal").unwrap().size.width <= px(32.));
        let tab = cx.debug_bounds("terminal-tab-1").unwrap();
        let close = cx.debug_bounds("terminal-tab-close-1").unwrap();
        assert!(tab.right() - close.right() >= px(4.));
    }
    let grid = cx.debug_bounds("terminal-grid").unwrap();
    let close = cx.debug_bounds("terminal-tab-close-1").unwrap();
    cx.simulate_mouse_down(close.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(close.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(runtime.active());
        assert_eq!(app.terminals.confirm_close, Some(id));
        assert_eq!(
            app.terminals.selected.get(&app.terminal_key().unwrap()),
            Some(&current_id)
        );
    });
    assert!(cx.debug_bounds("terminal-confirm-close").is_none());
    assert!(cx.debug_bounds("terminal-cancel-close").is_none());
    assert_eq!(cx.debug_bounds("terminal-grid").unwrap(), grid);
    assert_eq!(cx.debug_bounds("terminal-tab-close-1").unwrap(), close);
    // Clicking the current tab cancels the first click without stopping either shell.
    let current_tab = cx.debug_bounds("terminal-tab-2").unwrap();
    cx.simulate_mouse_down(
        current_tab.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        current_tab.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    draw(cx);
    cx.update(|_, cx| assert_eq!(app.read(cx).terminals.confirm_close, None));
    assert!(runtime.active() && current.active());
    cx.simulate_mouse_down(close.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(close.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|_, cx| assert_eq!(app.read(cx).terminals.confirm_close, Some(id)));
    assert!(runtime.active());
    let confirm = cx.debug_bounds("terminal-tab-close-1").unwrap();
    cx.simulate_mouse_down(confirm.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(confirm.center(), MouseButton::Left, Modifiers::default());
    wait(&runtime, false);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.poll_terminals(window, cx);
            assert_eq!(app.terminals.sessions.len(), 1);
            assert_eq!(app.terminals.sessions[0].id, current_id);
            assert!(current.active());
        })
    });
    draw(cx);
    current.stop();
    wait(&current, false);
    cx.update(|window, cx| app.update(cx, |app, cx| app.poll_terminals(window, cx)));
    draw(cx);
    let close = cx.debug_bounds("terminal-tab-close-2").unwrap();
    cx.simulate_mouse_down(close.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(close.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            assert!(app.terminals.sessions.is_empty());
        })
    });
    assert!(cx.debug_bounds("toggle-terminal").is_some());
}

#[gpui_kit::test]
fn terminal_capacity_host_command_and_exit_guards(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use conduit::plugins::builtins;
    use gpui_kit::component::WindowExt;
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "terminal-guards");
    std::fs::create_dir_all(root.join("other")).unwrap();
    let runtimes = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.sync_plugin_context();
            let stale = app.plugins.host_command(builtins::NEW_TERMINAL).unwrap();
            app.selected_workspace = Some(1);
            app.selected_session = Some(2);
            assert!(!app.dispatch_plugin_command(&stale, window, cx));
            assert!(app.terminals.sessions.is_empty());
            app.store.ui.terminal_profile = ShellProfile::Cmd;
            app.terminal_command(true, window, cx);
            for _ in 0..4 {
                app.new_terminal(ShellProfile::Cmd, window, cx);
            }
            assert_eq!(app.terminals.sessions.len(), 4);
            assert!(app.terminals.feedback.as_ref().unwrap().contains("最多 4"));
            assert!(!app.request_close(window, cx));
            assert!(!app.closing);
            assert!(app.terminals.active());
            window.close_dialog(cx);
            let runtimes = app
                .terminals
                .sessions
                .iter()
                .map(|s| s.runtime.clone())
                .collect::<Vec<_>>();
            app.terminals.stop_all();
            runtimes
        })
    });
    for runtime in runtimes {
        wait(&runtime, false);
    }
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let id = app.terminals.sessions[0].id;
            app.writable = false;
            app.restart_terminal(id, window, cx);
            assert_eq!(app.terminals.sessions.len(), 4);
            assert_eq!(app.terminals.sessions[0].id, id);
            assert!(!app.terminals.active());
        })
    });
}
