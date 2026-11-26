use super::{AppearanceGroup, UiSettings};
use crate::store::{AppearanceMode, UiIconSize};
use gpui_kit::TestAppContext;

#[gpui_kit::test]
fn appearance_filters_use_real_defaults_and_keep_legacy_deep_links(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "appearance-property-filter");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui = UiSettings::default();
            assert!(
                !app.appearance_properties(cx.entity())
                    .iter()
                    .any(|p| p.changed)
            );
            app.store.ui.chat_line_height = 1.8;
            app.store.ui.code_line_numbers = true;
            let rows = app.appearance_properties(cx.entity());
            let matching = |query, modified, group, legacy| {
                rows.iter()
                    .filter(|p| p.matches(query, modified, group, legacy))
                    .map(|p| p.id)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                matching("", true, None, None),
                ["chat_line_height", "code_line_numbers"]
            );
            assert_eq!(
                matching("CHAT line", true, None, None),
                ["chat_line_height"]
            );
            assert_eq!(
                matching("code_line_numbers", false, None, None),
                ["code_line_numbers"]
            );
            assert_eq!(matching("行号", false, None, None), ["code_line_numbers"]);
            assert_eq!(
                matching("", true, Some(AppearanceGroup::Chat), None),
                ["chat_line_height"]
            );
            assert!(matching("行号", false, None, Some(0)).is_empty());
            assert_eq!(matching("", false, None, Some(2)), ["background"]);
            assert!(
                matching("", false, None, Some(1))
                    .iter()
                    .all(|id| id.contains("font")
                        || id.contains("code")
                        || id.contains("chat")
                        || *id == "icon_size")
            );
            app.select_settings_page(1, window, cx);
            app.settings_ui.appearance_modified_only = true;
            app.settings_ui.appearance_group = Some(AppearanceGroup::Code);
            app.settings_ui
                .appearance_filter
                .update(cx, |input, cx| input.set_value("code", window, cx));
            let result = app
                .settings_search_matches("背景")
                .into_iter()
                .find(|r| r.page == 1 && r.group == Some(2))
                .unwrap();
            app.open_settings_result(result, window, cx);
            assert_eq!(app.settings_ui.group, Some(2));
            assert!(
                app.settings_ui
                    .appearance_filter
                    .read(cx)
                    .value()
                    .is_empty()
            );
            assert!(!app.settings_ui.appearance_modified_only);
            assert_eq!(app.settings_ui.appearance_group, None);
        });
    });
    cx.update(|window, _| window.remove_window());
    drop(app);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn appearance_group_resets_are_isolated_and_readonly_safe(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "appearance-scoped-resets");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.persistence.flush();
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.store.ui.ui_font_size = 18.;
            app.store.ui.icon_size = UiIconSize::Large;
            app.store.ui.chat_font_size = 19.;
            app.store.ui.chat_line_height = 1.9;
            app.store.ui.code_font_size = 17.;
            app.store.ui.code_line_numbers = true;
            app.store.ui.auto_sort_workspaces = false;
            app.store.ui.auto_collapse_inactive_days = 12;
            app.store.ui.appearance = AppearanceMode::Dark;
            app.store.ui.background.strength = 30;
            let original = serde_json::to_value(&app.store).unwrap();
            for (writable, closing) in [(false, false), (true, true)] {
                app.writable = writable;
                app.closing = closing;
                for group in AppearanceGroup::ALL {
                    app.reset_appearance_group(group, window, cx);
                }
                assert_eq!(serde_json::to_value(&app.store).unwrap(), original);
            }
            app.writable = true;
            app.closing = false;
            let defaults = serde_json::to_value(UiSettings::default()).unwrap();
            let mut expected = original;
            for (group, keys) in [
                (
                    AppearanceGroup::Chat,
                    vec!["chat_font_family", "chat_font_size", "chat_line_height"],
                ),
                (
                    AppearanceGroup::Code,
                    vec![
                        "code_font_family",
                        "code_font_size",
                        "code_copy_button",
                        "code_line_numbers",
                        "code_syntax_highlight",
                    ],
                ),
                (
                    AppearanceGroup::Interface,
                    vec!["ui_font_family", "ui_font_size", "icon_size"],
                ),
                (
                    AppearanceGroup::Theme,
                    vec!["appearance", "light_theme", "dark_theme"],
                ),
                (AppearanceGroup::Background, vec!["background"]),
            ] {
                let editor = app.input.entity_id();
                app.reset_appearance_group(group, window, cx);
                for key in keys {
                    expected["ui"][key] = defaults[key].clone();
                }
                assert_eq!(
                    serde_json::to_value(&app.store).unwrap(),
                    expected,
                    "{group:?}"
                );
                assert_eq!(app.input.entity_id(), editor);
            }
            app.persistence.flush();
        });
    });
    cx.update(|window, _| window.remove_window());
    drop(app);
    cx.run_until_parked();
}
