//! Read-only extension browser, scoped independently from the conversation.
use super::*;
use crate::extensions::{self, Catalog, Kind};

pub(in crate::app) struct ExtensionsUi {
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
    pub fn new(skills: Entity<InputState>, mcp: Entity<InputState>) -> Self {
        Self {
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
    pub fn invalidate(&mut self) {
        self.probe = None;
        self.catalogs.clear();
        self.context = None;
    }
}

impl ConduitApp {
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
            engine: self.connection_settings.engine,
            workspace: self.extensions_ui.workspace.clone(),
            cli_path: cli::resolve(self.connection_settings.engine, &self.store.cli)
                .ok()
                .map(|c| c.path),
        };
        if self.extensions_ui.context.as_ref() != Some(&context) {
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

    pub(in crate::app) fn poll_extensions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || !window.has_active_dialog(cx) || self.settings_ui.page != 4 {
            self.extensions_ui.probe = None;
            return;
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
    }

    pub(super) fn extensions_content(
        &self,
        _: &Window,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let engine = self.connection_settings.engine;
        let mut controls = h_flex().gap_2().flex_wrap();
        for (index, candidate) in Provider::ALL.into_iter().enumerate() {
            let view = view.clone();
            controls = controls.child(
                Button::new(("extensions-engine", index))
                    .debug_selector(move || format!("extensions-engine-{index}"))
                    .small()
                    .outline()
                    .label(candidate.label())
                    .selected(engine == candidate)
                    .on_click(move |_, window, cx| {
                        view.update(cx, |app, cx| {
                            app.connection_settings.engine = candidate;
                            app.ensure_extensions(window, cx, false);
                            cx.notify();
                        });
                    }),
            );
        }
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
                .max_w(px(230.))
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
        let tab_view = view.clone();
        let tabs = TabBar::new("extensions-tabs")
            .underline()
            .selected_index(selected)
            .child(
                Tab::new()
                    .label("Skills")
                    .debug_selector(|| "extensions-skills-tab".into()),
            )
            .child(
                Tab::new()
                    .label("MCP")
                    .debug_selector(|| "extensions-mcp-tab".into()),
            )
            .on_click(move |index, _, cx| {
                tab_view.update(cx, |app, cx| {
                    app.extensions_ui.mcp = *index == 1;
                    cx.notify();
                });
            });
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
        if let Some(catalog) = catalog {
            for entry in catalog.entries.iter().filter(|entry| entry.kind == kind) {
                if !format!("{} {} {}", entry.name, entry.description, entry.scope)
                    .to_lowercase()
                    .contains(query.as_str())
                {
                    continue;
                }
                count += 1;
                rows.push(ExtensionRow::Entry(entry.clone()));
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
            rows.push(ExtensionRow::Message(
                if probing {
                    "正在读取本机定义…"
                } else if !query.is_empty() {
                    "没有匹配的条目。"
                } else if catalog.is_none() {
                    "尚未读取或读取已取消，请刷新。"
                } else {
                    "已扫描的来源中没有发现条目。"
                }
                .into(),
                false,
            ));
        }
        let list_state = self.extensions_ui.lists[selected].clone();
        if list_state.item_count() != rows.len() {
            list_state.reset(rows.len());
        }
        let expanded = self.extensions_ui.expanded.clone();
        let list_view = view.clone();
        let rows = list(list_state.clone(), move |index, _, _| {
            render_extension_row(&rows[index], index, &expanded, list_view.clone())
        })
        .size_full();
        let sources_view = view.clone();
        let sources_expanded = self.extensions_ui.sources_expanded;
        let info = catalog
            .and_then(|c| c.updated)
            .map(|time| {
                format!(
                    "{count} 项 · 刷新于 {}",
                    DateTime::<Local>::from(time).format("%H:%M:%S")
                )
            })
            .unwrap_or_else(|| format!("{count} 项"));
        v_flex()
            .size_full()
            .text_color(rgb(palette().text))
            .child(
                v_flex()
                    .flex_shrink_0()
                    .p_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(div().text_size(px(18.)).font_semibold().child("扩展"))
                    .child(controls)
                    .child(tabs)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.extensions_ui.searches[selected]).small(),
                                ),
                            )
                            .child(
                                Button::new("extensions-refresh")
                                    .debug_selector(|| "extensions-refresh".into())
                                    .small()
                                    .outline()
                                    .label(if probing { "读取中…" } else { "刷新" })
                                    .disabled(probing || self.closing)
                                    .on_click(move |_, window, cx| {
                                        refresh.update(cx, |app, cx| {
                                            app.ensure_extensions(window, cx, true)
                                        });
                                    }),
                            )
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
                                            });
                                        }),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child("配置扫描结果 · 完整性与会话可用性未确认；不自动连接 MCP。"),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(info)
                            .child(
                                Button::new("extensions-sources")
                                    .small()
                                    .ghost()
                                    .label(if sources_expanded {
                                        "收起扫描范围"
                                    } else {
                                        "扫描范围"
                                    })
                                    .on_click(move |_, _, cx| {
                                        sources_view.update(cx, |app, cx| {
                                            app.extensions_ui.sources_expanded = !sources_expanded;
                                            app.extensions_ui.lists = new_lists();
                                            cx.notify();
                                        });
                                    }),
                            ),
                    ),
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
            )
            .into_any_element()
    }
}

