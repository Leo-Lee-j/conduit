//! Native extensions, scoped independently from connections and conversations.
use super::extension_catalog::{resource_badge, resource_button, resource_row};
use super::*;
use crate::extensions::management::{self, Filter};
use crate::extensions::{self, Catalog, Kind};

pub(in crate::app) struct ExtensionsUi {
    pub discovery: [super::extension_catalog::Browse; 2],
    pub engine: Provider,
    pub feedback: Option<String>,
    pub importing: bool,
    pub changing: bool,
    pub filters: [Filter; 2],
    pub connection_probe: Option<extensions::connection::ConnectionProbe>,
    pub connection_results: HashMap<String, String>,
    pub tool_reports: HashMap<String, extensions::connection::ToolReport>,
    auto_detection_started: bool,
    detection_queue: std::collections::VecDeque<extensions::Entry>,
    pub workspace: Option<PathBuf>,
    pub initialized: bool,
    pub mcp: bool,
    pub searches: [Entity<InputState>; 2],
    pub lists: [gpui_kit::ListState; 2],
    pub sources_expanded: bool,
    pub expanded: HashSet<String>,
    pub probe: Option<extensions::Probe>,
    pub catalogs: HashMap<extensions::Context, Catalog>,
    pub context: Option<extensions::Context>,
}
impl ExtensionsUi {
    pub fn new(skills: Entity<InputState>, mcp: Entity<InputState>, engine: Provider) -> Self {
        Self {
            discovery: [
                super::extension_catalog::Browse::new(false),
                super::extension_catalog::Browse::new(true),
            ],
            engine,
            feedback: None,
            importing: false,
            changing: false,
            filters: [Filter::All; 2],
            connection_probe: None,
            connection_results: HashMap::new(),
            tool_reports: HashMap::new(),
            auto_detection_started: false,
            detection_queue: Default::default(),
            workspace: None,
            initialized: false,
            mcp: false,
            searches: [skills, mcp],
            lists: new_lists(),
            sources_expanded: false,
            expanded: HashSet::new(),
            probe: None,
            catalogs: HashMap::new(),
            context: None,
        }
    }
    pub fn cancel_mcp_detection(&mut self) {
        self.connection_probe = None;
        self.detection_queue.clear();
        self.auto_detection_started = false;
    }
    pub fn invalidate(&mut self) {
        self.probe = None;
        self.cancel_mcp_detection();
        self.connection_results.clear();
        self.tool_reports.clear();
        self.catalogs.clear();
        self.context = None;
    }
}

