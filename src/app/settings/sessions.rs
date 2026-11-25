//! Conversation preferences grouped by their scope of effect.

use super::*;
use gpui_kit::component::switch::Switch;

impl ConduitApp {
    /// Stable route 10 groups: defaults, permission, chat display, automatic naming.
    /// The defaults group owns its engine selector; permission and display are global.
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
                "settings.sessions.naming_section",
            ))
            .items(self.settings_automatic_naming_items(disabled, view.clone()))
            .items(self.settings_naming_model_items(disabled, view.clone())),
            self.settings_empty_session_cleanup_group(disabled, view.clone()),
            self.settings_auto_archive_group(disabled, view),
        ])
    }

    fn settings_auto_archive_group(&self, disabled: bool, view: Entity<Self>) -> SettingGroup {
        let enabled_view = view.clone();
        let enabled = SettingField::render(move |_, _, cx| {
            let view = enabled_view.clone();
            Switch::new("session-auto-archive-enabled")
                .checked(view.read(cx).store.ui.auto_archive.enabled)
                .accessibility_label(conduit::i18n::text_static(
                    "settings.sessions.auto_archive_enabled",
                ))
                .disabled(disabled)
                .on_click(move |enabled, _, cx| {
                    view.update(cx, |app, cx| app.set_auto_archive_enabled(*enabled, cx));
                })
        });
        let period = SettingField::render(move |_, _, cx| {
            let view = view.clone();
            let policy = &view.read(cx).store.ui.auto_archive;
            let selected = policy.after;
            Button::new("session-auto-archive-days")
                .debug_selector(|| "session-auto-archive-days".into())
                .outline()
                .w(px(160.))
                .justify_between()
                .label(selected.label())
                .icon(FullIconName::ChevronDown)
                .accessibility_label(conduit::i18n::text_static(
                    "settings.sessions.auto_archive_after",
                ))
                .disabled(disabled)
                .dropdown_menu(move |mut menu, _, _| {
                    for after in crate::store::AutoArchiveDays::ALL {
                        let view = view.clone();
                        menu = menu.item(
                            PopupMenuItem::new(after.label())
                                .checked(after == selected)
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |app, cx| app.set_auto_archive_days(after, cx));
                                }),
                        );
                    }
                    menu
                })
        });
        settings_section(conduit::i18n::text_static("settings.sessions.auto_archive")).items(vec![
            SettingItem::new(
                conduit::i18n::text_static("settings.sessions.auto_archive_enabled"),
                enabled,
            )
            .disabled(disabled),
            SettingItem::new(
                conduit::i18n::text_static("settings.sessions.auto_archive_after"),
                period,
            )
            .disabled(disabled),
        ])
    }

    fn settings_empty_session_cleanup_group(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingGroup {
        let read_view = view.clone();
        let cleanup_empty_sessions = SettingField::switch(
            move |cx: &App| read_view.read(cx).store.ui.auto_cleanup_empty_sessions,
            move |value, cx| {
                view.update(cx, |this, cx| {
                    if !this.writable || this.closing {
                        return;
                    }
                    this.store.ui.auto_cleanup_empty_sessions = value;
                    if value {
                        this.cleanup_stale_empty_sessions(cx);
                    }
                    this.persist();
                    cx.notify();
                });
            },
        );
        settings_section(conduit::i18n::text_static(
            "settings.sessions.empty_cleanup",
        ))
        .items(vec![
            SettingItem::new(
                conduit::i18n::text_static("settings.sessions.auto_cleanup_empty_sessions"),
                cleanup_empty_sessions,
            )
            .disabled(disabled),
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
                        menu = menu.item(
                            PopupMenuItem::element(move |_, _| {
                                Self::permission_option_content(option, option == permission)
                                    .w(px(220.))
                            })
                            .on_click(move |_, _, cx| {
                                view.update(cx, |app, cx| {
                                    if !app.writable || app.closing {
                                        return;
                                    }
                                    app.store.ui.default_session_permission = option;
                                    app.pending_permission = option;
                                    app.persist();
                                    cx.notify();
                                });
                            }),
                        );
                    }
                    menu
                })
        });
        settings_section(conduit::i18n::text_static(
            "settings.sessions.permission_section",
        ))
        .item(
            SettingItem::new(
                conduit::i18n::text_static("settings.general.default_session_permission"),
                permission_field,
            )
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
        settings_section(conduit::i18n::text_static(
            "settings.sessions.display_section",
        ))
        .items(vec![
            SettingItem::new(
                conduit::i18n::text_static("settings.general.collapse_reasoning"),
                collapse_reasoning,
            )
            .disabled(disabled),
            SettingItem::new(
                conduit::i18n::text_static("settings.general.auto_collapse_question_history"),
                collapse_questions,
            )
            .disabled(disabled),
        ])
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
            .items(self.settings_automatic_naming_items(disabled, view.clone())),
            settings_section(conduit::i18n::text_static("settings.general.naming_model"))
                .items(self.settings_naming_model_items(disabled, view.clone())),
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
            .disabled(disabled),
            SettingItem::new(conduit::i18n::text_static("settings.general.trigger"), mode)
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
        // The pickers also serve manual renaming when automatic naming is off.
        vec![self.generation_disclosure(
            true,
            view,
            vec![
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.naming_engine"),
                    provider,
                )
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
],
        )]
    }
}
