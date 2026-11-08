use crate::app::queue::tests as queue_tests;
use conduit::i18n::{self, Locale};
use gpui_kit::{TestAppContext, VisualTestContext, px, size};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

#[gpui_kit::test]
fn language_switch_preserves_running_tasks_drafts_selection_and_preferences(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, path) = queue_tests::fixture(cx, "language-switch");
    let (tx, rx) = std::sync::mpsc::channel();
    let mut sender = None;
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.store.ui.language = Locale::legacy();
            app.persistence = crate::persistence::Persistence::with_writer(move |store| {
                tx.send(store).unwrap();
                Ok(())
            });
            sender = Some(queue_tests::running(app, 1));
            app.input.update(cx, |input, cx| {
                input.set_value("draft 中文 🦀", window, cx);
                input.set_selected_range(0..5, cx);
            });
            app.queued_messages.insert(
                1,
                crate::store::QueuedMessage {
                    text: "queued 中文".into(),
                    references: vec![],
                    paused: None,
                },
            );
            app.drafts.insert(2, "another draft".into());
            app.ensure_commit_draft(window, cx);
            app.git.drafts[&path].input.update(cx, |input, cx| {
                input.set_value("feat: keep my message", window, cx)
            });
            app.git_prompt_input.update(cx, |input, cx| {
                input.set_value("My custom prompt 中文", window, cx)
            });
            app.store.ui.chat_font_size = 18.;
            app.store.ui.code_font_size = 14.;
            app.select_language(Locale::default(), window, cx);
        })
    });
    let saved = rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(saved.ui.language.as_str(), "en");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(i18n::locale().as_str(), "en");
            assert_eq!(app.input.read(cx).value().as_ref(), "draft 中文 🦀");
            assert_eq!(app.input.read(cx).selected_range(), 0..5);
            assert_eq!(
                app.git.drafts[&path].input.read(cx).value().as_ref(),
                "feat: keep my message"
            );
            assert_eq!(
                app.git_prompt_input.read(cx).value().as_ref(),
                "My custom prompt 中文"
            );
            assert_eq!(app.queued_messages[&1].text, "queued 中文");
            assert_eq!(app.drafts[&2], "another draft");
            assert!(!app.session_control.runs[&1].runtime.is_cancelled());
            assert_eq!(app.store.ui.chat_font_size, 18.);
            assert_eq!(app.store.ui.code_font_size, 14.);
            app.select_language(Locale::legacy(), window, cx);
            assert_eq!(app.input.read(cx).value().as_ref(), "draft 中文 🦀");
            assert_eq!(app.input.read(cx).selected_range(), 0..5);
            assert!(!app.session_control.runs[&1].runtime.is_cancelled());
        })
    });
    let saved = rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(saved.ui.language.as_str(), "zh-CN");
    drop(sender);
}

#[gpui_kit::test]
fn language_selector_and_search_work_in_both_languages_at_narrow_width(cx: &mut TestAppContext) {
    let _locale = crate::test_support::scoped("en");
    let (app, cx, _) = queue_tests::fixture(cx, "language-search");
    cx.simulate_resize(size(px(850.), px(500.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.store.ui.language = Locale::default();
            app.open_settings(window, cx);
            app.settings_ui.group = Some(1);
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("application-language").is_some());
    for language in [Locale::legacy(), Locale::default()] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.select_language(language.clone(), window, cx);
                for query in ["language", "English", "简体中文", "应用语言"] {
                    assert!(
                        app.settings_search_matches(query)
                            .iter()
                            .any(|result| result.page == 0 && result.group == Some(1)),
                        "{query}"
                    );
                }
                let result = app
                    .settings_search_matches("language")
                    .into_iter()
                    .find(|result| result.page == 0 && result.group == Some(1))
                    .unwrap();
                assert_eq!(result.title, i18n::text("language.title"));
            })
        });
        draw(cx);
        assert!(cx.debug_bounds("application-language").is_some());
    }
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.writable = false;
            app.select_language(Locale::legacy(), window, cx);
            assert_eq!(app.store.ui.language.as_str(), "en");
        })
    });
}
