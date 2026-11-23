//! General settings.

use super::*;
use gpui_kit::component::switch::Switch;

impl ConduitApp {
    pub(in crate::app) fn settings_general_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        let sessions_limit = count_stepper_field(
            "sessions-limit",
            self.store.ui.sessions_per_workspace.clamp(1, 30),
            1,
            30,
            conduit::i18n::text_static("settings.general.decrease_workspace_session_count"),
            conduit::i18n::text_static("settings.general.increase_workspace_session_count"),
            disabled,
            view.clone(),
            |app, value| app.store.ui.sessions_per_workspace = value,
        );
        let max_tabs_limit = count_stepper_field(
            "max-tabs",
            self.store.ui.max_tabs.clamp(1, MAX_TAB_LIMIT),
            1,
            MAX_TAB_LIMIT,
            conduit::i18n::text_static("settings.general.decrease_maximum_tab_count"),
            conduit::i18n::text_static("settings.general.increase_maximum_tab_count"),
            disabled,
            view.clone(),
            |app, value| app.store.ui.max_tabs = value,
        );
        let full_width = switch_field(
            view.clone(),
            |app| app.store.ui.full_width_content,
            |app, value| app.store.ui.full_width_content = value,
        );
        let files_beside = switch_field(
            view.clone(),
            |app| app.store.ui.files_open_beside,
            |app, value| app.store.ui.files_open_beside = value,
        );
        let system_notifications = switch_field(
            view.clone(),
            |app| app.store.ui.system_notifications,
            |app, value| app.store.ui.system_notifications = value,
        );
        let system_notifications_description = if self.store.ui.system_notifications
            && self.notifications.system_preference == Some(true)
            && !self.notifications.system_available
        {
            conduit::i18n::text_static("settings.general.system_notifications_unavailable")
        } else {
            conduit::i18n::text_static("settings.general.system_notifications_description")
        };
        let compact_middle_packages = switch_field(
            view.clone(),
            |app| app.store.ui.compact_middle_packages,
            |app, value| app.store.ui.compact_middle_packages = value,
        );
        let group_tabs = switch_field(
            view.clone(),
            |app| app.store.ui.group_tabs,
            |app, value| app.store.ui.group_tabs = value,
        );
        let language_view = view.clone();
        let language = self.store.ui.language.clone();
        let language_label = conduit::i18n::languages()
            .iter()
            .find(|entry| entry.id == language.as_str())
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| "English".into());
        let language_field = SettingField::render(move |_, _, _| {
            let view = language_view.clone();
            Button::new("application-language")
                .debug_selector(|| "application-language".into())
                .outline()
                .w(px(160.))
                .justify_between()
                .label(language_label.clone())
                .icon(FullIconName::ChevronDown)
                .disabled(disabled)
                .dropdown_menu(move |mut menu, _, _| {
                    for entry in conduit::i18n::languages() {
                        let view = view.clone();
                        let locale = conduit::i18n::Locale::from(entry.id.clone());
                        menu = menu.item(PopupMenuItem::new(entry.name.clone()).on_click(
                            move |_, window, cx| {
                                view.update(cx, |app, cx| {
                                    app.select_language(locale.clone(), window, cx)
                                });
                            },
                        ));
                    }
                    menu
                })
        });
        settings_page(
            conduit::i18n::text_static("settings.general.general"),
            FullIconName::Settings,
        )
        .groups(vec![
            settings_section(conduit::i18n::text_static(
                "settings.general.workbench_layout",
            ))
            .items(vec![
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.full_width_content"),
                    full_width,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.fill_the_available_area_with_chat_content",
                )))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.system_notifications"),
                    system_notifications,
                )
                .description(settings_description(system_notifications_description))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.files_open_beside"),
                    files_beside,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.files_open_beside_description",
                )))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.compact_middle_packages"),
                    compact_middle_packages,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.compact_middle_packages_description",
                )))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.group_tabs"),
                    group_tabs,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.group_tabs_description",
                )))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.sidebar_session_count"),
                    sessions_limit,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.default_number_of_ordinary_sessions_per_workspace",
                )))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.tab_limit"),
                    max_tabs_limit,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.automatically_close_the_oldest_tab_when_the",
                )))
                .disabled(disabled),
            ]),
            settings_section(conduit::i18n::text("language.section")).items(vec![
                SettingItem::new(conduit::i18n::text("language.title"), language_field)
                    .description(settings_description(conduit::i18n::text(
                        "language.description",
                    )))
                    .disabled(disabled),
            ]),
            #[cfg(feature = "remote")]
            self.remote_settings_section(view, disabled),
        ])
    }

    pub(in crate::app) fn settings_maintenance_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        let state_path = Store::path();
        let data_dir = state_path
            .parent()
            .map(display_path)
            .unwrap_or_else(|| "—".into());
        let state_file = display_path(&state_path);
        let auto_check = switch_field(
            view.clone(),
            |app| app.store.updates.auto_check,
            |app, value| app.store.updates.auto_check = value,
        );
        let auto_download = switch_field(
            view.clone(),
            |app| app.store.updates.auto_download,
            |app, value| app.store.updates.auto_download = value,
        );
        let auto_install = switch_field(
            view.clone(),
            |app| app.store.updates.auto_install,
            |app, value| app.store.updates.auto_install = value,
        );
        let performance_view = view.clone();
        let performance_read = performance_view.clone();
        let performance = SettingField::switch(
            move |cx: &App| performance_read.read(cx).store.ui.performance_monitor,
            move |value, cx| {
                performance_view.update(cx, |this, cx| {
                    if !this.writable || this.closing {
                        return;
                    }
                    this.store.ui.performance_monitor = value;
                    if value {
                        this.performance.update(cx, |monitor, _| monitor.reset());
                    } else {
                        this.performance_open = false;
                    }
                    this.persist();
                    cx.notify();
                });
            },
        );
        settings_page(
            conduit::i18n::text_static("settings.general.data_and_updates"),
            FullIconName::Settings,
        )
        .groups(vec![
            settings_section(conduit::i18n::text_static("settings.general.local_data"))
                .items(vec![
                    value_item(
                        conduit::i18n::text_static("settings.general.data_directory"),
                        data_dir,
                    ),
                    value_item(
                        conduit::i18n::text_static("settings.general.configuration_file"),
                        state_file,
                    ),
                ])
                .footer(|_, _| {
                    conduit::i18n::text_static(
                        "settings.general.configuration_is_stored_locally_only",
                    )
                }),
            settings_section(conduit::i18n::text_static(
                "settings.general.application_updates",
            ))
            .items(vec![
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.automatically_check_for_updates"),
                    auto_check,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.check_in_the_background_after_startup_and",
                )))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.download_updates_in_background"),
                    auto_download,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.download_and_verify_new_versions_when_found",
                )))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.install_and_restart_when_idle"),
                    auto_install,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.save_drafts_after_tasks_finish_install_the",
                )))
                .disabled(disabled),
            ]),
            settings_section(conduit::i18n::text_static("settings.general.diagnostics")).item(
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.performance_monitoring"),
                    performance,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.show_performance_monitor_in_the_bottom_toolbar",
                )))
                .disabled(disabled),
            ),
        ])
    }

    pub(in crate::app) fn settings_git_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        let provider_view = view.clone();
        let provider = SettingField::render(move |_, _, cx| {
            provider_view
                .read(cx)
                .git_provider_picker(provider_view.clone(), disabled)
        });
        let channel_view = view.clone();
        let channel = SettingField::render(move |_, _, cx| {
            channel_view
                .read(cx)
                .git_channel_picker(channel_view.clone(), disabled)
        });
        let model_view = view.clone();
        let model = SettingField::render(move |_, _, cx| {
            model_view
                .read(cx)
                .git_model_picker(model_view.clone(), disabled)
        });
        let prompt_input = self.git_prompt_input.clone();
        let prompt = SettingField::render(move |_, _, _| {
            Textarea::new(&prompt_input)
                .h(px(180.))
                .w_full()
                .text_size(ui_text_size(12.))
                .aria_label(conduit::i18n::text_static(
                    "settings.general.commit_message_generation_prompt",
                ))
                .disabled(disabled)
        });
        let reset_view = view.clone();
        let reset_input = self.git_prompt_input.clone();
        let reset = SettingField::render(move |_, _window, _| {
            Button::new("git-prompt-reset")
                .label(conduit::i18n::text_static(
                    "settings.appearance.restore_defaults",
                ))
                .small()
                .ghost()
                .disabled(disabled)
                .when(!disabled, |button| button.cursor_pointer())
                .on_click({
                    let view = reset_view.clone();
                    let input = reset_input.clone();
                    move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            if !this.writable || this.closing {
                                return;
                            }
                            this.store.git.commit_message_prompt =
                                crate::store::DEFAULT_COMMIT_PROMPT.into();
                            input.update(cx, |input, cx| {
                                input.set_value(crate::store::DEFAULT_COMMIT_PROMPT, window, cx);
                            });
                            this.persist();
                            cx.notify();
                        });
                    }
                })
        });
        let generation = self.generation_disclosure(
            false,
            view.clone(),
            vec![
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.generation_engine"),
                    provider,
                )
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.generation_service"),
                    channel,
                )
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.generation_model"),
                    model,
                )
                .disabled(disabled),
            ],
        );
        let tag_settings_view = view.clone();
        let tag_increment = self.store.git.tag_increment;
        let tag_strategy = SettingField::render(move |_, _, _| {
            let app = tag_settings_view.clone();
            Button::new("git-tag-strategy")
                .small()
                .label(match tag_increment {
                    crate::store::TagIncrement::Patch => "Patch +1",
                    crate::store::TagIncrement::Minor => "Minor +1",
                    crate::store::TagIncrement::Major => "Major +1",
                })
                .disabled(disabled)
                .dropdown_menu(move |mut menu, _, _| {
                    for (strategy, label) in [
                        (crate::store::TagIncrement::Patch, "Patch +1"),
                        (crate::store::TagIncrement::Minor, "Minor +1"),
                        (crate::store::TagIncrement::Major, "Major +1"),
                    ] {
                        let app = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(label)
                                .checked(strategy == tag_increment)
                                .on_click(move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        if this.writable && !this.closing {
                                            this.store.git.tag_increment = strategy;
                                            this.persist();
                                            cx.notify();
                                        }
                                    });
                                }),
                        );
                    }
                    menu
                })
        });
        let tag_step = count_stepper_field(
            "git-tag-step",
            self.store.git.tag_step.clamp(1, 100),
            1,
            100,
            conduit::i18n::text_static("settings.general.decrease_tag_increment"),
            conduit::i18n::text_static("settings.general.increase_tag_increment"),
            disabled,
            view.clone(),
            |app, value| app.store.git.tag_step = value,
        );
        let expanded = self.settings_ui.prompt_expanded;
        let preview = if self.store.git.commit_message_prompt.trim().is_empty() {
            crate::store::DEFAULT_COMMIT_PROMPT
        } else {
            &self.store.git.commit_message_prompt
        }
        .to_owned();
        let prompt_view = view.clone();
        let prompt_fields = [
            SettingItem::new(
                conduit::i18n::text_static("settings.general.prompt"),
                prompt,
            )
            .layout(Axis::Vertical)
            .disabled(disabled),
            SettingItem::new(
                conduit::i18n::text_static("settings.appearance.restore_defaults"),
                reset,
            )
            .disabled(disabled),
        ];
        let edit = SettingItem::render(move |options, window, cx| {
            let view = prompt_view.clone();
            settings_accordion(
                "git-prompt-expand",
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .debug_selector(|| "git-prompt-expand".into())
                            .child(conduit::i18n::text_static("settings.general.edit_prompt")),
                    )
                    .when(!expanded, |title| {
                        title.child(
                            div()
                                .truncate()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().muted))
                                .child(preview.clone()),
                        )
                    }),
                expanded,
                v_flex().child(accordion_fields(&prompt_fields, options, window, cx)),
                move |open, _, cx| {
                    view.update(cx, |app, cx| {
                        app.settings_ui.prompt_expanded = open;
                        cx.notify();
                    })
                },
            )
        })
        .keywords([
            conduit::i18n::text_static("settings.general.prompt"),
            "prompt",
        ]);
        let auto_fetch_enabled = self.store.git.auto_fetch_incoming;
        let auto_fetch = SettingField::render(move |_, _, _| {
            let view = view.clone();
            Switch::new("git-auto-fetch-incoming")
                .checked(auto_fetch_enabled)
                .accessibility_label(conduit::i18n::text_static(
                    "settings.general.auto_fetch_incoming",
                ))
                .disabled(disabled)
                .on_click(move |enabled, _, cx| {
                    view.update(cx, |app, cx| {
                        if !app.writable || app.closing {
                            return;
                        }
                        app.store.git.auto_fetch_incoming = *enabled;
                        if *enabled {
                            app.refresh_branches();
                        } else {
                            app.cancel_incoming_fetch();
                            app.git.incoming_fetch = None;
                        }
                        app.persist();
                        cx.notify();
                    });
                })
        });
        settings_page(
            conduit::i18n::text_static("settings.general.git_commits"),
            FullIconName::GitFork,
        )
        .groups([
            settings_section(conduit::i18n::text_static(
                "settings.general.tag_generation",
            ))
            .items(vec![
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.version_increment_strategy"),
                    tag_strategy,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.generate_a_version_from_known_stable_tags",
                )))
                .disabled(disabled),
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.increment"),
                    tag_step,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.increase_the_selected_segment_by_this_amount",
                )))
                .disabled(disabled),
            ]),
            settings_section(conduit::i18n::text_static(
                "settings.general.generation_model",
            ))
            .item(generation),
            SettingGroup::new()
                .title(conduit::i18n::text_static(
                    "settings.general.commit_message_prompt",
                ))
                .item(edit)
                .footer(|_, _| {
                    conduit::i18n::text_static(
                        "settings.general.changes_save_automatically_and_apply_to_the",
                    )
                }),
            settings_section(conduit::i18n::text_static(
                "settings.general.branch_updates",
            ))
            .item(
                SettingItem::new(
                    conduit::i18n::text_static("settings.general.auto_fetch_incoming"),
                    auto_fetch,
                )
                .description(settings_description(conduit::i18n::text_static(
                    "settings.general.auto_fetch_incoming_description",
                )))
                .disabled(disabled),
            ),
        ])
    }
}