fn new_lists() -> [gpui_kit::ListState; 2] {
    std::array::from_fn(|_| gpui_kit::ListState::new(0, ListAlignment::Top, px(100.)))
}

enum ExtensionRow {
    Entry(extensions::Entry),
    Message(String, bool),
    Source(PathBuf),
}
fn render_extension_row(
    row: &ExtensionRow,
    index: usize,
    expanded: &HashSet<String>,
    view: Entity<ConduitApp>,
) -> AnyElement {
    match row {
        ExtensionRow::Entry(entry) => {
            let mcp = entry.kind == Kind::Mcp;
            let id = entry.id();
            let expanded = expanded.contains(&id);
            let toggle_view = view.clone();
            let toggle_id = id.clone();
            let copy_path = entry.path.display().to_string();
            let summary = if mcp {
                format!("{} · {}", entry.transport, entry.target)
            } else {
                entry.description.replace('\n', " ")
            };
            (v_flex()
                .w_full()
                .px_3()
                .py_2()
                .gap_1()
                .border_b_1()
                .border_color(rgb(palette().border))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_start()
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
                                                .text_size(px(13.))
                                                .font_semibold()
                                                .child(entry.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(rgb(palette().muted))
                                                .child(format!(
                                                    "{} · {}",
                                                    entry.scope,
                                                    entry.status()
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(palette().muted))
                                        .truncate()
                                        .child(summary),
                                ),
                        )
                        .child(
                            Button::new(("extension-detail", index))
                                .debug_selector(move || format!("extension-detail-{index}"))
                                .small()
                                .ghost()
                                .label(if expanded { "收起" } else { "详情" })
                                .on_click(move |_, _, cx| {
                                    toggle_view.update(cx, |app, cx| {
                                        if !app.extensions_ui.expanded.remove(&toggle_id) {
                                            app.extensions_ui.expanded.insert(toggle_id.clone());
                                        }
                                        let selected = usize::from(app.extensions_ui.mcp);
                                        app.extensions_ui.lists[selected]
                                            .splice(index..index + 1, 1);
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
                            .text_size(px(12.))
                            .when(!entry.description.is_empty(), |details| {
                                details.child(entry.description.clone())
                            })
                            .child(
                                div()
                                    .id(("extension-source", index))
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
                            .when(mcp, |details| {
                                details
                                    .child("连接与认证状态未检测；命令参数、凭据和端点路径不展示。")
                            })
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
        ExtensionRow::Message(message, error) => div()
            .px_3()
            .py_2()
            .text_size(px(12.))
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
                        .text_size(px(12.))
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
