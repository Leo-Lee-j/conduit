use super::*;
use gpui_kit::{
    Bounds, ElementId, Modifiers, MouseButton, Pixels, Role,
    base::test_support::{ElementSnapshot, find},
    component::input::TextareaState,
};

fn input_snapshot(cx: &mut VisualTestContext, input: &Entity<TextareaState>) -> ElementSnapshot {
    let id = ElementId::from(("input", input.entity_id()));
    cx.update(|window, _| find(window, &[], &id).expect("rendered composer input"))
}

fn control(
    cx: &mut VisualTestContext,
    input: &Entity<TextareaState>,
    id: &'static str,
) -> ElementSnapshot {
    let editor = input_snapshot(cx, input);
    let composer = ElementId::from(("composer", input.entity_id()));
    let index = editor
        .path()
        .iter()
        .position(|ancestor| ancestor == &composer)
        .expect("input must belong to its own composer");
    // Controls have the same IDs in both columns. Resolve them beneath this
    // editor's composer ancestor, never by an ambiguous window-wide ID.
    cx.update(|window, _| find(window, &editor.path()[..=index], &id.into()).expect(id))
}

fn assert_send_enabled(cx: &mut VisualTestContext, input: &Entity<TextareaState>, enabled: bool) {
    let send = control(cx, input, "send");
    assert!(send.visible());
    assert_eq!(send.role(), Some(Role::Button));
    // Observed::prepaint records node.is_disabled().then_some(true), so
    // enabled Buttons have no disabled flag, never Some(false).
    assert_eq!(send.disabled() == Some(true), !enabled, "{send:?}");
}

fn near(actual: Pixels, expected: Pixels, description: &str) {
    assert!(
        f32::from(actual - expected).abs() <= 1.5,
        "{description}: actual {actual:?}, expected {expected:?}"
    );
}

fn contained(inner: Bounds<Pixels>, outer: Bounds<Pixels>, description: &str) {
    assert!(
        inner.size.width > px(0.)
            && inner.size.height > px(0.)
            && inner.left() >= outer.left() - px(1.)
            && inner.right() <= outer.right() + px(1.)
            && inner.top() >= outer.top() - px(1.)
            && inner.bottom() <= outer.bottom() + px(1.),
        "{description}: {inner:?} must remain inside {outer:?}"
    );
}

fn compare_composers(cx: &mut VisualTestContext, inputs: &[Entity<TextareaState>; 2]) -> Pixels {
    let main = cx.debug_bounds("content-main").expect("main pane");
    let reference = cx
        .debug_bounds("content-reference")
        .expect("reference pane");
    near(main.size.width, reference.size.width, "equal pane widths");
    // These are the actual card selectors in render_bound_composer. The main
    // composer has no `composer-area` debug selector.
    let cards = [
        cx.debug_bounds("composer").expect("main composer"),
        cx.debug_bounds("reference-composer")
            .expect("reference composer"),
    ];
    near(cards[0].size.width, cards[1].size.width, "composer width");
    near(
        cards[0].size.height,
        cards[1].size.height,
        "composer height",
    );
    let editors = [
        input_snapshot(cx, &inputs[0]),
        input_snapshot(cx, &inputs[1]),
    ];
    for (index, (pane, strip)) in [
        (main, "document-tab-strip"),
        (reference, "reference-tab-strip"),
    ]
    .into_iter()
    .enumerate()
    {
        contained(cards[index], pane, "composer within pane");
        assert!(cards[index].top() >= cx.debug_bounds(strip).unwrap().bottom());
        let editor = &editors[index];
        assert!(editor.visible());
        assert_eq!(editor.role(), Some(Role::MultilineTextInput));
        assert_ne!(editor.disabled(), Some(true));
        contained(editor.bounds(), cards[index], "editor within composer");
    }
    near(
        editors[0].bounds().size.width,
        editors[1].bounds().size.width,
        "input width",
    );
    near(
        editors[0].bounds().size.height,
        editors[1].bounds().size.height,
        "input height",
    );
    assert_eq!(editors[0].label(), editors[1].label());
    assert_eq!(
        editors[0].label(),
        Some(conduit::i18n::text_static(
            "localization.describe_a_task_type_to_reference_files"
        ))
    );
    let mut previous_right = [cards[0].left(), cards[1].left()];
    for id in [
        "chat-new-session",
        "permission-picker-trigger",
        "composer-model-trigger",
        "attach-file",
        "send",
    ] {
        let controls = [control(cx, &inputs[0], id), control(cx, &inputs[1], id)];
        for index in 0..2 {
            assert!(
                controls[index].visible(),
                "{id} must be visible in pane {index}"
            );
            let bounds = controls[index].bounds();
            contained(bounds, cards[index], id);
            assert!(
                bounds.top() >= editors[index].bounds().bottom() - px(1.),
                "{id} overlaps input"
            );
            assert!(
                bounds.left() >= previous_right[index] - px(1.),
                "{id} overlaps another action"
            );
            previous_right[index] = bounds.right();
        }
        let left = controls[0].bounds();
        let right = controls[1].bounds();
        near(left.size.width, right.size.width, id);
        near(left.size.height, right.size.height, id);
        near(
            left.left() - cards[0].left(),
            right.left() - cards[1].left(),
            id,
        );
        near(
            left.top() - cards[0].top(),
            right.top() - cards[1].top(),
            id,
        );
        assert_eq!(controls[0].label(), controls[1].label(), "{id}");
    }
    editors[0].bounds().size.height
}

