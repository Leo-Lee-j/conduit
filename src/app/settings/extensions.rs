//! Native extensions, scoped independently from connections and conversations.
use super::*;
use crate::extensions::management::{self, Filter};
use crate::extensions::{self, Catalog, Kind};

pub(in crate::app) struct ExtensionsUi {
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
            self.extensions_ui
                .connection_results
                .insert(entry.id(), "正在初始化并读取工具列表…".into());
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
        let disabled = !self.writable || self.closing || self.extensions_ui.importing;
        let import_view = view;
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                Button::new("extensions-add")
                    .debug_selector(|| "extensions-add".into())
                    .primary()
                    .small()
                    .icon(FullIconName::Plus)
                    .accessibility_label(if kind == Kind::Mcp {
                        "添加 MCP 服务"
                    } else {
                        "创建 Skill"
                    })
                    .tooltip(if kind == Kind::Mcp {
                        "添加 MCP 服务"
                    } else {
                        "创建 Skill"
                    })
                    .when(!narrow, |button| {
                        button.label(if kind == Kind::Mcp {
                            "添加 MCP 服务"
                        } else {
                            "创建 Skill"
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
                    Button::new("extensions-import")
                        .outline()
                        .small()
                        .icon(FullIconName::FolderOpen)
                        .accessibility_label("从文件夹导入")
                        .tooltip(if self.extensions_ui.importing {
                            "导入中…"
                        } else {
                            "从文件夹导入"
                        })
                        .when(!narrow, |button| {
                            button.label(if self.extensions_ui.importing {
                                "导入中…"
                            } else {
                                "从文件夹导入"
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
        if !self.writable || self.closing || self.extensions_ui.importing {
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
            prompt: Some("选择含 SKILL.md 的技能文件夹".into()),
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
                _ => Some(Err(anyhow::anyhow!("无法打开文件夹选择窗口"))),
            };
            let _ = this.update_in(cx, |app, window, cx| {
                app.extensions_ui.importing = false;
                if let Some(result) = result {
                    let level = if result.is_ok() { NoticeLevel::Success } else { NoticeLevel::Error };
                    let message = match result {
                        Ok(path) => format!("已导入 {}；下次启动会话时加载。", path.display()),
                        Err(error) => error.to_string(),
                    };
                    app.notify_user(Notice::new(NoticeSource::Settings, level, "Skill 导入结果", message.clone())
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
            self.extensions_ui.probe = None;
            self.extensions_ui.cancel_mcp_detection();
            return;
        }
        if self.settings_ui.page != 9 {
            self.extensions_ui.cancel_mcp_detection();
        }
        let connection = self
            .extensions_ui
            .connection_probe
            .as_ref()
            .and_then(|probe| match probe.events.try_recv() {
                Ok(result) => Some((probe.id.clone(), result)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some((probe.id.clone(), Err("测试线程已退出，请重试".into())))
                }
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
                    format!("连接未完成：{error}")
                }
            };
            self.notify_user(
                Notice::new(
                    NoticeSource::Settings,
                    level,
                    "MCP 连接测试",
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
                        errors: vec!["扫描线程已结束，请刷新重试。".into()],
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
                .filter(|entry| entry.kind == Kind::Mcp && !entry.disabled)
                .cloned()
                .collect();
            self.extensions_ui.auto_detection_started = true;
            self.extensions_ui.tool_reports.clear();
            self.extensions_ui.connection_results.clear();
            for entry in &self.extensions_ui.detection_queue {
                self.extensions_ui
                    .connection_results
                    .insert(entry.id(), "等待检测工具…".into());
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

    pub(super) fn extensions_content(
        &self,
        window: &Window,
        available_width: f32,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let narrow = available_width < 540.;
        let compact = narrow || f32::from(window.viewport_size().height) < 650.;
        let engine = self.extensions_ui.engine;
        let mut controls = h_flex().w_full().min_w_0().gap_2().flex_wrap();
        let mut choices = vec![(None, "全局".to_owned())];
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
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "全局".into());
        let scope_view = view.clone();
        controls = controls.child(
            Button::new("extensions-scope")
                .debug_selector(|| "extensions-scope".into())
                .small()
                .ghost()
                .accessibility_label(format!("范围：{current}"))
                .max_w(px(if narrow { 110. } else { 150. }))
                .child(div().truncate().child(format!("范围：{current}")))
                .icon(FullIconName::ChevronDown)
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
            if self.extensions_ui.sources_expanded {
                rows.push(ExtensionRow::Message(
                    "扫描范围（文件不存在时跳过；插件缓存不代表当前加载版本）".into(),
                    false,
                ));
                let mut sources = catalog.sources.clone();
                sources.sort();
                sources.dedup();
                for source in sources {
                    rows.push(ExtensionRow::Source(source));
                }
                rows.push(ExtensionRow::Message("扫描不合并配置优先级，不验证项目受信任状态；环境内联配置、托管配置、远程目录和运行时插件加载未解析。".into(), false));
            }
        }
        if count == 0 {
            rows.push(ExtensionRow::Empty(
                kind,
                if probing {
                    "正在读取本机定义…"
                } else if !query.is_empty() || filter != Filter::All {
                    "没有匹配的条目。"
                } else if catalog.is_none() {
                    "尚未读取或读取已取消，请刷新。"
                } else {
                    "已扫描的来源中没有发现条目。"
                }
                .into(),
            ));
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
        let sources_view = view.clone();
        let sources_expanded = self.extensions_ui.sources_expanded;
        let info = catalog
            .and_then(|c| c.updated)
            .map(|time| {
                format!(
                    "{count} / {total} 项 · 刷新于 {}",
                    DateTime::<Local>::from(time).format("%H:%M:%S")
                )
            })
            .unwrap_or_else(|| format!("{count} / {total} 项"));
        let sources_button = || {
            let sources_view = sources_view.clone();
            Button::new("extensions-sources")
                .small()
                .ghost()
                .icon(if sources_expanded {
                    FullIconName::ChevronUp
                } else {
                    FullIconName::ChevronDown
                })
                .accessibility_label("扫描范围")
                .tooltip("扫描范围")
                .when(!narrow, |button| {
                    button.label(if sources_expanded {
                        "收起扫描范围"
                    } else {
                        "扫描范围"
                    })
                })
                .on_click(move |_, _, cx| {
                    sources_view.update(cx, |app, cx| {
                        app.extensions_ui.sources_expanded = !sources_expanded;
                        app.extensions_ui.lists = new_lists();
                        cx.notify();
                    })
                })
        };
        let filter_view = view.clone();
        let actions = self.extension_actions(narrow, view.clone());
        let search = || {
            div().flex_1().min_w_0().w_full().child(
                Input::new(&self.extensions_ui.searches[selected])
                    .small()
                    .prefix(FullIconName::Search)
                    .cleanable(true)
                    .aria_label("搜索扩展名称、说明或来源"),
            )
        };
        let toolbar = controls
            .when(!narrow, |row| row.child(search()))
            .child(
                Button::new("extensions-filter-trigger")
                    .debug_selector(|| "extensions-filter-trigger".into())
                    .small()
                    .outline()
                    .label(filter.label())
                    .icon(FullIconName::ChevronDown)
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
                Button::new("extensions-refresh")
                    .debug_selector(|| "extensions-refresh".into())
                    .small()
                    .outline()
                    .icon(FullIconName::RefreshCw)
                    .accessibility_label(if probing { "读取中…" } else { "刷新" })
                    .tooltip(if probing { "读取中…" } else { "刷新" })
                    .when(!narrow, |button| {
                        button.label(if probing { "读取中…" } else { "刷新" })
                    })
                    .disabled(probing || self.closing)
                    .on_click(move |_, window, cx| {
                        refresh.update(cx, |app, cx| app.ensure_extensions(window, cx, true))
                    }),
            )
            .when(narrow, |row| row.child(sources_button()))
            .when(probing, |row| {
                row.child(
                    Button::new("extensions-cancel")
                        .small()
                        .ghost()
                        .label("取消")
                        .on_click(move |_, _, cx| {
                            cancel.update(cx, |app, cx| {
                                app.extensions_ui.probe = None;
                                cx.notify();
                            })
                        }),
                )
            });
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
            .child(management_actions(actions, narrow))
            .child(
                div()
                    .px(px(if narrow { 12. } else { 24. }))
                    .pb(px(if narrow { 8. } else { 12. }))
                    .child(engine_tabs(engine, true, narrow, view)),
            )
            .child(
                v_flex()
                    .w_full()
                    .flex_shrink_0()
                    .px(px(if narrow { 12. } else { 24. }))
                    .pb_3()
                    .gap_2()
                    .child(toolbar)
                    .when(!compact, |body| {
                        body.child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().muted))
                                .child(if self.extensions_ui.workspace.is_some() {
                                    "当前工作区及继承的全局定义 · 加载状态以会话为准"
                                } else {
                                    "全局定义 · 加载状态以会话为准"
                                }),
                        )
                    })
                    .when_some(self.extensions_ui.feedback.clone(), |body, message| {
                        body.child(div().text_size(ui_text_size(12.)).child(message))
                    })
                    .when(!narrow, |body| {
                        body.child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(ui_text_size(12.))
                                        .text_color(rgb(palette().muted))
                                        .child(info),
                                )
                                .child(sources_button()),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .px(px(if narrow { 12. } else { 24. }))
                    .pb_4()
                    .child(
                        v_flex()
                            .size_full()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(palette().border))
                            .overflow_hidden()
                            .child(
                                h_flex()
                                    .w_full()
                                    .flex_shrink_0()
                                    .px_4()
                                    .py(px(if narrow { 4. } else { 8. }))
                                    .justify_between()
                                    .bg(rgb(palette().surface_strong))
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(if narrow {
                                        format!(
                                            "{} · {count} / {total}",
                                            if mcp {
                                                "服务 / 连接地址"
                                            } else {
                                                "技能 / 说明"
                                            }
                                        )
                                    } else if mcp {
                                        "服务 / 连接地址".to_owned()
                                    } else {
                                        "技能 / 说明".to_owned()
                                    })
                                    .child("状态与操作"),
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

enum ExtensionRow {
    Entry(extensions::Entry, bool),
    Empty(Kind, String),
    Message(String, bool),
    Source(PathBuf),
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
            let copy_path = entry.path.display().to_string();
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
            let quick_edit = view.clone();
            let quick_path = entry.path.clone();
            let count_entry = entry.clone();
            let count_view = view.clone();
            let ui = &view.read(cx).extensions_ui;
            let report = ui.tool_reports.get(&id).cloned();
            let probing = ui.connection_probe.is_some();
            let this_probe = ui
                .connection_probe
                .as_ref()
                .is_some_and(|probe| probe.id == id);
            let queued = ui.detection_queue.iter().any(|entry| entry.id() == id);
            let connection_result = ui.connection_results.get(&id).cloned();
            let entry_status = if !mcp || entry.disabled {
                entry.status()
            } else if report.is_some() {
                "连接正常"
            } else if this_probe {
                "检测中"
            } else if queued {
                "等待检测"
            } else if connection_result.is_some() {
                "检测未完成"
            } else {
                entry.status()
            };
            let tool_label = if entry.disabled {
                "0 个可用工具".to_string()
            } else if let Some(report) = &report {
                format!("{} 个可用工具", report.count())
            } else if this_probe {
                "正在检测工具…".to_string()
            } else if queued {
                "等待检测工具…".to_string()
            } else if connection_result.is_some() {
                "重新检测工具".to_string()
            } else {
                "检测工具数量".to_string()
            };
            let tool_hint = report
                .as_ref()
                .map(|report| report.summary())
                .or_else(|| connection_result.clone())
                .unwrap_or_else(|| "初始化连接并读取完整工具列表；不会调用工具。".into());
            let tool_count = mcp.then(|| {
                Button::new(("mcp-tool-count", index))
                    .debug_selector(move || format!("mcp-tool-count-{index}"))
                    .small()
                    .text()
                    .self_start()
                    .accessibility_label(format!("{} · {tool_label}", entry.name))
                    .child(management_badge(tool_label, false).when(
                        report.is_some() || entry.disabled,
                        |tag| {
                            tag.bg(gpui_kit::rgba((palette().activity_completed << 8) | 0x18))
                                .text_color(rgb(palette().activity_completed))
                        },
                    ))
                    .tooltip(tool_hint)
                    .disabled(
                        entry.disabled
                            || view.read(cx).closing
                            || view.read(cx).extensions_ui.connection_probe.is_some(),
                    )
                    .on_click(move |_, _, cx| {
                        count_view.update(cx, |app, cx| {
                            app.test_mcp_connection(count_entry.clone(), cx)
                        })
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
            (v_flex()
                .w_full()
                .px_4()
                .py(px(if narrow { 8. } else { 12. }))
                .gap_2()
                .border_b_1()
                .border_color(rgb(palette().border))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_start()
                        .child(div().size(px(32.)).flex_shrink_0().rounded_md().bg(rgb(palette().surface_strong))
                            .flex().items_center().justify_center()
                            .child(Icon::new(if mcp { FullIconName::Server } else { FullIconName::Sparkles }).small().text_color(rgb(palette().muted))))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .flex_wrap()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(ui_text_size(13.))
                                                .font_semibold()
                                                .debug_selector(move || format!("extension-name-{index}"))
                                                .child(entry.name.clone()),
                                        )
                                        .when(!narrow, |row| row
                                            .child(management_badge(entry_status, !entry.disabled))
                                            .child(management_badge(entry.scope.clone(), false))
                                            .when(*duplicate, |row| row.child(management_badge("同名定义", false)))),
                                )
                                .child(
                                    div()
                                        .text_size(ui_text_size(12.))
                                        .text_color(rgb(palette().muted))
                                        .truncate()
                                        .child(summary),
                                )
                                .when_some(tool_count, |body, count| body.child(count))
                                .when(narrow, |body| body.child(
                                    h_flex().gap_1().flex_wrap()
                                        .child(management_badge(entry_status, !entry.disabled))
                                        .child(management_badge(entry.scope.clone(), false))
                                        .when(*duplicate, |row| row.child(management_badge("同名定义", false)))
                                )),
                        )
                .child(h_flex().gap_2().flex_wrap()
                    .child(Button::new(("extension-quick-edit", index)).small().ghost().icon(FullIconName::Pencil).accessibility_label(if managed { "插件托管" } else { "编辑" }).when(!narrow, |button| button.label(if managed { "插件托管" } else { "编辑" })).disabled(disabled || managed).tooltip(if managed { "由插件安装包管理，请在对应 CLI 中调整" } else { "编辑此条目的来源文件" })
                        .on_click(move |_, window, cx| quick_edit.update(cx, |app, cx| app.open_native_config(quick_path.clone(), kind, false, window, cx))))
                    .when(!managed && toggle_supported, |row| row.child(Button::new(("extension-toggle", index)).small().ghost()
                        .label(if entry.disabled { "恢复" } else { "停用" }).disabled(disabled || view.read(cx).extensions_ui.changing)
                        .on_click(move |_, window, cx| state_view.update(cx, |app, cx| app.open_extension_toggle(state_entry.clone(), window, cx)))))
                    .when(!managed && !toggle_supported, |row| row.child(Button::new(("extension-native-toggle", index)).small().ghost().label("原生管理").disabled(true).tooltip("此引擎的启停请编辑来源配置"))))
                        .child(
                            Button::new(("extension-detail", index))
                                .debug_selector(move || format!("extension-detail-{index}"))
                                .small()
                                .ghost()
                                .label(if expanded { "收起" } else { "详情" })
                                .on_click(move |_, _, cx| {
                                    toggle_view.update(cx, |app, cx| {
                                        if !app.extensions_ui.expanded.remove(&toggle_id) {
                                            app.extensions_ui.expanded.clear();
                                            app.extensions_ui.expanded.insert(toggle_id.clone());
                                        }
                                        let selected = usize::from(app.extensions_ui.mcp);
                                        app.extensions_ui.lists[selected]
                                            .reset(app.extensions_ui.lists[selected].item_count());
                                        cx.notify();
                                    });
                                }),
                        ),
                )
                .when(expanded, |row| {
                    row.child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .py_2()
                            .text_size(ui_text_size(12.))
                            .when(!entry.description.is_empty(), |details| {
                                details.child(entry.description.clone())
                            })
                            .child(
                                div()
                                    .id(("extension-source", index))
                                    .debug_selector(move || format!("extension-source-{index}"))
                                    .truncate()
                                    .tooltip({
                                        let path = copy_path.clone();
                                        move |window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(path.clone())
                                                .build(window, cx)
                                        }
                                    })
                                    .text_color(rgb(palette().muted))
                                    .child(format!("来源：{}", entry.path.display())),
                            )
                            .when(mcp, |details| details.child("测试会启动此定义中的命令或连接远程服务，仅初始化并读取工具列表。"))
                            .when(mcp, |details| details.child(h_flex().gap_2()
                                .child(Button::new(("mcp-test", index)).small().outline().label(if this_probe { "测试中…" } else { "测试连接" })
                                    .disabled(probing || entry.disabled || view.read(cx).closing)
                                    .on_click(move |_, _, cx| test_view.update(cx, |app, cx| app.test_mcp_connection(test_entry.clone(), cx))))
                                .when(this_probe, |row| row.child(Button::new(("mcp-cancel", index)).small().ghost().label("取消")
                                    .on_click(move |_, _, cx| cancel_view.update(cx, |app, cx| {
                                        if let Some(probe) = app.extensions_ui.connection_probe.take() { app.extensions_ui.connection_results.insert(probe.id.clone(), "测试已取消".into()); }
                                        app.extensions_ui.lists = new_lists(); cx.notify();
                                    }))))))
                            .when_some(connection_result, |details, message| details.child(div().text_size(ui_text_size(12.)).child(message)))
                            .child(h_flex().gap_2().flex_wrap()
                                .child(Button::new(("extension-edit", index)).small().outline().label("编辑来源文件").disabled(disabled || managed)
                                    .tooltip(if managed { "插件文件由安装包管理，请在对应 CLI 中管理插件" } else { "校验格式并备份后保存" })
                                    .on_click(move |_, window, cx| edit_view.update(cx, |app, cx| app.open_native_config(edit_path.clone(), kind, false, window, cx))))
                                .child(Button::new(("extension-reveal", index)).small().ghost().label("在文件夹中显示").on_click(move |_, _, cx| cx.reveal_path(&reveal_path))))
                            .child(
                                Button::new(("extension-copy-path", index))
                                    .small()
                                    .ghost()
                                    .label("复制来源路径")
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_path.clone(),
                                        ))
                                    }),
                            ),
                    )
                }))
            .into_any_element()
        }
        ExtensionRow::Empty(kind, message) => v_flex()
            .w_full()
            .items_center()
            .gap_3()
            .p_6()
            .child(
                div()
                    .size(px(40.))
                    .rounded_lg()
                    .bg(rgb(palette().surface_strong))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::new(if *kind == Kind::Mcp {
                            FullIconName::Server
                        } else {
                            FullIconName::Sparkles
                        })
                        .text_color(rgb(palette().muted)),
                    ),
            )
            .child(
                div()
                    .text_size(ui_text_size(13.))
                    .text_color(rgb(palette().muted))
                    .child(message.clone()),
            )
            .into_any_element(),
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
        ExtensionRow::Source(path) => {
            let text = path.display().to_string();
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
                    Button::new(("extensions-source-copy", index))
                        .small()
                        .ghost()
                        .label("复制路径")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                        }),
                )
                .into_any_element()
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use crate::app::ConduitApp;
    use crate::extensions::{
        Catalog, Entry, Kind, Probe,
        connection::{ConnectionProbe, ToolReport},
    };
    use gpui_kit::{AppContext as _, TestAppContext};

    #[gpui_kit::test]
    fn settings_mcp_automatically_checks_once_per_visit_and_continues_after_failure(
        cx: &mut TestAppContext,
    ) {
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
                let catalog = Catalog {
                    entries: vec![
                        first.clone(),
                        entry("disabled", Kind::Mcp, true),
                        entry("skill", Kind::Skill, false),
                        second.clone(),
                    ],
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
    }
}
