use crate::app::{ConduitApp, shell::TimelineSource};
use crate::domain::{ChatItem, Workspace};
use crate::store::Store;
use gpui_kit::component::h_flex;
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ListState, Modifiers, MouseButton, ParentElement,
    Render, Styled, Subscription, TestAppContext, VisualTestContext, Window, div, px, size,
};

struct OutlineHarness {
    app: Entity<ConduitApp>,
    reference: TimelineSource,
    _subscription: Subscription,
}

impl Render for OutlineHarness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app.update(cx, |app, cx| {
            TimelineSource::Main.refresh_outline(app);
            self.reference.refresh_outline(app);
            h_flex()
                .size_full()
                .child(app.message_outline(TimelineSource::Main, cx))
                .child(div().w(px(100.)))
                .child(app.message_outline(self.reference.clone(), cx))
        })
    }
}

fn item(key: &str, kind: &str) -> ChatItem {
    serde_json::from_value(serde_json::json!({
        "key": key, "kind": kind, "text": format!("Message {key}")
    }))
    .unwrap()
}

fn fixture(
    cx: &mut TestAppContext,
) -> (Entity<ConduitApp>, TimelineSource, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut first = crate::app::tests::session();
            first.messages = vec![
                item("first", "User"),
                item("reply", "Assistant"),
                item("second", "User"),
            ];
            let mut second = first.clone();
            second.id = 2;
            second.messages.insert(2, item("tool-a", "Tool"));
            second.messages.insert(3, item("tool-b", "Tool"));
            let store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir().join("conduit-outline-sources"),
                    sessions: vec![first, second],
                }],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app.startup = None;
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.sync_chat_rows();
            app
        });
        let reference = TimelineSource::session(
            2,
            ListState::new(0, gpui_kit::ListAlignment::Bottom, px(240.)),
        );
        app.update(cx, |app, _| reference.refresh(app));
        *output.borrow_mut() = Some((app.clone(), reference.clone()));
        let harness = cx.new(|cx| OutlineHarness {
            _subscription: cx.observe(&app, |_, _, cx| cx.notify()),
            app,
            reference,
        });
        gpui_kit::component::Root::new(harness, window, cx)
    });
    let (app, reference) = slot.borrow_mut().take().unwrap();
    cx.simulate_resize(size(px(600.), px(400.)));
    (app, reference, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        let _ = window.draw(cx);
    });
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    draw(cx);
    let center = cx.debug_bounds(selector).unwrap().center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn shared_outline_clicks_navigate_only_their_source(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, reference, cx) = fixture(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| app.jump_to_message(1, "first", cx));
    });
    click(cx, "outline-marker-2-1");
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.selected_session, Some(1));
        assert_eq!(app.scroll.logical_scroll_top().item_ix, 0);
        assert_eq!(app.pending_message_jump, Some(0));
        assert_eq!(app.outline.active, Some(0));
        assert_eq!(app.message_focus.as_ref().map(|(id, _, _)| *id), Some(1));
        let TimelineSource::Session(state) = &reference else {
            unreachable!()
        };
        let state = state.borrow();
        assert_eq!(state.pending_message_jump, Some(4));
        assert_eq!(
            state.scroll.logical_scroll_top().item_ix,
            state.rows.row(4).unwrap()
        );
        assert_eq!(state.outline.active, Some(1));
        assert!(!state.at_bottom);
    });
    click(cx, "outline-marker-1-1");
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.pending_message_jump, Some(2));
        assert_eq!(app.outline.active, Some(1));
        reference.with_outline(app, |outline| assert_eq!(outline.active, Some(1)));
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            reference.jump_to_end(app, cx);
            assert_eq!(app.pending_message_jump, Some(2));
            assert!(!app.at_bottom);
            let TimelineSource::Session(state) = &reference else {
                unreachable!()
            };
            let state = state.borrow();
            assert!(state.at_bottom);
            assert!(state.pending_message_jump.is_none());
        });
    });
}

#[gpui_kit::test]
fn reference_outline_refresh_and_stale_clicks_preserve_main_navigation(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, reference, cx) = fixture(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            TimelineSource::Main.refresh_outline(app);
            app.jump_to_message(1, "second", cx);
            reference.jump_to_outline_message(2, "first", app, cx);
            let TimelineSource::Session(state) = &reference else {
                unreachable!()
            };
            let before = state.borrow().scroll.logical_scroll_top();
            for (id, key) in [(1, "second"), (2, "missing"), (2, "tool-a")] {
                reference.jump_to_outline_message(id, key, app, cx);
            }
            TimelineSource::Main.jump_to_outline_message(2, "first", app, cx);
            assert_eq!(
                state.borrow().scroll.logical_scroll_top().item_ix,
                before.item_ix
            );
            assert_eq!(app.pending_message_jump, Some(2));
            assert_eq!(
                app.scroll.logical_scroll_top().item_ix,
                app.chat_rows.row(2).unwrap()
            );

            app.session_mut(2)
                .unwrap()
                .messages
                .push(item("third", "User"));
            reference.refresh(app);
            reference.refresh_outline(app);
            reference.jump_to_outline_message(2, "third", app, cx);
            reference.with_outline(app, |outline| {
                assert_eq!(outline.entries.len(), 3);
                assert_eq!(outline.active, Some(2));
            });
            app.session_mut(2).unwrap().messages.truncate(2);
            reference.refresh(app);
            reference.refresh_outline(app);
            reference.refresh_active(app, cx);
            reference.with_outline(app, |outline| {
                assert_eq!(outline.entries.len(), 1);
                assert_eq!(outline.active, Some(0));
            });
            assert_eq!(app.outline.entries.len(), 2);
            assert_eq!(app.pending_message_jump, Some(2));
            assert_eq!(app.selected_session, Some(1));
        });
    });
}