fn type_in(cx: &mut VisualTestContext, input: &Entity<TextareaState>, text: &str) {
    let point = input_snapshot(cx, input).bounds().center();
    cx.simulate_mouse_move(point, None, Modifiers::default());
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input(text);
    settle(cx);
    let editor = input_snapshot(cx, input);
    assert_eq!(editor.focused(), Some(true));
    assert_eq!(editor.value(), Some(text));
}

#[gpui_kit::test]
fn content_panes_shared_composers_match_controls_layout_growth_and_independent_editing(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-shared-composer-ui");
    prepare(&app, cx, &root);
    let inputs = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.persistence.flush();
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.writable = true;
            app.store.ui.full_width_content = true;
            app.store.ui.content_split_ratio = Some(0.5);
            app.show_chat_beside(2, window, cx);
            let reference = app.reference_chat_view(2).unwrap().read(cx).input.clone();
            cx.notify();
            [app.input.clone(), reference]
        })
    });
    assert!(inputs[0] != inputs[1]);
    settle(cx);
    let empty_height = compare_composers(cx, &inputs);
    assert_send_enabled(cx, &inputs[0], false);
    assert_send_enabled(cx, &inputs[1], false);

    // Exercise actual pointer focus and native input, not just set_value.
    type_in(cx, &inputs[0], "main draft");
    assert_eq!(input_snapshot(cx, &inputs[1]).value(), Some(""));
    assert_send_enabled(cx, &inputs[0], true);
    assert_send_enabled(cx, &inputs[1], false);
    type_in(cx, &inputs[1], "reference draft");
    assert_eq!(input_snapshot(cx, &inputs[0]).value(), Some("main draft"));
    type_in(cx, &inputs[0], "edited main draft");
    assert_eq!(
        input_snapshot(cx, &inputs[1]).value(),
        Some("reference draft")
    );
    app.read_with(cx, |app, cx| {
        assert_eq!(app.selected_session, Some(1));
        assert_eq!(app.selected_workspace, Some(0));
        assert_eq!(inputs[0].read(cx).value(), "edited main draft");
        assert_eq!(inputs[1].read(cx).value(), "reference draft");
    });

    // Both inputs must have the shared constructor's 3-row minimum and
    // 8-row cap. This catches the former reference-only auto_grow(2, 5).
    let line_height = cx.update(|_, cx| {
        let main = inputs[0].read(cx).line_height().unwrap();
        near(
            main,
            inputs[1].read(cx).line_height().unwrap(),
            "line height",
        );
        main
    });
    for rows in [6, 12, 16] {
        let value = vec!["short line"; rows].join("\n");
        cx.update(|window, cx| {
            for input in &inputs {
                input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            }
        });
        settle(cx);
        let height = compare_composers(cx, &inputs);
        near(
            height - empty_height,
            line_height * (rows.min(8) - 3) as f32,
            "shared input auto-grow",
        );
    }

    // Keep both columns visible at roughly 350 px each, then check every
    // control against its own card and pane. Clipped overflow cannot pass.
    let main = cx.debug_bounds("content-main").unwrap();
    let reference = cx.debug_bounds("content-reference").unwrap();
    let chrome = cx.update(|window, _| window.viewport_size().width)
        - main.size.width
        - reference.size.width;
    cx.simulate_resize(size(chrome + px(700.), px(1000.)));
    settle(cx);
    compare_composers(cx, &inputs);
    assert!(cx.debug_bounds("content-main").unwrap().size.width <= px(352.));
    type_in(cx, &inputs[1], "narrow reference draft");
    assert_eq!(
        input_snapshot(cx, &inputs[0]).value(),
        Some(vec!["short line"; 16].join("\n").as_str())
    );
    assert_send_enabled(cx, &inputs[0], true);
    assert_send_enabled(cx, &inputs[1], true);
}
