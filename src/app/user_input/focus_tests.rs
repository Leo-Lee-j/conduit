use crate::domain::Provider;
use gpui_kit::{Focusable, Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        let _ = window.draw(cx);
    });
}

#[gpui_kit::test]
fn reference_question_submit_returns_focus_to_its_composer(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "reference-question-focus");
    cx.simulate_resize(size(px(1800.), px(1000.)));
    let mut protocol =
        crate::protocol::Protocol::new(Provider::Codex, "C:/work".into(), "test".into(), None);
    let step = protocol.accept(serde_json::json!({
        "id": "reference-question",
        "method": "item/tool/requestUserInput",
        "params": {"questions": [{
            "id": "answer", "header": "Name", "question": "Project name?",
            "isOther": true, "options": null
        }]}
    }));
    let sender = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.show_chat_beside(2, window, cx);
            app.input
                .update(cx, |input, cx| input.set_value("main draft", window, cx));
            let sender = crate::app::queue::tests::running(app, 2);
            sender
                .send(step.events.into_iter().next().unwrap())
                .unwrap();
            app.poll_streams(window, cx);
            app.session_control.runs[&2].question_drafts["reference-question"][0]
                .custom
                .update(cx, |input, cx| {
                    input.set_value("reference answer", window, cx)
                });
            app.input.read(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
            sender
        })
    });
    draw(cx);
    let center = cx.debug_bounds("cli-question-submit").unwrap().center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|window, cx| {
        let app = app.read(cx);
        assert!(
            app.session_control.runs[&2]
                .approvals
                .is_submitted("reference-question")
        );
        let view = app.reference_chat_view(2).unwrap();
        let reference = view.read(cx);
        assert!(reference.input.read(cx).focus_handle(cx).is_focused(window));
        assert!(!app.input.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(app.input.read(cx).value(), "main draft");
        assert_eq!(app.selected_session, Some(1));
    });
    drop(sender);
}
