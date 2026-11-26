use super::*;

impl ConduitApp {
    pub(super) fn appearance_properties(&self, view: Entity<Self>) -> Vec<Property> {
        use AppearanceGroup::*;
        let ui = &self.store.ui;
        let defaults = UiSettings::default();
        let disabled = !self.writable || self.closing;
        let mut properties = Vec::new();
        macro_rules! add {
            ($field:ident, $group:expr, $key:literal, $item:expr) => {
                properties.push(Property {
                    id: stringify!($field),
                    group: $group,
                    title: conduit::i18n::text_static($key),
                    changed: ui.$field != defaults.$field,
                    item: $item.disabled(disabled),
                });
            };
        }
        macro_rules! field {
            ($name:ident, $group:expr, $key:literal, $control:expr) => {
                add!(
                    $name,
                    $group,
                    $key,
                    SettingItem::new(conduit::i18n::text_static($key), $control)
                );
            };
        }
        let theme_view = view.clone();
        let appearance = ui.appearance;
        field!(
            appearance,
            Theme,
            "settings.appearance.theme",
            SettingField::render(move |_, _, _| theme_options(
                appearance,
                theme_view.clone(),
                disabled
            ))
        );
        field!(
            light_theme,
            Theme,
            "settings.appearance.light_theme",
            theme_select_field(
                view.clone(),
                gpui_kit::component::ThemeMode::Light,
                disabled
            )
        );
        field!(
            dark_theme,
            Theme,
            "settings.appearance.dark_theme",
            theme_select_field(view.clone(), gpui_kit::component::ThemeMode::Dark, disabled)
        );
        add!(
            ui_font_family,
            Interface,
            "settings.appearance.interface_font",
            font_select_item(view.clone(), FontTarget::Interface, disabled)
        );
        field!(
            ui_font_size,
            Interface,
            "settings.appearance.interface_font_size",
            typography_field(
                "ui-font-size",
                ui.ui_font_size.clamp(MIN_UI_FONT_SIZE, MAX_UI_FONT_SIZE),
                disabled,
                TypographyField::UiSize,
                view.clone()
            )
        );
        let icon_view = view.clone();
        let icon_size = ui.icon_size;
        field!(
            icon_size,
            Interface,
            "settings.appearance.functional_icon_size",
            SettingField::render(move |_, _, _| icon_size_options(
                icon_size,
                icon_view.clone(),
                disabled
            ))
        );
        add!(
            chat_font_family,
            Chat,
            "settings.appearance.body_font",
            font_select_item(view.clone(), FontTarget::Chat, disabled)
        );
        field!(
            chat_font_size,
            Chat,
            "settings.appearance.font_size",
            typography_field(
                "chat-font-size",
                ui.chat_font_size
                    .clamp(MIN_CHAT_FONT_SIZE, MAX_CHAT_FONT_SIZE),
                disabled,
                TypographyField::ChatSize,
                view.clone()
            )
        );
        field!(
            chat_line_height,
            Chat,
            "settings.appearance.line_height",
            typography_field(
                "chat-line-height",
                ui.chat_line_height
                    .clamp(MIN_CHAT_LINE_HEIGHT, MAX_CHAT_LINE_HEIGHT),
                disabled,
                TypographyField::ChatLineHeight,
                view.clone()
            )
        );
        add!(
            code_font_family,
            Code,
            "settings.appearance.code_font",
            font_select_item(view.clone(), FontTarget::Code, disabled)
        );
        field!(
            code_font_size,
            Code,
            "settings.appearance.code_font_size",
            typography_field(
                "code-font-size",
                ui.code_font_size
                    .clamp(MIN_CODE_FONT_SIZE, MAX_CODE_FONT_SIZE),
                disabled,
                TypographyField::CodeSize,
                view.clone()
            )
        );
        field!(
            code_copy_button,
            Code,
            "settings.appearance.copy_button",
            switch_field(
                view.clone(),
                |app| app.store.ui.code_copy_button,
                |app, value| app.store.ui.code_copy_button = value
            )
        );
        field!(
            code_line_numbers,
            Code,
            "settings.appearance.line_numbers",
            switch_field(
                view.clone(),
                |app| app.store.ui.code_line_numbers,
                |app, value| app.store.ui.code_line_numbers = value
            )
        );
        field!(
            code_syntax_highlight,
            Code,
            "settings.appearance.syntax_highlighting",
            switch_field(
                view.clone(),
                |app| app.store.ui.code_syntax_highlight,
                |app, value| app.store.ui.code_syntax_highlight = value
            )
        );
        properties.push(Property {
            id: "background",
            group: Background,
            title: conduit::i18n::text_static("wallpaper.title"),
            // Download history is retained content, not a changed preference.
            changed: ui.background.enabled != defaults.background.enabled
                || ui.background.id.is_some()
                || ui.background.strength != defaults.background.strength,
            item: SettingItem::render(move |_, _, cx| {
                view.read(cx).background_controls(view.clone())
            })
            .disabled(disabled),
        });
        properties
    }
}
