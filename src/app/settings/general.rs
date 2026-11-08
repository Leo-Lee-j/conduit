//! General settings.

use super::*;

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

    pub(in crate::app) fn settings_session_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
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
        let mode_view = view.clone();
        let mode = SettingField::render(move |_, _, cx| {
            let auto_rename = mode_view.read(cx).store.ui.auto_rename;
            mode_view
                .read(cx)
                .naming_mode_picker(mode_view.clone(), disabled || !auto_rename)
        });
        let mut items = vec![
            SettingItem::new(
                conduit::i18n::text_static("settings.defaults.automatic_naming"),
                auto_rename,
            )
            .description(settings_description(conduit::i18n::text_static(
                "settings.general.manual_ai_renaming_remains_available_when_disabled",
            )))
            .disabled(disabled),
            SettingItem::new(conduit::i18n::text_static("settings.general.trigger"), mode)
                .disabled(disabled),
            self.generation_disclosure(true, view.clone()),
        ];
        if self.settings_ui.naming_expanded {
            items.extend([
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
            ]);
        }
        settings_page(
            conduit::i18n::text_static("settings.defaults.automatic_naming"),
            FullIconName::Pencil,
        )
        .group(
            settings_section(conduit::i18n::text_static(
                "settings.general.session_naming",
            ))
            .items(items)
            .footer(|_, _| {
                conduit::i18n::text_static(
                    "settings.general.naming_runs_in_the_background_failures_keep",
                )
            }),
        )
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
        let mut generation = vec![self.generation_disclosure(false, view.clone())];
        if self.settings_ui.git_expanded {
            generation.extend([
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
            ]);
        }
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
        let edit = SettingItem::render(move |_, _, _| {
            let view = view.clone();
            v_flex()
                .w_full()
                .gap_2()
                .when(!expanded, |row| {
                    row.child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(preview.clone()),
                    )
                })
                .child(
                    Button::new("git-prompt-expand")
                        .debug_selector(|| "git-prompt-expand".into())
                        .small()
                        .ghost()
                        .label(if expanded {
                            conduit::i18n::text_static("settings.general.hide_prompt")
                        } else {
                            conduit::i18n::text_static("settings.general.edit_prompt")
                        })
                        .on_click(move |_, _, cx| {
                            view.update(cx, |app, cx| {
                                app.settings_ui.prompt_expanded = !expanded;
                                cx.notify();
                            });
                        }),
                )
        })
        .keywords([
            conduit::i18n::text_static("settings.general.prompt"),
            "prompt",
        ]);
        let mut prompts = vec![edit];
        if expanded {
            prompts.extend([
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
            ]);
        }
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
            .items(generation),
            settings_section(conduit::i18n::text_static(
                "settings.general.commit_message_prompt",
            ))
            .items(prompts)
            .footer(|_, _| {
                conduit::i18n::text_static(
                    "settings.general.changes_save_automatically_and_apply_to_the",
                )
            }),
        ])
    }
}
