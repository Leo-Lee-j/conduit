//! Conversation preferences grouped by their scope of effect.

use super::*;

impl ConduitApp {
    /// Stable route 10 groups: defaults, permission, chat display, automatic naming.
    /// The host renders the shared management engine selector above this page.
    pub(in crate::app) fn settings_conversation_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        let mut page = self.settings_defaults_page(disabled, view.clone());
        page.page = page
            .page
            .title(conduit::i18n::text_static("settings.sessions.title"))
            .icon(FullIconName::MessageSquare);
        page.groups([
            self.settings_conversation_permission_group(disabled, view.clone()),
            self.settings_conversation_display_group(disabled, view.clone()),
            settings_section(conduit::i18n::text_static(
                "settings.defaults.automatic_naming",
            ))
            .items(self.settings_automatic_naming_items(disabled, view.clone()))
            .items(self.settings_naming_model_items(disabled, view))
            .footer(|_, _| {
                conduit::i18n::text_static(
                    "settings.general.naming_runs_in_the_background_failures_keep",
                )
            }),
        ])
    }

    fn settings_conversation_permission_group(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingGroup {
        let permission_view = view.clone();
        let permission = self.store.ui.default_session_permission;
        let permission_field = SettingField::render(move |_, _, _| {
            let view = permission_view.clone();
            Button::new("default-session-permission")
                .outline()
                .w(px(160.))
                .justify_between()
                .label(permission.label())
                .icon(FullIconName::ChevronDown)
                .disabled(disabled)
                .dropdown_menu(move |mut menu, _, _| {
                    for option in [
                        PermissionLevel::ReadOnly,
                        PermissionLevel::WorkspaceWrite,
                        PermissionLevel::FullAccess,
                    ] {
                        let view = view.clone();
                        menu = menu.item(PopupMenuItem::new(option.label()).on_click(
                            move |_, _, cx| {
                                view.update(cx, |app, cx| {
                                    if !app.writable || app.closing {
                                        return;
                                    }
                                    app.store.ui.default_session_permission = option;
                                    app.pending_permission = option;
                                    app.persist();
                                    cx.notify();
                                });
                            },
                        ));
                    }
                    menu
                })
        });
        settings_section(conduit::i18n::text_static(
            "settings.general.default_session_permission",
        ))
        .item(
            SettingItem::new(
                conduit::i18n::text_static("settings.general.default_session_permission"),
                permission_field,
            )
            .description(settings_description(conduit::i18n::text_static(
                "settings.sessions.permission_scope",
            )))
            .disabled(disabled),
        )
        .footer(|_, _| conduit::i18n::text_static("settings.sessions.permission_risk"))
    }

    fn settings_conversation_display_group(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingGroup {
        let collapse_reasoning = switch_field(
            view.clone(),
            |app| app.store.ui.collapse_reasoning,
            ConduitApp::set_reasoning_collapsed,
        );
        let collapse_questions = switch_field(
            view.clone(),
            |app| app.store.ui.auto_collapse_question_history,
            ConduitApp::set_question_history_collapsed,
        );
        settings_section(conduit::i18n::text_static("settings.sessions.chat_display"))
            .items(vec![
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.collapse_reasoning"),
                    collapse_reasoning,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.collapse_reasoning_description",
                )))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.auto_collapse_question_history"),
                    collapse_questions,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.auto_collapse_question_history_description",
                )))
                .disabled(disabled),
            ])
            .footer(|_, _| conduit::i18n::text_static("settings.sessions.chat_display_scope"))
    }

    /// Compatibility route 5 retains group 0 for the switch/trigger and group 1
    /// for naming engine/service/model. Route 10 composes both into group 3.
    pub(in crate::app) fn settings_session_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        settings_page(
            conduit::i18n::text_static("settings.defaults.automatic_naming"),
            FullIconName::Pencil,
        )
        .groups([
            settings_section(conduit::i18n::text_static(
                "settings.general.session_naming",
            ))
            .items(self.settings_automatic_naming_items(disabled, view.clone()))
            .footer(|_, _| {
                conduit::i18n::text_static(
                    "settings.general.naming_runs_in_the_background_failures_keep",
                )
            }),
            settings_section(conduit::i18n::text_static("settings.general.naming_model"))
                .items(self.settings_naming_model_items(disabled, view))
                .footer(|_, _| conduit::i18n::text_static("settings.sessions.naming_model_scope")),
        ])
    }

    fn settings_automatic_naming_items(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> Vec<SettingItem> {
        let rename_view = view.clone();
        let rename_read = rename_view.clone();
        let auto_rename = SettingField::switch(
            move |cx: &App| rename_read.read(cx).store.ui.auto_rename,
            move |value, cx| {
                rename_view.update(cx, |this, cx| {
                    if !this.writable || this.closing {
                        return;
                    }
                    this.store.ui.auto_rename = value;
                    if !value {
                        this.session_control.namings.retain(|_, naming| {
                            if naming.automatic {
                                naming.runtime.cancel();
                                false
                            } else {
                                true
                            }
                        });
                    }
                    this.persist();
                    cx.notify();
                });
            },
        );
        let mode_view = view.clone();
        let mode = SettingField::render(move |_, _, cx| {
            let auto_rename = mode_view.read(cx).store.ui.auto_rename;
            mode_view
                .read(cx)
                .naming_mode_picker(mode_view.clone(), disabled || !auto_rename)
        });
        vec![
            SettingItem::new(
                conduit::i18n::text_static("settings.defaults.automatic_naming"),
                auto_rename,
            )
            .description(settings_description(conduit::i18n::text_static(
                "settings.general.manual_ai_renaming_remains_available_when_disabled",
            )))
            .disabled(disabled),
            SettingItem::new(conduit::i18n::text_static("settings.general.trigger"), mode)
                .description(settings_description(conduit::i18n::text_static(
                    "settings.sessions.naming_trigger_scope",
                )))
                .disabled(disabled),
        ]
    }

    fn settings_naming_model_items(&self, disabled: bool, view: Entity<Self>) -> Vec<SettingItem> {
        let provider_view = view.clone();
        let provider = SettingField::render(move |_, _, cx| {
            provider_view
                .read(cx)
                .naming_provider_picker(provider_view.clone(), disabled)
        });
        let channel_view = view.clone();
        let channel = SettingField::render(move |_, _, cx| {
            channel_view
                .read(cx)
                .naming_channel_picker(channel_view.clone(), disabled)
        });
        let model_view = view.clone();
        let model = SettingField::render(move |_, _, cx| {
            model_view
                .read(cx)
                .naming_model_picker(model_view.clone(), disabled)
        });
        let mut items = vec![self.generation_disclosure(true, view)];
        if self.settings_ui.naming_expanded {
            // These pickers also serve manual AI renaming: only the trigger is
            // disabled when automatic naming is off, not this configuration.
            items.extend([
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.naming_engine"),
                    provider,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.sessions.naming_model_scope",
                )))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.naming_service"),
                    channel,
                )
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.naming_model"),
                    model,
                )
                .disabled(disabled),
            ]);
        }
        items
    }
}