impl ConduitApp {
    fn test_mcp_connection(&mut self, entry: extensions::Entry, cx: &mut Context<Self>) {
        if self.closing || self.extensions_ui.connection_probe.is_some() {
            return;
        }
        if let Some(context) = self.extensions_ui.context.clone() {
            self.extensions_ui
                .detection_queue
                .retain(|queued| queued.id() != entry.id());
            self.extensions_ui.tool_reports.remove(&entry.id());
            self.extensions_ui.connection_results.insert(
                entry.id(),
                conduit::i18n::text_static(
                    "settings.extensions.initializing_and_reading_the_tool_list",
                )
                .into(),
            );
            self.extensions_ui.connection_probe = Some(
                extensions::connection::ConnectionProbe::start(entry, context),
            );
            self.extensions_ui.lists = new_lists();
            cx.notify();
        }
    }
    fn extension_actions(&self, narrow: bool, view: Entity<Self>) -> AnyElement {
        let kind = if self.extensions_ui.mcp {
            Kind::Mcp
        } else {
            Kind::Skill
        };
        let target = extensions::edit::destination(
            self.extensions_ui.engine,
            self.extensions_ui.workspace.as_deref(),
            kind,
        );
        let edit_view = view.clone();
        let disabled = matches!(
            self.extensions_ui.engine,
            Provider::Qoder | Provider::Gemini
        ) || !self.writable
            || self.closing
            || self.extensions_ui.importing;
        let import_view = view;
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                resource_button("extensions-add")
                    .debug_selector(|| "extensions-add".into())
                    .primary()
                    .small()
                    .icon(FullIconName::Plus)
                    .accessibility_label(if kind == Kind::Mcp {
                        conduit::i18n::text_static("settings.config_editor.add_mcp_service")
                    } else {
                        conduit::i18n::text_static("settings.config_editor.create_skill")
                    })
                    .tooltip(if kind == Kind::Mcp {
                        conduit::i18n::text_static("settings.config_editor.add_mcp_service")
                    } else {
                        conduit::i18n::text_static("settings.config_editor.create_skill")
                    })
                    .when(!narrow, |button| {
                        button.label(if kind == Kind::Mcp {
                            conduit::i18n::text_static("settings.config_editor.add_mcp_service")
                        } else {
                            conduit::i18n::text_static("settings.config_editor.create_skill")
                        })
                    })
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        edit_view.update(cx, |app, cx| {
                            if kind == Kind::Mcp {
                                app.open_mcp_form(window, cx);
                            } else {
                                app.open_native_config(target.clone(), kind, true, window, cx);
                            }
                        })
                    }),
            )
            .when(kind == Kind::Skill, |row| {
                row.child(
                    resource_button("extensions-import")
                        .outline()
                        .small()
                        .icon(FullIconName::FolderOpen)
                        .accessibility_label(conduit::i18n::text_static(
                            "settings.extensions.import_from_folder",
                        ))
                        .tooltip(if self.extensions_ui.importing {
                            conduit::i18n::text_static("settings.extensions.importing")
                        } else {
                            conduit::i18n::text_static("settings.extensions.import_from_folder")
                        })
                        .when(!narrow, |button| {
                            button.label(if self.extensions_ui.importing {
                                conduit::i18n::text_static("settings.extensions.importing")
                            } else {
                                conduit::i18n::text_static("settings.extensions.import_from_folder")
                            })
                        })
                        .disabled(disabled)
                        .on_click(move |_, window, cx| {
                            import_view.update(cx, |app, cx| app.import_skill(window, cx))
                        }),
                )
            })
            .into_any_element()
    }

    fn import_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.extensions_ui.engine,
            Provider::Qoder | Provider::Gemini
        ) || !self.writable
            || self.closing
            || self.extensions_ui.importing
        {
            return;
        }
        let destination = extensions::edit::destination(
            self.extensions_ui.engine,
            self.extensions_ui.workspace.as_deref(),
            Kind::Skill,
        );
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(
                conduit::i18n::text_static(
                    "settings.extensions.select_a_skill_folder_containing_skill_md",
                )
                .into(),
            ),
        });
        self.extensions_ui.importing = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = match paths.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                    let source = paths[0].clone();
                    Some(cx.background_executor().spawn(async move { extensions::edit::import_skill(&source, &destination) }).await)
                }
                Ok(Ok(_)) => None,
                _ => Some(Err(anyhow::anyhow!(conduit::i18n::text("settings.extensions.unable_to_open_folder_picker")))),
            };
            let _ = this.update_in(cx, |app, window, cx| {
                app.extensions_ui.importing = false;
                if let Some(result) = result {
                    let level = if result.is_ok() { NoticeLevel::Success } else { NoticeLevel::Error };
                    let message = match result {
                        Ok(path) => conduit::i18n::format("settings.extensions.imported_loaded_when_the_next_session_starts", &[("arg0", display_path(&path))]),
                        Err(error) => error.to_string(),
                    };
                    app.notify_user(Notice::new(NoticeSource::Settings, level, conduit::i18n::text_static("settings.extensions.skill_import_result"), message.clone())
                        .target(NoticeTarget::Settings).presentation(NoticePresentation::History), cx);
                    app.extensions_ui.feedback = Some(message);
                    app.extensions_ui.invalidate();
                    app.ensure_extensions(window, cx, true);
                }
                cx.notify();
            });
        }).detach();
    }

    pub(in crate::app) fn ensure_extensions(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
        refresh: bool,
    ) {
        if self.closing {
            return;
        }
        if !self.extensions_ui.initialized {
            self.extensions_ui.workspace = self
                .selected_workspace
                .and_then(|i| self.store.workspaces.get(i))
                .map(|w| w.path.clone());
            self.extensions_ui.initialized = true;
        }
        let context = extensions::Context {
            engine: self.extensions_ui.engine,
            workspace: self.extensions_ui.workspace.clone(),
            cli_path: cli::resolve(self.extensions_ui.engine, &self.store.cli)
                .ok()
                .map(|c| c.path),
        };
        if refresh || self.extensions_ui.context.as_ref() != Some(&context) {
            self.extensions_ui.cancel_mcp_detection();
            self.extensions_ui.connection_results.clear();
            self.extensions_ui.tool_reports.clear();
            self.extensions_ui.expanded.clear();
            self.extensions_ui.lists = new_lists();
            for index in 0..2 {
                self.refresh_catalog_target(index, cx);
            }
        }
        self.extensions_ui.context = Some(context.clone());
        if !refresh
            && self
                .extensions_ui
                .probe
                .as_ref()
                .is_some_and(|p| p.context == context)
        {
            return;
        }
        self.extensions_ui.probe = None;
        if !refresh
            && self
                .extensions_ui
                .catalogs
                .get(&context)
                .and_then(|c| c.updated)
                .is_some_and(|t| t.elapsed().is_ok_and(|d| d < Duration::from_secs(60)))
        {
            return;
        }
        self.extensions_ui.probe = Some(extensions::Probe::start(context));
        cx.notify();
    }

    pub(in crate::app) fn poll_extensions(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || !self.settings_ui.open || !matches!(self.settings_ui.page, 4 | 9) {
            for state in &mut self.extensions_ui.discovery {
                state.cancel();
            }
            self.extensions_ui.probe = None;
            self.extensions_ui.cancel_mcp_detection();
            return;
        }
        if self.settings_ui.page != 9 {
            self.extensions_ui.cancel_mcp_detection();
        }
        // Skills and MCP keep their lists, but a hidden discovery page must not open a late dialog.
        let inactive = usize::from(self.settings_ui.page == 4);
        self.extensions_ui.discovery[inactive].cancel();
        let connection = self
            .extensions_ui
            .connection_probe
            .as_ref()
            .and_then(|probe| match probe.events.try_recv() {
                Ok(result) => Some((probe.id.clone(), result)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                    probe.id.clone(),
                    Err(conduit::i18n::text_static(
                        "settings.extensions.test_thread_exited_please_retry",
                    )
                    .into()),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
        if let Some((id, result)) = connection {
            self.extensions_ui.connection_probe = None;
            let level = if result.is_ok() {
                NoticeLevel::Success
            } else {
                NoticeLevel::Error
            };
            let message = match result {
                Ok(report) => {
                    let message = report.summary();
                    self.extensions_ui.tool_reports.insert(id.clone(), report);
                    message
                }
                Err(error) => {
                    self.extensions_ui.tool_reports.remove(&id);
                    conduit::i18n::format(
                        "settings.extensions.connection_incomplete",
                        &[("error", error.to_string())],
                    )
                }
            };
            self.notify_user(
                Notice::new(
                    NoticeSource::Settings,
                    level,
                    conduit::i18n::text_static("settings.extensions.mcp_connection_test"),
                    message.clone(),
                )
                .target(NoticeTarget::Settings)
                .presentation(NoticePresentation::History),
                cx,
            );
            self.extensions_ui.connection_results.insert(id, message);
            self.extensions_ui.lists = new_lists();
            cx.notify();
        }
        let result = self
            .extensions_ui
            .probe
            .as_ref()
            .and_then(|p| match p.events.try_recv() {
                Ok(catalog) => Some((p.context.clone(), catalog)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                    p.context.clone(),
                    Catalog {
                        errors: vec![
                            conduit::i18n::text_static(
                                "settings.extensions.scan_thread_exited_refresh_to_retry",
                            )
                            .into(),
                        ],
                        ..Default::default()
                    },
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
        if let Some((context, catalog)) = result {
            self.extensions_ui.probe = None;
            if self.extensions_ui.context.as_ref() == Some(&context) {
                self.extensions_ui.catalogs.insert(context, catalog);
                self.extensions_ui.lists = new_lists();
            }
            cx.notify();
        }
        self.detect_mcp_tools(cx);
    }

    fn detect_mcp_tools(&mut self, cx: &mut Context<Self>) {
        if self.closing
            || !self.settings_ui.open
            || self.settings_ui.page != 9
            || self.extensions_ui.probe.is_some()
        {
            return;
        }
        if !self.extensions_ui.auto_detection_started {
            let Some(catalog) = self
                .extensions_ui
                .context
                .as_ref()
                .and_then(|context| self.extensions_ui.catalogs.get(context))
            else {
                return;
            };
            self.extensions_ui.detection_queue = catalog
                .entries
                .iter()
                .filter(|entry| {
                    entry.kind == Kind::Mcp
                        && !entry.disabled
                        && !catalog.manual_detection.contains(&entry.id())
                })
                .cloned()
                .collect();
            self.extensions_ui.auto_detection_started = true;
            self.extensions_ui.tool_reports.clear();
            self.extensions_ui.connection_results.clear();
            for id in &catalog.manual_detection {
                self.extensions_ui
                    .connection_results
                    .insert(id.clone(), extensions::catalog::text("manual_detection"));
            }
            for entry in &self.extensions_ui.detection_queue {
                self.extensions_ui.connection_results.insert(
                    entry.id(),
                    conduit::i18n::text_static("settings.extensions.waiting_to_detect_tools")
                        .into(),
                );
            }
            self.extensions_ui.lists = new_lists();
            cx.notify();
        }
        if self.extensions_ui.connection_probe.is_none()
            && let Some(entry) = self.extensions_ui.detection_queue.pop_front()
        {
            self.test_mcp_connection(entry, cx);
        }
    }

    pub(super) fn installed_extensions_content(
        &self,
        _window: &Window,
        available_width: f32,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let scale = self.store.ui.ui_font_size / DEFAULT_UI_FONT_SIZE;
        let narrow = available_width < 640. * scale;
        let mut controls = h_flex()
            .debug_selector(|| "extension-filter-bar".into())
            .w_full()
            .min_w_0()
            .gap_2()
            .flex_wrap();
        let mut choices = vec![(
            None,
            conduit::i18n::text_static("settings.extensions.global").to_owned(),
        )];
        choices.extend(
            self.store
                .workspaces
                .iter()
                .map(|w| (Some(w.path.clone()), w.name())),
        );
        let current = self
            .extensions_ui
            .workspace
            .as_ref()
            .map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| display_path(path))
            })
            .unwrap_or_else(|| conduit::i18n::text_static("settings.extensions.global").into());
        let scope_view = view.clone();
        controls = controls.child(
            settings_dropdown("extensions-scope")
                .debug_selector(|| "extensions-scope".into())
                .when_some(self.extensions_ui.workspace.as_ref(), |button, path| {
                    button.tooltip(display_path(path))
                })
                .accessibility_label(conduit::i18n::format(
                    "settings.extensions.scope",
                    &[("current", current.to_string())],
                ))
                .max_w(px(240.))
                .child(div().truncate().child(conduit::i18n::format(
                    "settings.extensions.scope",
                    &[("current", current.to_string())],
                )))
                .dropdown_menu(move |mut menu, _, _| {
                    for (path, name) in &choices {
                        let path = path.clone();
                        let view = scope_view.clone();
                        menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                            move |_, window, cx| {
                                view.update(cx, |app, cx| {
                                    app.extensions_ui.workspace = path.clone();
                                    app.extensions_ui.feedback = None;
                                    app.ensure_extensions(window, cx, false);
                                    cx.notify();
                                });
                            },
                        ));
                    }
                    menu
                }),
        );
        let mcp = self.extensions_ui.mcp;
        let selected = usize::from(mcp);
        let refresh = view.clone();
        let cancel = view.clone();
        let probing = self.extensions_ui.probe.is_some();
        let query = self.extensions_ui.searches[selected]
            .read(cx)
            .value()
            .to_lowercase();
        let catalog = self
            .extensions_ui
            .context
            .as_ref()
            .and_then(|context| self.extensions_ui.catalogs.get(context));
        let kind = if mcp { Kind::Mcp } else { Kind::Skill };
        let mut rows: Vec<ExtensionRow> = Vec::new();
        let mut count = 0;
        let total = catalog.map_or(0, |c| c.entries.iter().filter(|e| e.kind == kind).count());
        let duplicates = catalog
            .map(|c| management::duplicates(c, kind))
            .unwrap_or_default();
        let filter = self.extensions_ui.filters[selected];
        if let Some(catalog) = catalog {
            for entry in catalog.entries.iter().filter(|entry| entry.kind == kind) {
                if !filter.matches(entry, &duplicates, &query) {
                    continue;
                }
                count += 1;
                rows.push(ExtensionRow::Entry(
                    entry.clone(),
                    duplicates.contains(&entry.name),
                ));
            }
            for error in &catalog.errors {
                rows.push(ExtensionRow::Message(error.clone(), true));
            }
        }
        if count == 0 {
            rows.push(ExtensionRow::Empty(
                kind,
                if probing {
                    conduit::i18n::text_static("settings.extensions.reading_local_definitions")
                } else if !query.is_empty() || filter != Filter::All {
                    conduit::i18n::text_static("settings.extensions.no_matching_entries")
                } else if catalog.is_none() {
                    conduit::i18n::text_static(
                        "settings.extensions.not_loaded_or_reading_cancelled_refresh_to",
                    )
                } else {
                    conduit::i18n::text_static(
                        "settings.extensions.no_entries_found_in_the_scanned_sources",
                    )
                }
                .into(),
            ));
        }
        if let Some(catalog) = catalog {
            let mut sources = catalog.sources.clone();
            sources.sort();
            sources.dedup();
            rows.push(ExtensionRow::ScanScope(sources));
        }
        let list_state = self.extensions_ui.lists[selected].clone();
        if list_state.item_count() != rows.len() {
            list_state.reset(rows.len());
        }
        let expanded = self.extensions_ui.expanded.clone();
        let list_view = view.clone();
        let rows = list(list_state.clone(), move |index, _, cx| {
            render_extension_row(
                &rows[index],
                index,
                narrow,
                &expanded,
                list_view.clone(),
                cx,
            )
        })
        .size_full();
        let info = catalog
            .and_then(|c| c.updated)
            .map(|time| {
                conduit::i18n::format(
                    "settings.extensions.entries_refreshed_at",
                    &[
                        ("count", format!("{}", count)),
                        ("total", format!("{}", total)),
                        (
                            "arg0",
                            format!("{}", DateTime::<Local>::from(time).format("%H:%M:%S")),
                        ),
                    ],
                )
            })
            .unwrap_or_else(|| {
                conduit::i18n::format(
                    "settings.extensions.entries",
                    &[
                        ("count", format!("{}", count)),
                        ("total", format!("{}", total)),
                    ],
                )
            });
        let filter_view = view.clone();
        let actions = self.extension_actions(narrow, view.clone());
        let search = || {
            div().flex_1().min_w(px(120.)).max_w(px(480.)).child(
                settings_input(&self.extensions_ui.searches[selected])
                    .prefix(FullIconName::Search)
                    .cleanable(true)
                    .aria_label(conduit::i18n::text_static(
                        "settings.extensions.search_extension_names_descriptions_or_sources",
                    )),
            )
        };
        let toolbar = controls
            .when(!narrow, |row| row.child(search()))
            .child(
                settings_dropdown("extensions-filter-trigger")
                    .debug_selector(|| "extensions-filter-trigger".into())
                    .label(filter.label())
                    .dropdown_menu(move |mut menu, _, _| {
                        for option in Filter::ALL {
                            let view = filter_view.clone();
                            menu = menu.item(PopupMenuItem::new(option.label()).on_click(
                                move |_, _, cx| {
                                    view.update(cx, |app, cx| {
                                        app.extensions_ui.filters[selected] = option;
                                        app.extensions_ui.lists = new_lists();
                                        cx.notify();
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            )
            .child(
                resource_button("extensions-refresh")
                    .debug_selector(|| "extensions-refresh".into())
                    .small()
                    .outline()
                    .icon(FullIconName::RefreshCw)
                    .accessibility_label(if probing {
                        conduit::i18n::text_static("settings.connections.reading")
                    } else {
                        conduit::i18n::text_static("model_picker.refresh")
                    })
                    .tooltip(if probing {
                        conduit::i18n::text_static("settings.connections.reading")
                    } else {
                        conduit::i18n::text_static("model_picker.refresh")
                    })
                    .when(!narrow, |button| {
                        button.label(if probing {
                            conduit::i18n::text_static("settings.connections.reading")
                        } else {
                            conduit::i18n::text_static("model_picker.refresh")
                        })
                    })
                    .disabled(probing || self.closing)
                    .on_click(move |_, window, cx| {
                        refresh.update(cx, |app, cx| app.ensure_extensions(window, cx, true))
                    }),
            )
            .when(probing, |row| {
                row.child(
                    resource_button("extensions-cancel")
                        .small()
                        .ghost()
                        .label(conduit::i18n::text_static("git_branches.cancel"))
                        .on_click(move |_, _, cx| {
                            cancel.update(cx, |app, cx| {
                                app.extensions_ui.probe = None;
                                cx.notify();
                            })
                        }),
                )
            })
            .child(actions);
        let toolbar = if narrow {
            v_flex()
                .w_full()
                .gap_2()
                .child(toolbar)
                .child(search())
                .into_any_element()
        } else {
            toolbar.into_any_element()
        };
        v_flex()
            .size_full()
            .min_w_0()
            .text_color(rgb(palette().text))
            .child(
                v_flex()
                    .w_full()
                    .flex_shrink_0()
                    .px(px(settings_content_inset()))
                    .py_2()
                    .gap_1()
                    .child(toolbar)
                    .child(settings_description(conduit::i18n::text_static(if mcp {
                        "settings.extensions.connection_session_hint"
                    } else if self.extensions_ui.workspace.is_some() {
                        "settings.extensions.workspace_scope_hint"
                    } else {
                        "settings.extensions.global_scope_hint"
                    })))
                    .when(
                        matches!(
                            self.extensions_ui.engine,
                            Provider::Qoder | Provider::Gemini
                        ),
                        |body| {
                            body.child(settings_description(conduit::i18n::text_static(
                                "catalog.read_only",
                            )))
                        },
                    )
                    .when_some(self.extensions_ui.feedback.clone(), |body, message| {
                        body.child(div().text_size(ui_text_size(12.)).child(message))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .px(px(settings_content_inset()))
                    .pb_4()
                    .child(
                        v_flex()
                            .size_full()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(rgb(palette().border))
                            .overflow_hidden()
                            .child(
                                h_flex()
                                    .w_full()
                                    .flex_shrink_0()
                                    .px_3()
                                    .py_2()
                                    .gap_2()
                                    .flex_wrap()
                                    .justify_between()
                                    .bg(rgb(palette().surface_strong))
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().muted))
                                    .child(if mcp {
                                        conduit::i18n::text_static(
                                            "settings.extensions.service_connection_address",
                                        )
                                        .to_owned()
                                    } else {
                                        conduit::i18n::text_static(
                                            "settings.extensions.skill_description",
                                        )
                                        .to_owned()
                                    })
                                    .child(div().child(info)),
                            )
                            .child(
                                div()
                                    .id("extensions-list")
                                    .debug_selector(|| "extensions-list".into())
                                    .flex_1()
                                    .min_h_0()
                                    .w_full()
                                    .child(rows)
                                    .vertical_scrollbar(&list_state),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn new_lists() -> [gpui_kit::ListState; 2] {
    std::array::from_fn(|_| gpui_kit::ListState::new(0, ListAlignment::Top, px(100.)))
}

fn show_mcp_tools(
    name: String,
    report: extensions::connection::ToolReport,
    window: &mut Window,
    cx: &mut App,
) {
    let names = Arc::new(report.names);
    let list_state = gpui_kit::ListState::new(names.len(), ListAlignment::Top, px(34.));
    window.open_dialog(cx, move |dialog, window, _| {
        let names = names.clone();
        let list_state = list_state.clone();
        let height = (window.viewport_size().height - px(220.))
            .clamp(px(80.), px(320.))
            .min(px(names.len() as f32 * 34.));
        dialog
            .title(conduit::i18n::format(
                "settings.extensions.tools_title",
                &[("name", name.clone())],
            ))
            .w((window.viewport_size().width - px(48.)).clamp(px(0.), px(520.)))
            .content(move |body, _, _| {
                let names = names.clone();
                body.child(
                    v_flex()
                        .gap_3()
                        .child(settings_description(conduit::i18n::text_static(
                            "settings.extensions.tools_snapshot_hint",
                        )))
                        .when(names.is_empty(), |body| {
                            body.child(conduit::i18n::text_static(
                                "settings.extensions.no_tools_available",
                            ))
                        })
                        .when(!names.is_empty(), |body| {
                            body.child(
                                div()
                                    .id("mcp-tools-list")
                                    .debug_selector(|| "mcp-tools-list".into())
                                    .h(height)
                                    .w_full()
                                    .child(
                                        list(list_state.clone(), move |index, _, _| {
                                            div()
                                                .px_2()
                                                .py_2()
                                                .text_size(ui_text_size(14.))
                                                .child(names[index].clone())
                                                .into_any_element()
                                        })
                                        .size_full(),
                                    )
                                    .vertical_scrollbar(&list_state),
                            )
                        })
                        .child(
                            h_flex().justify_end().child(
                                resource_button("mcp-tools-close")
                                    .outline()
                                    .label(conduit::i18n::text_static(
                                        "settings.extensions.close_tools",
                                    ))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            ),
                        ),
                )
            })
    });
}

enum ExtensionRow {
    Entry(extensions::Entry, bool),
    Empty(Kind, String),
    Message(String, bool),
    ScanScope(Vec<PathBuf>),
}
fn render_extension_row(
    row: &ExtensionRow,
    index: usize,
    narrow: bool,
    expanded: &HashSet<String>,
    view: Entity<ConduitApp>,
    cx: &App,
) -> AnyElement {
    let disabled = !view.read(cx).writable || view.read(cx).closing;
    match row {
        ExtensionRow::Entry(entry, duplicate) => {
            let mcp = entry.kind == Kind::Mcp;
            let id = entry.id();
            let expanded = expanded.contains(&id);
            let toggle_view = view.clone();
            let toggle_id = id.clone();
            let copy_path = display_path(&entry.path);
            let edit_path = entry.path.clone();
            let reveal_path = entry.path.clone();
            let kind = entry.kind;
            let edit_view = view.clone();
            let managed = entry.managed();
            let toggle_supported = !mcp
                || view
                    .read(cx)
                    .extensions_ui
                    .engine
                    .descriptor()
                    .capabilities
                    .mcp_toggle;
            let state_view = view.clone();
            let state_entry = entry.clone();
            let ui = &view.read(cx).extensions_ui;
            let report = ui.tool_reports.get(&id).cloned();
            let probing = ui.connection_probe.is_some();
            let this_probe = ui
                .connection_probe
                .as_ref()
                .is_some_and(|probe| probe.id == id);
            let queued = ui.detection_queue.iter().any(|entry| entry.id() == id);
            let connection_result = ui.connection_results.get(&id).cloned();
            let manual_detection = connection_result.as_deref()
                == Some(extensions::catalog::text("manual_detection").as_str());
            let incomplete = mcp
                && !entry.disabled
                && report.is_none()
                && !this_probe
                && !queued
                && !manual_detection
                && connection_result.is_some();
            let entry_status = if entry.disabled {
                conduit::i18n::text_static("settings.extensions.not_tested")
            } else if report.is_some() {
                conduit::i18n::text_static("settings.extensions.connected")
            } else if this_probe {
                conduit::i18n::text_static("settings.extensions.detecting")
            } else if queued {
                conduit::i18n::text_static("settings.extensions.awaiting_detection")
            } else if incomplete {
                conduit::i18n::text_static("settings.extensions.detection_incomplete")
            } else {
                conduit::i18n::text_static("settings.extensions.not_tested")
            };
            let status_color = if entry.disabled {
                palette().muted
            } else if incomplete {
                palette().warning_text
            } else if mcp && report.is_some() {
                palette().activity_completed
            } else if !mcp || this_probe || queued {
                palette().activity_running
            } else {
                palette().muted
            };
            let status_badge = || {
                resource_badge(entry_status, false)
                    .text_color(rgb(status_color))
                    .bg(gpui_kit::rgba((status_color << 8) | 0x12))
            };
            let tool_count = report
                .clone()
                .filter(|_| mcp && !entry.disabled)
                .map(|report| {
                    let name = entry.name.clone();
                    let tool_label = conduit::i18n::format(
                        "settings.extensions.available_tools",
                        &[("arg0", report.count().to_string())],
                    );
                    resource_button(("mcp-tool-count", index))
                        .debug_selector(move || format!("mcp-tool-count-{index}"))
                        .small()
                        .ghost()
                        .self_start()
                        .accessibility_label(format!("{} · {tool_label}", entry.name))
                        .label(tool_label)
                        .icon(FullIconName::ChevronRight)
                        .tooltip(conduit::i18n::text_static("settings.extensions.view_tools"))
                        .on_click(move |_, window, cx| {
                            show_mcp_tools(name.clone(), report.clone(), window, cx);
                        })
                });
            let test_entry = entry.clone();
            let test_view = view.clone();
            let cancel_view = view.clone();
            let summary = if mcp {
                format!("{} · {}", entry.transport, entry.target)
            } else {
                entry.description.replace('\n', " ")
            };
            let summary = format!("{} · {}", conduit::i18n::source(&entry.scope), summary);
            let summary_hint = summary.clone();
            resource_row()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_start()
                        .child(
                            div().w(px(20.)).h(px(32.)).flex_shrink_0()
                                .flex().items_center().justify_center()
                                .child(Icon::new(if mcp { FullIconName::Server } else { FullIconName::Blocks })
                                    .small().text_color(rgb(palette().muted))),
                        )
                        .child(
                            v_flex().flex_1().min_w_0().gap_1()
                                .child(
                                    h_flex().w_full().gap_2().flex_wrap()
                                        .child(div().id(("extension-name", index))
                                            .debug_selector(move || format!("extension-name-{index}"))
                                            .min_w_0().truncate().font_semibold().text_size(ui_text_size(14.))
                                            .tooltip({ let name = entry.name.clone(); move |window, cx| {
                                                gpui_kit::component::tooltip::Tooltip::new(name.clone()).build(window, cx)
                                            }})
                                            .child(entry.name.clone()))
                                        .child(resource_badge(
                                            conduit::i18n::text_static(if entry.disabled {
                                                "extensions.configuration_disabled"
                                            } else if mcp {
                                                "settings.extensions.configuration_enabled"
                                            } else { "settings.plugins.enabled" }), !entry.disabled))
                                        .when(*duplicate, |row| row.child(resource_badge(
                                            conduit::i18n::text_static("settings.extensions.duplicate_name"), false))),
                                )
                                .child(div().id(("extension-summary", index))
                                    .truncate().text_size(ui_text_size(12.)).text_color(rgb(palette().muted))
                                    .tooltip(move |window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(summary_hint.clone()).build(window, cx)
                                    })
                                    .child(summary)),
                        )
                        .child(
                            h_flex().gap_1().flex_shrink_0().flex_wrap()
                                .child(resource_button(("extension-detail", index))
                                    .debug_selector(move || format!("extension-detail-{index}"))
                                    .small().ghost().selected(expanded)
                                    .icon(if expanded { FullIconName::ChevronUp } else { FullIconName::ChevronDown })
                                    .accessibility_label(format!("{} · {}", entry.name,
                                        conduit::i18n::text_static("settings.extensions.details")))
                                    .tooltip(conduit::i18n::text_static("settings.extensions.details"))
                                    .when(!narrow, |button| button.label(conduit::i18n::text_static("settings.extensions.details")))
                                    .on_click(move |_, _, cx| {
                                        toggle_view.update(cx, |app, cx| {
                                            app.extensions_ui.expanded.clear();
                                            if !expanded { app.extensions_ui.expanded.insert(toggle_id.clone()); }
                                            let selected = usize::from(app.extensions_ui.mcp);
                                            let list = &app.extensions_ui.lists[selected];
                                            list.reset(list.item_count());
                                            cx.notify();
                                        });
                                    }))
                                .when(!managed && toggle_supported, |row| row.child(
                                    resource_button(("extension-toggle", index)).small().ghost()
                                        .label(conduit::i18n::text_static(if entry.disabled {
                                            "settings.extensions.restore"
                                        } else { "settings.extensions.disable" }))
                                        .disabled(disabled || view.read(cx).extensions_ui.changing)
                                        .on_click(move |_, window, cx| state_view.update(cx, |app, cx| {
                                            app.open_extension_toggle(state_entry.clone(), window, cx)
                                        })),
                                ))
                                .when(managed, |row| row.child(resource_badge(
                                    conduit::i18n::text_static("settings.extensions.plugin_managed"), false))),
                        ),
                )
                .when(mcp, |body| body.child(
                    h_flex().w_full().gap_2().flex_wrap()
                        .child(status_badge())
                        .when_some(tool_count, |row, count| row.child(count))
                        .child(resource_button(("mcp-test", index))
                            .debug_selector(move || format!("mcp-test-{index}"))
                            .small().outline().icon(FullIconName::RefreshCw)
                            .label(conduit::i18n::text_static(if this_probe {
                                "settings.extensions.testing"
                            } else if report.is_some() || incomplete {
                                "settings.extensions.retest"
                            } else { "settings.extensions.test_connection" }))
                            .tooltip(conduit::i18n::text_static("settings.extensions.testing_starts_the_command_or_connects_to"))
                            .disabled(probing || entry.disabled || view.read(cx).closing)
                            .on_click(move |_, _, cx| test_view.update(cx, |app, cx| {
                                app.test_mcp_connection(test_entry.clone(), cx)
                            })))
                        .when(manual_detection && !entry.disabled, |row| row.child(
                            div().min_w_0().text_size(ui_text_size(12.)).text_color(rgb(palette().muted))
                                .child(connection_result.clone().unwrap_or_default()),
                        ))
                        .when(this_probe, |row| row.child(
                            resource_button(("mcp-cancel", index)).small().ghost()
                                .label(conduit::i18n::text_static("git_branches.cancel"))
                                .on_click(move |_, _, cx| cancel_view.update(cx, |app, cx| {
                                    if let Some(probe) = app.extensions_ui.connection_probe.take() {
                                        app.extensions_ui.connection_results.insert(probe.id.clone(),
                                            conduit::i18n::text_static("settings.extensions.test_cancelled").into());
                                    }
                                    app.extensions_ui.lists = new_lists();
                                    cx.notify();
                                })),
                        )),
                ))
                .when(incomplete, |body| body.child(
                    div().text_size(ui_text_size(12.)).text_color(rgb(palette().warning_text))
                        .child(connection_result.clone().unwrap_or_default()),
                ))
                .when(expanded, |body| body.child(
                    v_flex().w_full().min_w_0().gap_2().pt_2()
                        .border_t_1().border_color(settings_group_border())
                        .text_size(ui_text_size(14.))
                        .when(!entry.description.is_empty(), |details| details.child(entry.description.clone()))
                        .child(div().id(("extension-source", index))
                            .debug_selector(move || format!("extension-source-{index}"))
                            .min_w_0().truncate().text_size(ui_text_size(12.)).text_color(rgb(palette().muted))
                            .tooltip({ let path = copy_path.clone(); move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(path.clone()).build(window, cx)
                            }})
                            .child(conduit::i18n::format("settings.extensions.source",
                                &[("arg0", display_path(&entry.path))])))
                        .when(!managed && !toggle_supported, |details| details.child(
                            settings_description(conduit::i18n::text_static(
                                "settings.extensions.edit_the_source_configuration_to_enable_or")),
                        ))
                        .child(h_flex().gap_2().flex_wrap()
                            .child(resource_button(("extension-edit", index)).small().outline()
                                .label(conduit::i18n::text_static("settings.extensions.edit_source_file"))
                                .disabled(disabled || managed)
                                .tooltip(conduit::i18n::text_static(if managed {
                                    "settings.extensions.plugin_files_are_package_managed_manage_the"
                                } else { "settings.extensions.validate_back_up_and_save" }))
                                .on_click(move |_, window, cx| edit_view.update(cx, |app, cx| {
                                    app.open_native_config(edit_path.clone(), kind, false, window, cx)
                                })))
                            .child(resource_button(("extension-reveal", index)).small().ghost()
                                .label(conduit::i18n::text_static("settings.extensions.show_in_folder"))
                                .on_click(move |_, _, cx| cx.reveal_path(&reveal_path)))
                            .child(resource_button(("extension-copy-path", index)).small().ghost()
                                .label(conduit::i18n::text_static("settings.extensions.copy_source_path"))
                                .on_click(move |_, _, cx| cx.write_to_clipboard(
                                    ClipboardItem::new_string(copy_path.clone()))))),
                ))
                .into_any_element()
        }
        ExtensionRow::Empty(kind, message) => {
            let selected = usize::from(*kind == Kind::Mcp);
            let filtered = view.read(cx).extensions_ui.filters[selected] != Filter::All
                || !view.read(cx).extensions_ui.searches[selected]
                    .read(cx)
                    .value()
                    .is_empty();
            h_flex()
                .w_full()
                .min_w_0()
                .px_3()
                .py_3()
                .gap_2()
                .flex_wrap()
                .child(
                    Icon::new(if *kind == Kind::Mcp {
                        FullIconName::Server
                    } else {
                        FullIconName::Blocks
                    })
                    .small()
                    .text_color(rgb(palette().muted)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(ui_text_size(14.))
                        .text_color(rgb(palette().muted))
                        .child(message.clone()),
                )
                .when(filtered, |row| {
                    row.child(
                        resource_button("extensions-clear-filter")
                            .ghost()
                            .label(conduit::i18n::text_static(
                                "settings.appearance.clear_filters",
                            ))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |app, cx| {
                                    app.extensions_ui.filters[selected] = Filter::All;
                                    app.extensions_ui.searches[selected]
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    app.extensions_ui.lists = new_lists();
                                    cx.notify();
                                })
                            }),
                    )
                })
                .into_any_element()
        }
        ExtensionRow::Message(message, error) => div()
            .px_3()
            .py_2()
            .text_size(ui_text_size(12.))
            .text_color(rgb(if *error {
                palette().danger_text
            } else {
                palette().muted
            }))
            .child(message.clone())
            .into_any_element(),
        ExtensionRow::ScanScope(sources) => {
            let open = view.read(cx).extensions_ui.sources_expanded;
            let panel = v_flex()
                .w_full()
                .gap_2()
                .text_size(ui_text_size(12.))
                .text_color(rgb(palette().muted))
                .child(conduit::i18n::text_static(
                    "settings.extensions.scan_scope_missing_files_are_skipped_plugin",
                ))
                .children(sources.iter().enumerate().map(|(index, path)| {
                    let text = display_path(path);
                    let tooltip = text.clone();
                    h_flex()
                        .px_3()
                        .py_1()
                        .gap_2()
                        .child(
                            div()
                                .id(("extensions-scan-source", index))
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                                        .build(window, cx)
                                })
                                .text_size(ui_text_size(12.))
                                .child(text.clone()),
                        )
                        .child(
                            resource_button(("extensions-source-copy", index))
                                .small()
                                .ghost()
                                .label(conduit::i18n::text_static("settings.extensions.copy_path"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                                }),
                        )
                }))
                .child(conduit::i18n::text_static(
                    "settings.extensions.scanning_does_not_resolve_configuration_priority_or",
                ));
            div()
                .my_1()
                .child(settings_accordion(
                    "extensions-sources",
                    conduit::i18n::text_static("settings.extensions.scan_scope"),
                    open,
                    panel,
                    move |open, _, cx| {
                        view.update(cx, |app, cx| {
                            app.extensions_ui.sources_expanded = open;
                            let list = &app.extensions_ui.lists[usize::from(app.extensions_ui.mcp)];
                            list.reset(list.item_count());
                            cx.notify();
                        })
                    },
                ))
                .into_any_element()
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::new_lists;
    use crate::app::ConduitApp;
    use crate::extensions::{
        Catalog, Entry, Kind, Probe,
        connection::{ConnectionProbe, ToolReport},
    };
    use gpui_kit::{AppContext as _, Modifiers, MouseButton, TestAppContext, px};

    #[gpui_kit::test]
    fn settings_mcp_automatically_checks_once_per_visit_and_continues_after_failure(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app =
                    ConduitApp::with_store(window, cx, Ok(Default::default()), false, false);
                app.startup = None;
                app.writable = false;
                app.connection_settings.syncing = true;
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_settings(window, cx);
                app.select_settings_page(9, window, cx);
                let context = app.extensions_ui.context.clone().unwrap();
                // Missing source files ensure the real workers cannot launch any server.
                let entry = |name: &str, kind, disabled| Entry {
                    name: name.into(),
                    description: String::new(),
                    path: std::env::temp_dir().join("conduit-missing-auto-probe-source.json"),
                    scope: "测试".into(),
                    kind,
                    disabled,
                    transport: "stdio".into(),
                    target: "unused".into(),
                };
                let first = entry("first", Kind::Mcp, false);
                let second = entry("second", Kind::Mcp, false);
                let from_catalog = entry("catalog", Kind::Mcp, false);
                let catalog = Catalog {
                    entries: vec![
                        first.clone(),
                        entry("disabled", Kind::Mcp, true),
                        entry("skill", Kind::Skill, false),
                        second.clone(),
                        from_catalog.clone(),
                    ],
                    manual_detection: [from_catalog.id()].into_iter().collect(),
                    updated: Some(std::time::SystemTime::now()),
                    ..Default::default()
                };
                app.extensions_ui.probe = Some(Probe::ready(context.clone(), catalog.clone()));
                app.poll_extensions(window, cx);
                assert_eq!(
                    app.extensions_ui.connection_probe.as_ref().unwrap().id,
                    first.id()
                );
                assert_eq!(app.extensions_ui.detection_queue.len(), 1);
                assert_eq!(
                    app.extensions_ui.connection_results[&from_catalog.id()],
                    crate::extensions::catalog::text("manual_detection")
                );
                assert_eq!(
                    app.extensions_ui.connection_results[&second.id()],
                    "等待检测工具…"
                );

                let (probe, sender) = ConnectionProbe::test_pending(first.id());
                app.extensions_ui.connection_probe = Some(probe);
                app.poll_extensions(window, cx);
                assert_eq!(
                    app.extensions_ui.detection_queue.len(),
                    1,
                    "pending probe must not start a second worker"
                );
                sender.send(Err("fixture failure".into())).unwrap();
                app.poll_extensions(window, cx);
                assert!(
                    app.extensions_ui.connection_results[&first.id()].contains("fixture failure")
                );
                assert_eq!(
                    app.extensions_ui.connection_probe.as_ref().unwrap().id,
                    second.id()
                );
                assert!(app.extensions_ui.detection_queue.is_empty());

                let (probe, sender) = ConnectionProbe::test_pending(second.id());
                app.extensions_ui.connection_probe = Some(probe);
                sender
                    .send(Ok(ToolReport {
                        names: vec!["one".into(), "two".into()],
                    }))
                    .unwrap();
                app.poll_extensions(window, cx);
                assert_eq!(app.extensions_ui.tool_reports[&second.id()].count(), 2);
                for _ in 0..3 {
                    app.poll_extensions(window, cx);
                }
                assert!(
                    app.extensions_ui.connection_probe.is_none(),
                    "completed checks and failures must not restart on polling"
                );
                app.select_settings_page(9, window, cx);
                app.poll_extensions(window, cx);
                assert!(
                    app.extensions_ui.connection_probe.is_none(),
                    "selecting the active page must not repeat the batch"
                );

                app.select_settings_page(2, window, cx);
                app.select_settings_page(9, window, cx);
                app.poll_extensions(window, cx);
                assert_eq!(
                    app.extensions_ui.connection_probe.as_ref().unwrap().id,
                    first.id(),
                    "reentering the cached page starts a fresh batch"
                );
                app.select_settings_page(2, window, cx);
                assert!(app.extensions_ui.connection_probe.is_none());
                assert!(app.extensions_ui.detection_queue.is_empty());
                app.select_settings_page(9, window, cx);
                app.poll_extensions(window, cx);
                app.ensure_extensions(window, cx, true);
                assert!(app.extensions_ui.connection_probe.is_none());
                assert!(app.extensions_ui.tool_reports.is_empty());
                app.extensions_ui.probe = Some(Probe::ready(context, catalog));
                app.poll_extensions(window, cx);
                assert_eq!(
                    app.extensions_ui.connection_probe.as_ref().unwrap().id,
                    first.id()
                );
                app.close_settings(window, cx);
                assert!(app.extensions_ui.connection_probe.is_none());
                assert!(app.extensions_ui.detection_queue.is_empty());
            })
        });

        // Reading a completed tool report must remain available while another
        // check is pending, without restarting either server or losing the report.
        let pending = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_settings(window, cx);
                app.select_settings_page(9, window, cx);
                let context = app.extensions_ui.context.clone().unwrap();
                let entry = Entry {
                    name: "tool-list-fixture".into(),
                    description: String::new(),
                    path: std::env::temp_dir().join("conduit-missing-tool-list-source.json"),
                    scope: "测试".into(),
                    kind: Kind::Mcp,
                    disabled: false,
                    transport: "stdio".into(),
                    target: "unused".into(),
                };
                app.extensions_ui.tool_reports.clear();
                app.extensions_ui.tool_reports.insert(
                    entry.id(),
                    ToolReport {
                        names: vec!["read_file".into(), "list_files".into()],
                    },
                );
                app.extensions_ui.catalogs.insert(
                    context,
                    Catalog {
                        entries: vec![entry],
                        updated: Some(std::time::SystemTime::now()),
                        ..Default::default()
                    },
                );
                app.extensions_ui.probe = None;
                app.extensions_ui.auto_detection_started = true;
                let (probe, sender) = ConnectionProbe::test_pending("unrelated-server".into());
                app.extensions_ui.connection_probe = Some(probe);
                app.extensions_ui.lists = new_lists();
                cx.notify();
                sender
            })
        });
        cx.simulate_resize(gpui_kit::size(px(1200.), px(800.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let button = cx.debug_bounds("mcp-tool-count-0").unwrap().center();
        cx.simulate_mouse_down(button, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(button, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("mcp-tools-list").is_some());
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.extensions_ui.connection_probe.as_ref().unwrap().id,
                "unrelated-server"
            );
            assert_eq!(app.extensions_ui.tool_reports.len(), 1);
            assert_eq!(
                app.extensions_ui
                    .tool_reports
                    .values()
                    .next()
                    .unwrap()
                    .count(),
                2
            );
            assert!(app.extensions_ui.detection_queue.is_empty());
        });
        drop(pending);
    }
}
