//! Cli settings.

use super::*;

const LATEST_FEEDBACK_DURATION: std::time::Duration = std::time::Duration::from_secs(2);

pub(super) enum CliUpdateAction {
    Available {
        path: std::path::PathBuf,
    },
    Latest {
        path: std::path::PathBuf,
        until: Instant,
    },
}

impl CliUpdateAction {
    fn path(&self) -> &std::path::Path {
        match self {
            Self::Available { path } | Self::Latest { path, .. } => path,
        }
    }
}

impl ConduitApp {
    pub(in crate::app) fn set_engine_enabled(
        &mut self,
        provider: Provider,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.cli_installing() {
            return;
        }
        let previous = self.store.cli.clone();
        self.store.cli.set_enabled(provider, enabled);
        if !self.persist() {
            self.store.cli = previous;
        }
        cx.emit(crate::providers::ProviderCatalogChanged);
        cx.notify();
    }
    pub(in crate::app) fn choose_cli(
        &mut self,
        provider: Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.cli_installing() {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                conduit::i18n::format(
                    "settings.cli.select_executable_exe_or_cmd",
                    &[("arg0", provider.label().to_string())],
                )
                .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        match cli::validate_executable(&path) {
                            Ok(path) => this.set_cli_path(provider, Some(path), cx),
                            Err(error) => {
                                this.diagnostics.insert(
                                    provider,
                                    conduit::i18n::format(
                                        "settings.cli.path_not_saved",
                                        &[("error", format!("{:#}", error))],
                                    ),
                                );
                            }
                        }
                        cx.notify();
                    });
                }
            }
            Ok(Ok(None)) => {}
            other => {
                let _ = this.update(cx, |this, cx| {
                    this.diagnostics.insert(
                        provider,
                        conduit::i18n::format(
                            "settings.cli.failed_to_open_file",
                            &[("other", format!("{:?}", other))],
                        ),
                    );
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(in crate::app) fn set_cli_path(
        &mut self,
        provider: Provider,
        path: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.cli_installing() {
            return;
        }
        let previous = self.store.cli.clone();
        self.store.cli.set(provider, path);
        if self.persist() {
            self.connection_settings.cli_updates.remove(&provider);
            self.extensions_ui.invalidate();
            if provider == Provider::OpenCode {
                self.opencode_model_probes.clear();
                self.opencode_catalogs.clear();
            }
            if let Some(probe) = self.probes.remove(&provider) {
                probe.cancel();
            }
            self.probe_cli(provider, cx);
        } else {
            self.store.cli = previous;
            self.diagnostics.insert(
                provider,
                self.message.clone().unwrap_or_else(|| {
                    conduit::i18n::text_static("settings.cli.configuration_save_failed").into()
                }),
            );
        }
        cx.notify();
    }

    pub(in crate::app) fn probe_cli(&mut self, provider: Provider, cx: &mut Context<Self>) {
        self.start_cli_probe(provider, false, cx);
    }

    pub(in crate::app) fn check_cli_updates(&mut self, provider: Provider, cx: &mut Context<Self>) {
        self.start_cli_probe(provider, true, cx);
    }

    fn cli_update_action(&mut self, provider: Provider, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        if let Some(probe) = self.probes.get(&provider) {
            if !probe.is_cancelled() && (probe.checking_updates || probe.installing_update) {
                probe.cancel();
                self.diagnostics.insert(
                    provider,
                    if probe.installing_update {
                        conduit::i18n::text_static("settings.cli.stopping_the_update_process")
                            .into()
                    } else {
                        conduit::i18n::text_static("settings.cli.cancelling_update_check").into()
                    },
                );
                cx.notify();
            }
            return;
        }
        let resolved = cli::resolve(provider, &self.store.cli).ok();
        let action = self
            .connection_settings
            .cli_updates
            .get(&provider)
            .filter(|action| {
                resolved
                    .as_ref()
                    .is_some_and(|cli| cli.path == action.path())
            });
        match action {
            Some(CliUpdateAction::Available { .. }) => self.install_cli_update(provider, cx),
            Some(CliUpdateAction::Latest { until, .. }) if *until > Instant::now() => {}
            _ => self.check_cli_updates(provider, cx),
        }
    }

    pub(in crate::app) fn cli_installing(&self) -> bool {
        self.probes.values().any(|probe| probe.installing_update)
    }

    fn cli_update_busy(&self) -> bool {
        !self.session_control.runs.is_empty()
            || !self.session_control.namings.is_empty()
            || self.git.generation.is_some()
            || !self.opencode_model_probes.is_empty()
    }

    pub(in crate::app) fn install_cli_update(
        &mut self,
        provider: Provider,
        cx: &mut Context<Self>,
    ) {
        if self.closing
            || !self.writable
            || self.cli_installing()
            || self.probes.contains_key(&provider)
        {
            return;
        }
        if self.cli_update_busy() {
            self.diagnostics.insert(
                provider,
                conduit::i18n::text_static("settings.cli.wait_for_running_cli_tasks_to_finish")
                    .into(),
            );
            cx.notify();
            return;
        }
        match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => {
                self.probes
                    .insert(provider, VersionProbe::start_install(provider, cli.path));
                self.diagnostics.insert(
                    provider,
                    conduit::i18n::text_static(
                        "settings.cli.checking_version_and_detecting_installation_method",
                    )
                    .into(),
                );
            }
            Err(error) => {
                self.diagnostics.insert(
                    provider,
                    conduit::i18n::format(
                        "settings.cli.unable_to_update",
                        &[("error", format!("{:#}", error))],
                    ),
                );
            }
        }
        cx.notify();
    }

    fn start_cli_probe(
        &mut self,
        provider: Provider,
        checking_updates: bool,
        cx: &mut Context<Self>,
    ) {
        if self.closing || self.cli_installing() || self.probes.contains_key(&provider) {
            return;
        }
        match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => {
                let probe = if checking_updates {
                    VersionProbe::start_update_check(provider, cli.path)
                } else {
                    VersionProbe::start(provider, cli.path)
                };
                self.probes.insert(provider, probe);
                self.connection_settings.cli_updates.remove(&provider);
                self.diagnostics.insert(
                    provider,
                    if checking_updates {
                        conduit::i18n::text_static(
                            "settings.cli.reading_local_version_and_checking_the_latest",
                        )
                        .into()
                    } else {
                        conduit::i18n::text_static("settings.cli.running_version_up_to_10_seconds")
                            .into()
                    },
                );
            }
            Err(error) => {
                self.diagnostics.insert(
                    provider,
                    conduit::i18n::format(
                        "settings.cli.unable_to_detect_version",
                        &[("error", format!("{:#}", error))],
                    ),
                );
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn poll_cli_probes(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let previous = self.connection_settings.cli_updates.len();
        self.connection_settings.cli_updates.retain(
            |_, action| !matches!(action, CliUpdateAction::Latest { until, .. } if *until <= now),
        );
        if previous != self.connection_settings.cli_updates.len() {
            cx.notify();
        }
        let mut completed = vec![];
        for (provider, probe) in &self.probes {
            if !probe.is_cancelled()
                && let Some(progress) = &probe.progress
                && let Some(text) = progress.try_iter().last()
            {
                self.diagnostics.insert(*provider, text);
                cx.notify();
            }
            match probe.events.try_recv() {
                Ok(result) => completed.push((
                    *provider,
                    probe.path.clone(),
                    probe.checking_updates,
                    probe.installing_update,
                    probe.is_cancelled(),
                    result,
                )),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => completed.push((
                    *provider,
                    probe.path.clone(),
                    probe.checking_updates,
                    probe.installing_update,
                    probe.is_cancelled(),
                    Err(conduit::i18n::text_static(
                        "settings.cli.detection_thread_exited_run_detection_again",
                    )
                    .into()),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        for (provider, path, checking_updates, installing_update, cancelled, result) in completed {
            self.probes.remove(&provider);
            if (checking_updates || installing_update) && cancelled {
                self.diagnostics.insert(
                    provider,
                    if installing_update {
                        conduit::i18n::text_static(
                            "settings.cli.update_stopped_installation_may_be_incomplete_detect",
                        )
                        .into()
                    } else {
                        conduit::i18n::text_static("settings.cli.update_check_cancelled").into()
                    },
                );
                cx.notify();
                continue;
            }
            let diagnostic_level = if match &result {
                Err(_) => true,
                Ok(info) => info.update.as_ref().is_some_and(Result::is_err),
            } {
                NoticeLevel::Error
            } else {
                NoticeLevel::Success
            };
            let text = if cli::resolve(provider, &self.store.cli)
                .ok()
                .is_none_or(|cli| cli.path != path)
            {
                self.connection_settings.cli_updates.remove(&provider);
                conduit::i18n::text_static("settings.cli.configuration_changed_run_detection_again")
                    .into()
            } else {
                if checking_updates || installing_update {
                    if let Ok(info) = &result
                        && let Some(Ok(update)) = &info.update
                    {
                        let action = match update.status {
                            cli::updates::UpdateStatus::Available => {
                                CliUpdateAction::Available { path: path.clone() }
                            }
                            cli::updates::UpdateStatus::Current
                            | cli::updates::UpdateStatus::Ahead => CliUpdateAction::Latest {
                                path: path.clone(),
                                until: Instant::now() + LATEST_FEEDBACK_DURATION,
                            },
                        };
                        self.connection_settings
                            .cli_updates
                            .insert(provider, action);
                    } else if checking_updates {
                        self.connection_settings.cli_updates.remove(&provider);
                    }
                }
                match result {
                    Ok(info) => {
                        if installing_update {
                            self.extensions_ui.invalidate();
                            if provider == Provider::OpenCode {
                                self.opencode_model_probes.clear();
                                self.opencode_catalogs.clear();
                            }
                            conduit::i18n::format(
                                "settings.cli.update_complete_detection_refreshed",
                                &[("arg0", info.diagnostic().to_string())],
                            )
                        } else {
                            info.diagnostic()
                        }
                    }
                    Err(error) if cancelled => error,
                    Err(error) => conduit::i18n::format(
                        "settings.cli.failed",
                        &[
                            (
                                "arg0",
                                (if installing_update {
                                    conduit::i18n::text_static("settings.cli.update_now")
                                } else if checking_updates {
                                    conduit::i18n::text_static("settings.cli.check_for_updates")
                                } else {
                                    conduit::i18n::text_static("settings.cli.detect")
                                })
                                .to_string(),
                            ),
                            ("error", error.to_string()),
                        ],
                    ),
                }
            };
            self.notify_user(
                Notice::new(
                    NoticeSource::Settings,
                    diagnostic_level,
                    format!(
                        "{} CLI {}",
                        provider.label(),
                        if installing_update {
                            conduit::i18n::text_static("settings.cli.update_now")
                        } else if checking_updates {
                            conduit::i18n::text_static("settings.cli.update_check")
                        } else {
                            conduit::i18n::text_static("settings.cli.detect")
                        }
                    ),
                    text.clone(),
                )
                .target(NoticeTarget::Settings)
                .presentation(NoticePresentation::History),
                cx,
            );
            self.diagnostics.insert(provider, text);
            cx.notify();
        }
    }

    pub(super) fn settings_provider_group(
        &self,
        provider: Provider,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingGroup {
        let index = provider as usize;
        let enabled = self.store.cli.enabled(provider);
        let configured = self
            .store
            .cli
            .path(provider)
            .map(display_path)
            .unwrap_or_else(|| {
                conduit::i18n::text_static("settings.cli.automatically_locate_installed_cli").into()
            });
        let has_path = self.store.cli.path(provider).is_some();
        let (effective, resolved_path) = match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => (
                format!("{} · {}", cli.source.label(), display_path(&cli.path)),
                Some(cli.path),
            ),
            Err(error) => (
                conduit::i18n::format(
                    "settings.cli.unavailable",
                    &[("error", format!("{:#}", error))],
                ),
                None,
            ),
        };
        let available = resolved_path.is_some();
        let action = self
            .connection_settings
            .cli_updates
            .get(&provider)
            .filter(|action| {
                resolved_path
                    .as_ref()
                    .is_some_and(|path| path == action.path())
            });
        let install_available = matches!(action, Some(CliUpdateAction::Available { .. }));
        let latest = matches!(action, Some(CliUpdateAction::Latest { until, .. }) if *until > Instant::now());
        let installing = self
            .probes
            .get(&provider)
            .is_some_and(|probe| probe.installing_update);
        let any_installing = self.cli_installing();
        let update_busy = self.cli_update_busy();
        let disabled = disabled || any_installing;
        let probing = self.probes.contains_key(&provider);
        let checking_updates = self
            .probes
            .get(&provider)
            .is_some_and(|probe| probe.checking_updates);
        let cancelling = self
            .probes
            .get(&provider)
            .is_some_and(VersionProbe::is_cancelled);
        let diagnostic = self.diagnostics.get(&provider).cloned().unwrap_or_else(|| {
            conduit::i18n::text_static("settings.cli.check_for_updates_to_get_the_installed").into()
        });
        let self_writable = self.writable;
        let closing = self.closing;
        let expanded = self.connection_settings.path_help;
        let descriptor = provider.descriptor();
        let card = SettingItem::render(move |_, _, _| {
            let toggle = view.clone();
            let choose = view.clone();
            let clear = view.clone();
            let update = view.clone();
            let help = view.clone();
            v_flex()
                .w_full()
                .gap_4()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(rgb(palette().border))
                .bg(rgb(palette().surface))
                .child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .child(
                            div()
                                .size(px(40.))
                                .flex_shrink_0()
                                .rounded_lg()
                                .bg(rgb(palette().surface_strong))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(provider_icon(provider)),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(ui_text_size(16.))
                                        .font_semibold()
                                        .child(provider.label()),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .flex_wrap()
                                        .child(management_badge(
                                            if available {
                                                conduit::i18n::text_static("settings.cli.cli_found")
                                            } else {
                                                conduit::i18n::text_static(
                                                    "settings.cli.cli_not_found",
                                                )
                                            },
                                            available,
                                        ))
                                        .child(management_badge(
                                            descriptor.capabilities.protocol,
                                            false,
                                        )),
                                ),
                        )
                        .child(
                            Button::new(("engine-enabled", index))
                                .debug_selector(move || format!("engine-enabled-{index}"))
                                .small()
                                .outline()
                                .label(if enabled {
                                    conduit::i18n::text_static("settings.cli.enabled_disable")
                                } else {
                                    conduit::i18n::text_static("settings.cli.disabled_enable")
                                })
                                .disabled(disabled)
                                .on_click(move |_, _, cx| {
                                    toggle.update(cx, |app, cx| {
                                        app.set_engine_enabled(provider, !enabled, cx)
                                    })
                                }),
                        ),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .pt_3()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .flex_wrap()
                                .justify_between()
                                .child(div().text_size(ui_text_size(12.)).font_semibold().child(
                                    conduit::i18n::text_static("settings.cli.version_and_updates"),
                                ))
                                .child(
                                    h_flex().gap_2().flex_wrap().child(
                                        Button::new(("settings-check-updates", index))
                                            .debug_selector(move || {
                                                format!("settings-check-updates-{index}")
                                            })
                                            .small()
                                            .when(install_available || installing, |button| {
                                                button.primary()
                                            })
                                            .when(!install_available && !installing, |button| {
                                                button.outline()
                                            })
                                            .icon(if installing || install_available {
                                                FullIconName::Download
                                            } else if latest {
                                                FullIconName::Check
                                            } else {
                                                FullIconName::RefreshCw
                                            })
                                            .label(if cancelling {
                                                if installing {
                                                    conduit::i18n::text_static(
                                                        "settings.cli.stopping",
                                                    )
                                                } else {
                                                    conduit::i18n::text_static(
                                                        "settings.cli.cancelling",
                                                    )
                                                }
                                            } else if installing {
                                                conduit::i18n::text_static(
                                                    "settings.cli.stop_update",
                                                )
                                            } else if checking_updates {
                                                conduit::i18n::text_static(
                                                    "settings.cli.cancel_check",
                                                )
                                            } else if probing {
                                                conduit::i18n::text_static("settings.cli.detecting")
                                            } else if install_available {
                                                conduit::i18n::text_static(
                                                    "settings.cli.update_now",
                                                )
                                            } else if latest {
                                                conduit::i18n::text_static(
                                                    "settings.cli.up_to_date",
                                                )
                                            } else {
                                                conduit::i18n::text_static(
                                                    "settings.cli.check_for_updates",
                                                )
                                            })
                                            .disabled(
                                                closing
                                                    || cancelling
                                                    || latest
                                                    || (any_installing && !installing)
                                                    || (probing
                                                        && !checking_updates
                                                        && !installing)
                                                    || (!probing
                                                        && (!available
                                                            || (install_available
                                                                && (!self_writable
                                                                    || update_busy)))),
                                            )
                                            .on_click(move |_, _, cx| {
                                                update.update(cx, |app, cx| {
                                                    app.cli_update_action(provider, cx)
                                                })
                                            }),
                                    ),
                                ),
                        )
                        .when(update_busy && install_available && !installing, |body| {
                            body.child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(conduit::i18n::text_static(
                                        "settings.cli.wait_for_running_cli_tasks_to_finish",
                                    )),
                            )
                        })
                        .child(
                            div()
                                .debug_selector(move || format!("settings-cli-diagnostic-{index}"))
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().muted))
                                .child(diagnostic.clone()),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    Button::new(("settings-update-guide", index))
                                        .small()
                                        .ghost()
                                        .label(conduit::i18n::text_static(
                                            "settings.cli.official_release_notes",
                                        ))
                                        .on_click(move |_, _, cx| {
                                            cx.open_url(cli::updates::guide_url(provider))
                                        }),
                                )
                                .child(
                                    div()
                                        .text_size(ui_text_size(11.))
                                        .text_color(rgb(palette().faint))
                                        .child(conduit::i18n::format(
                                            "settings.cli.verified_version",
                                            &[("arg0", cli::tested_version(provider).to_string())],
                                        )),
                                ),
                        ),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .pt_3()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .font_semibold()
                                .child(conduit::i18n::text_static("settings.cli.executable_path")),
                        )
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .p_3()
                                .rounded_md()
                                .bg(rgb(palette().surface_strong))
                                .text_size(ui_text_size(12.))
                                .child(configured.clone()),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(if available {
                                    palette().muted
                                } else {
                                    palette().danger_text
                                }))
                                .child(effective.clone()),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    Button::new(("settings-choose", index))
                                        .small()
                                        .outline()
                                        .icon(FullIconName::FolderOpen)
                                        .label(conduit::i18n::text_static(
                                            "settings.cli.choose_file",
                                        ))
                                        .disabled(disabled)
                                        .on_click(move |_, window, cx| {
                                            choose.update(cx, |app, cx| {
                                                app.choose_cli(provider, window, cx)
                                            })
                                        }),
                                )
                                .when(has_path, |row| {
                                    row.child(
                                        Button::new(("settings-clear", index))
                                            .small()
                                            .ghost()
                                            .label(conduit::i18n::text_static(
                                                "settings.cli.restore_automatic_detection",
                                            ))
                                            .disabled(disabled)
                                            .on_click(move |_, _, cx| {
                                                clear.update(cx, |app, cx| {
                                                    app.set_cli_path(provider, None, cx)
                                                })
                                            }),
                                    )
                                })
                                .child(
                                    Button::new(("engine-guide", index))
                                        .small()
                                        .ghost()
                                        .label(conduit::i18n::text_static(
                                            "settings.cli.installation_and_sign_in_instructions",
                                        ))
                                        .on_click(move |_, _, cx| cx.open_url(descriptor.guide)),
                                ),
                        ),
                )
                .child(
                    Button::new("settings-path-help")
                        .small()
                        .ghost()
                        .justify_start()
                        .icon(if expanded {
                            FullIconName::ChevronUp
                        } else {
                            FullIconName::ChevronDown
                        })
                        .label(conduit::i18n::text_static(
                            "settings.cli.path_lookup_and_when_changes_apply",
                        ))
                        .on_click(move |_, _, cx| {
                            help.update(cx, |app, cx| {
                                app.connection_settings.path_help = !expanded;
                                cx.notify();
                            })
                        }),
                )
                .when(expanded, |card| {
                    card.child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::text_static(
                                "settings.cli.lookup_order_environment_variable_app_specified_path",
                            )),
                    )
                })
        });
        SettingGroup::new()
            .variant(GroupBoxVariant::Normal)
            .item(card)
    }
}
