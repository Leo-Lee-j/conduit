use super::*;
use crate::extensions::{
    Kind,
    catalog::{self, Item, Payload, Source, text},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
mod dialogs;

/// Shared, content-sized rows for extension and application-plugin resources.
/// The workspace owns the page title and maximum content width.
pub(super) fn resource_row() -> Div {
    v_flex()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .px_3()
        .py_2()
        .gap_1()
        .border_b_1()
        .border_color(settings_group_border())
        .text_size(ui_text_size(14.))
}

pub(super) fn resource_list() -> Div {
    v_flex()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .border_1()
        .border_color(settings_group_border())
        .rounded(px(4.))
        .overflow_hidden()
}

pub(super) fn resource_badge(label: impl Into<SharedString>, active: bool) -> Div {
    management_badge(label, active)
        .text_size(ui_text_size(12.))
        .rounded(px(4.))
}

pub(super) fn resource_button(id: impl Into<ElementId>) -> Button {
    Button::new(id)
        .small()
        .h(ui_text_size(32.))
        .min_w(ui_text_size(32.))
        .text_size(ui_text_size(14.))
}

pub(super) fn resource_meta(id: impl Into<ElementId>, value: String) -> Stateful<Div> {
    div()
        .id(id)
        .min_w_0()
        .max_w_full()
        .truncate()
        .text_size(ui_text_size(12.))
        .text_color(rgb(palette().muted))
        .tooltip({
            let value = value.clone();
            move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(value.clone()).build(window, cx)
            }
        })
        .child(value)
}

/// View-only disclosure state stays in the window; browsing never changes a
/// package, its enabled state or the current icon theme.
#[derive(IntoElement)]
pub(super) struct ResourceDetails {
    pub id: SharedString,
    pub name: String,
    pub view: Entity<ConduitApp>,
    pub summary: AnyElement,
    pub details: AnyElement,
}

impl RenderOnce for ResourceDetails {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            SharedString::from(format!("{}-expanded", self.id)),
            cx,
            |_, _| false,
        );
        let expanded = *state.read(cx);
        let view = self.view;
        resource_row()
            .id(self.id.clone())
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .items_start()
                    .child(div().flex_1().min_w_0().child(self.summary))
                    .child(
                        resource_button(self.id.clone())
                            .debug_selector({
                                let id = self.id.clone();
                                move || id.to_string()
                            })
                            .small()
                            .ghost()
                            .h(ui_text_size(32.))
                            .text_size(ui_text_size(14.))
                            .selected(expanded)
                            .flex_shrink_0()
                            .accessibility_label(format!(
                                "{} · {}",
                                self.name,
                                conduit::i18n::text_static("settings.extensions.details")
                            ))
                            .label(conduit::i18n::text_static("settings.extensions.details"))
                            .icon(if expanded {
                                FullIconName::ChevronUp
                            } else {
                                FullIconName::ChevronDown
                            })
                            .on_click(move |_, _, cx| {
                                state.update(cx, |open, cx| {
                                    *open = !*open;
                                    cx.notify();
                                });
                                view.update(cx, |_, cx| cx.notify());
                            }),
                    ),
            )
            .when(expanded, |row| {
                row.child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(settings_group_border())
                        .child(self.details),
                )
            })
    }
}

pub(in crate::app) struct Browse {
    pub discover: bool,
    source: Source,
    input: Option<Entity<InputState>>,
    query: String,
    entries: Vec<Item>,
    next: Option<String>,
    loading: bool,
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    feedback: Option<String>,
    fetched_at: u64,
    cached: bool,
    installed: HashSet<String>,
    checking_installed: bool,
    target: Option<(Provider, Option<PathBuf>)>,
    preparing: bool,
}
impl Browse {
    pub fn new(mcp: bool) -> Self {
        Self {
            discover: false,
            source: if mcp {
                Source::McpRegistry
            } else {
                Source::Featured
            },
            input: None,
            query: String::new(),
            entries: vec![],
            next: None,
            loading: false,
            generation: 0,
            cancel: None,
            feedback: None,
            fetched_at: 0,
            cached: false,
            installed: HashSet::new(),
            checking_installed: false,
            target: None,
            preparing: false,
        }
    }
    pub(super) fn cancel(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.generation = self.generation.wrapping_add(1);
        self.loading = false;
        self.preparing = false;
        if self.checking_installed {
            self.target = None;
            self.checking_installed = false;
        }
    }
}
impl Drop for Browse {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl ConduitApp {
    pub(super) fn extensions_content(
        &self,
        window: &Window,
        available_width: f32,
        view: Entity<Self>,
        cx: &App,
    ) -> SettingsContent {
        let index = usize::from(self.extensions_ui.mcp);
        let discover = self.extensions_ui.discovery[index].discover;
        let installed_view = view.clone();
        let discover_view = view.clone();
        let tabs = h_flex()
            .debug_selector(|| "extension-mode-tabs".into())
            .gap_1()
            .flex_shrink_0()
            .child(
                resource_button("extensions-installed-tab")
                    .small()
                    .ghost()
                    .label(text("installed"))
                    .selected(!discover)
                    .on_click(move |_, _, cx| {
                        installed_view.update(cx, |app, cx| {
                            app.extensions_ui.discovery[index].discover = false;
                            app.extensions_ui.discovery[index].cancel();
                            cx.notify();
                        });
                    }),
            )
            .child(
                resource_button("extensions-discover-tab")
                    .small()
                    .ghost()
                    .debug_selector(|| "extensions-discover-tab".into())
                    .label(text("discover"))
                    .selected(discover)
                    .on_click(move |_, window, cx| {
                        discover_view.update(cx, |app, cx| {
                            app.show_extension_discovery(index, window, cx)
                        });
                    }),
            );
        SettingsContent::new(if discover {
            self.extension_catalog_content(index, view.clone())
        } else {
            self.installed_extensions_content(window, available_width, view.clone(), cx)
        })
        .with_toolbar(management_toolbar(
            self.extensions_ui.engine,
            true,
            Some(tabs.into_any_element()),
            None,
            view,
        ))
    }
    fn show_extension_discovery(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = &mut self.extensions_ui.discovery[index];
        state.discover = true;
        if state.input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(text("search_hint")));
            cx.subscribe(&input, move |app, _, event, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && !app.extensions_ui.discovery[index].preparing
                {
                    app.submit_extension_catalog(index, cx);
                }
            })
            .detach();
            state.input = Some(input);
        }
        if state.entries.is_empty() && !state.loading {
            self.query_extension_catalog(index, false, false, cx);
        } else {
            self.refresh_catalog_target(index, cx);
        }
        cx.notify();
    }
    pub(super) fn refresh_catalog_target(&mut self, index: usize, cx: &mut Context<Self>) {
        let target = (
            self.extensions_ui.engine,
            self.extensions_ui.workspace.clone(),
        );
        let state = &mut self.extensions_ui.discovery[index];
        if state.target.as_ref() == Some(&target) || state.entries.is_empty() {
            return;
        }
        state.cancel();
        state.target = Some(target.clone());
        state.installed.clear();
        state.checking_installed = true;
        let generation = state.generation;
        let entries = state.entries.clone();
        cx.spawn(async move |this, cx| {
            let selected = target.clone();
            let installed = cx
                .background_executor()
                .spawn(async move {
                    catalog::receipts::installed(
                        &catalog::receipts::root(),
                        &entries,
                        &crate::extensions::edit::destination(
                            target.0,
                            target.1.as_deref(),
                            Kind::Skill,
                        ),
                        &crate::extensions::edit::destination(
                            target.0,
                            target.1.as_deref(),
                            Kind::Mcp,
                        ),
                    )
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                let state = &mut app.extensions_ui.discovery[index];
                if !app.closing
                    && state.generation == generation
                    && state.target.as_ref() == Some(&selected)
                {
                    state.installed = installed;
                    state.checking_installed = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn submit_extension_catalog(&mut self, index: usize, cx: &mut Context<Self>) {
        let state = &mut self.extensions_ui.discovery[index];
        if index == 0
            && state.source != Source::Github
            && state
                .input
                .as_ref()
                .is_some_and(|input| catalog::is_github_reference(input.read(cx).value().trim()))
        {
            state.source = Source::Github;
            state.query.clear();
        }
        self.query_extension_catalog(index, false, false, cx);
    }
    fn query_extension_catalog(
        &mut self,
        index: usize,
        more: bool,
        refresh: bool,
        cx: &mut Context<Self>,
    ) {
        if self.closing {
            return;
        }
        let target = (
            self.extensions_ui.engine,
            self.extensions_ui.workspace.clone(),
        );
        let state = &mut self.extensions_ui.discovery[index];
        state.cancel();
        let query = if more {
            state.query.clone()
        } else {
            state
                .input
                .as_ref()
                .map(|i| i.read(cx).value().trim().to_owned())
                .unwrap_or_default()
        };
        let cursor = if more { state.next.clone() } else { None };
        if more && cursor.is_none() {
            return;
        }
        if !more && state.query != query {
            state.entries.clear();
            state.installed.clear();
            state.next = None;
            state.fetched_at = 0;
        }
        state.query = query.clone();
        state.loading = true;
        state.feedback = None;
        let generation = state.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        state.cancel = Some(cancel.clone());
        let source = state.source;
        let root = Store::path().parent().unwrap().to_owned();
        let writable = self.writable;
        cx.spawn(async move |this, cx| {
            let selected_target = target.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let page = catalog::search(
                        source,
                        &query,
                        cursor.as_deref(),
                        &root,
                        refresh,
                        writable,
                        &cancel,
                    )?;
                    let skill_path = crate::extensions::edit::destination(
                        target.0,
                        target.1.as_deref(),
                        Kind::Skill,
                    );
                    let mcp_path = crate::extensions::edit::destination(
                        target.0,
                        target.1.as_deref(),
                        Kind::Mcp,
                    );
                    let installed = catalog::receipts::installed(
                        &catalog::receipts::root(),
                        &page.items,
                        &skill_path,
                        &mcp_path,
                    );
                    Ok::<_, anyhow::Error>((page, installed))
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                let state = &mut app.extensions_ui.discovery[index];
                if app.closing || state.generation != generation {
                    return;
                }
                state.loading = false;
                state.cancel = None;
                match result {
                    Ok((page, installed)) => {
                        if !more {
                            state.entries.clear();
                            state.installed.clear();
                        }
                        for item in page.items {
                            if !state.entries.iter().any(|old| old.id == item.id) {
                                state.entries.push(item);
                            }
                        }
                        if source == Source::McpRegistry {
                            state.entries.truncate(300);
                        }
                        state.next = if source == Source::McpRegistry && state.entries.len() >= 300
                        {
                            None
                        } else {
                            page.next
                        };
                        state.feedback = page.warning;
                        state.fetched_at = page.fetched_at;
                        state.cached = page.cached;
                        state.installed.extend(installed);
                        state.target = Some(selected_target);
                    }
                    Err(error) => state.feedback = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn extension_catalog_content(&self, index: usize, view: Entity<Self>) -> AnyElement {
        let state = &self.extensions_ui.discovery[index];
        let source_view = view.clone();
        let search_view = view.clone();
        let refresh_view = view.clone();
        let cancel_view = view.clone();
        let scope_view = view.clone();
        let scope = self
            .extensions_ui
            .workspace
            .as_ref()
            .map(|p| display_path(p))
            .unwrap_or_else(|| text("global"));
        let choices: Vec<_> = std::iter::once((None, text("global")))
            .chain(
                self.store
                    .workspaces
                    .iter()
                    .map(|w| (Some(w.path.clone()), w.name())),
            )
            .collect();
        let mut body = v_flex().size_full().min_w_0().min_h_0().child(
            v_flex()
                .px(px(settings_content_inset()))
                .pt_2()
                .gap_2()
                .flex_shrink_0()
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            settings_dropdown("catalog-source")
                                .min_w(ui_text_size(32.))
                                .label(state.source.label())
                                .dropdown_menu(move |mut menu, _, _| {
                                    let sources = if index == 1 {
                                        vec![Source::McpRegistry]
                                    } else {
                                        vec![Source::Featured, Source::SkillsSh, Source::Github]
                                    };
                                    for source in sources {
                                        let view = source_view.clone();
                                        menu =
                                            menu.item(PopupMenuItem::new(source.label()).on_click(
                                                move |_, window, cx| {
                                                    view.update(cx, |app, cx| {
                                                        let state =
                                                            &mut app.extensions_ui.discovery[index];
                                                        if state.source == source {
                                                            return;
                                                        }
                                                        state.cancel();
                                                        state.source = source;
                                                        if let Some(input) = state.input.as_ref() {
                                                            input.update(cx, |input, cx| {
                                                                input.set_value("", window, cx)
                                                            });
                                                        }
                                                        state.query.clear();
                                                        state.entries.clear();
                                                        state.installed.clear();
                                                        state.next = None;
                                                        state.fetched_at = 0;
                                                        state.feedback = None;
                                                        app.query_extension_catalog(
                                                            index, false, false, cx,
                                                        );
                                                    });
                                                },
                                            ));
                                    }
                                    menu
                                }),
                        )
                        .child(
                            settings_dropdown("catalog-scope")
                                .debug_selector(|| "catalog-scope".into())
                                .min_w(ui_text_size(32.))
                                .max_w(px(280.))
                                .accessibility_label(scope.clone())
                                .tooltip(scope.clone())
                                .child(div().min_w_0().truncate().child(scope))
                                .dropdown_menu(move |mut menu, _, _| {
                                    for (path, name) in &choices {
                                        let path = path.clone();
                                        let view = scope_view.clone();
                                        menu =
                                            menu.item(PopupMenuItem::new(name.clone()).on_click(
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
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .w_full()
                        .flex_wrap()
                        .when_some(state.input.clone(), |row, input| {
                            row.child(
                                div().flex_1().min_w(px(160.)).max_w(px(480.)).child(
                                    settings_input(&input)
                                        .cleanable(true)
                                        .aria_label(text("search_hint")),
                                ),
                            )
                        })
                        .child(
                            resource_button("catalog-search")
                                .label(text("search"))
                                .disabled(state.loading || state.preparing)
                                .on_click(move |_, _, cx| {
                                    search_view.update(cx, |app, cx| {
                                        app.submit_extension_catalog(index, cx)
                                    });
                                }),
                        )
                        .child(
                            resource_button("catalog-refresh")
                                .icon(FullIconName::RefreshCw)
                                .tooltip(text("refresh"))
                                .accessibility_label(text("refresh"))
                                .disabled(state.loading || state.preparing)
                                .on_click(move |_, _, cx| {
                                    refresh_view.update(cx, |app, cx| {
                                        app.query_extension_catalog(index, false, true, cx)
                                    });
                                }),
                        ),
                )
                .when(
                    index == 0 && state.source == Source::Github && state.query.is_empty(),
                    |body| {
                        let mut repos = h_flex().gap_2().flex_wrap();
                        for (position, repo) in [
                            "anthropics/skills",
                            "vercel-labs/agent-skills",
                            "obra/superpowers",
                            "remotion-dev/skills",
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let view = view.clone();
                            repos = repos.child(
                                resource_button(("catalog-browse-repo", position))
                                    .label(repo)
                                    .small()
                                    .outline()
                                    .disabled(state.preparing || self.closing)
                                    .on_click(move |_, window, cx| {
                                        view.update(cx, |app, cx| {
                                            if let Some(input) =
                                                app.extensions_ui.discovery[index].input.as_ref()
                                            {
                                                input.update(cx, |input, cx| {
                                                    input.set_value(repo, window, cx)
                                                });
                                            }
                                            app.submit_extension_catalog(index, cx);
                                        });
                                    }),
                            );
                        }
                        body.child(repos)
                    },
                )
                .when(state.fetched_at > 0, |body| {
                    body.child(note(format!(
                        "{} · {}",
                        if state.cached {
                            text("cached")
                        } else {
                            text("updated")
                        },
                        chrono::DateTime::from_timestamp(state.fetched_at as i64, 0)
                            .map(|time| time
                                .with_timezone(&chrono::Local)
                                .format("%Y-%m-%d %H:%M")
                                .to_string())
                            .unwrap_or_default()
                    )))
                })
                .when_some(state.feedback.clone(), |body, message| {
                    body.child(note(message))
                })
                .when(
                    matches!(
                        self.extensions_ui.engine,
                        Provider::Qoder | Provider::Gemini
                    ),
                    |body| body.child(note(text("read_only"))),
                )
                .when_some(self.extensions_ui.feedback.clone(), |body, message| {
                    body.child(note(message))
                })
                .when(state.loading || state.preparing, |body| {
                    body.child(
                        h_flex().gap_2().child(note(text("loading"))).child(
                            resource_button("catalog-cancel")
                                .small()
                                .label(text("cancel"))
                                .on_click(move |_, _, cx| {
                                    cancel_view.update(cx, |app, cx| {
                                        app.extensions_ui.discovery[index].cancel();
                                        cx.notify();
                                    });
                                }),
                        ),
                    )
                }),
        );
        let mut entries = settings_scroll_area("extension-catalog-list");
        let mut resources = resource_list();
        if state.entries.is_empty() && !state.loading && state.feedback.is_none() {
            entries = entries.child(note(text("empty")));
        }
        for (position, item) in state.entries.iter().enumerate() {
            let item = item.clone();
            let item_view = view.clone();
            let link = item.source_url.clone();
            let installed = state.target.as_ref()
                == Some(&(
                    self.extensions_ui.engine,
                    self.extensions_ui.workspace.clone(),
                ))
                && state.installed.contains(&item.id);
            resources = resources.child(
                resource_row()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                resource_meta(
                                    ("catalog-resource-name", position),
                                    item.name.clone(),
                                )
                                .text_size(ui_text_size(14.))
                                .text_color(rgb(palette().text))
                                .font_semibold(),
                            )
                            .child(note(item.version().to_owned()))
                            .when(installed, |row| {
                                row.child(resource_badge(text("installed"), false))
                            }),
                    )
                    .child(note(item.description.clone()))
                    .child(
                        div()
                            .id(("catalog-resource-source", position))
                            .min_w_0()
                            .truncate()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .tooltip({
                                let source = item.source_url.clone();
                                move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(source.clone())
                                        .build(window, cx)
                                }
                            })
                            .child(conduit::i18n::format(
                                "settings.extensions.source",
                                &[("arg0", item.source_url.clone())],
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                resource_button(("catalog-detail", position))
                                    .debug_selector(move || format!("catalog-detail-{position}"))
                                    .small()
                                    .outline()
                                    .label(text(if installed { "installed" } else { "details" }))
                                    .disabled(state.preparing || self.closing || installed)
                                    .on_click(move |_, window, cx| {
                                        item_view.update(cx, |app, cx| {
                                            app.open_catalog_item(index, item.clone(), window, cx)
                                        });
                                    }),
                            )
                            .child(
                                resource_button(("catalog-source-link", position))
                                    .small()
                                    .ghost()
                                    .label(text("source_page"))
                                    .on_click(move |_, _, cx| {
                                        if catalog::mcp::safe_url(&link) {
                                            cx.open_url(&link);
                                        }
                                    }),
                            ),
                    ),
            );
        }
        if !state.entries.is_empty() {
            entries = entries.child(resources);
        }
        if state.next.is_some() {
            let more_view = view;
            entries = entries.child(
                resource_button("catalog-more")
                    .label(text("more"))
                    .disabled(state.loading)
                    .on_click(move |_, _, cx| {
                        more_view.update(cx, |app, cx| {
                            app.query_extension_catalog(index, true, false, cx)
                        });
                    }),
            );
        }
        body = body.child(entries);
        body.into_any_element()
    }
    fn open_catalog_item(
        &mut self,
        index: usize,
        item: Item,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing || self.extensions_ui.discovery[index].preparing {
            return;
        }
        let engine = self.extensions_ui.engine;
        let workspace = self.extensions_ui.workspace.clone();
        let kind = if index == 0 { Kind::Skill } else { Kind::Mcp };
        let destination = crate::extensions::edit::destination(engine, workspace.as_deref(), kind);
        let state = &mut self.extensions_ui.discovery[index];
        state.cancel();
        state.preparing = true;
        state.feedback = None;
        let generation = state.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        state.cancel = Some(cancel.clone());
        cx.spawn_in(window, async move |this, cx| {
            let prepared_item = item.clone();
            let read_path = destination.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    match prepared_item.payload {
                        Payload::Skill(_) => {
                            catalog::prepare_skill(&catalog::Client::new(&cancel), &prepared_item)
                                .map(|skill| dialogs::Prepared::Skill(Box::new(skill)))
                        }
                        Payload::Mcp(_) => {
                            crate::extensions::edit::read(&read_path).map(|original| {
                                dialogs::Prepared::Mcp(
                                    original,
                                    catalog::mcp::npm_runtime_available(),
                                )
                            })
                        }
                    }
                })
                .await;
            let _ = this.update_in(cx, |app, window, cx| {
                let state = &mut app.extensions_ui.discovery[index];
                if app.closing || state.generation != generation {
                    return;
                }
                state.preparing = false;
                state.cancel = None;
                if app.extensions_ui.engine != engine
                    || app.extensions_ui.workspace != workspace
                    || !state.discover
                    || app.settings_ui.page != if index == 0 { 4 } else { 9 }
                {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(prepared) => dialogs::show(
                        cx.entity(),
                        dialogs::Installation {
                            engine,
                            workspace,
                            destination,
                            item,
                            prepared,
                        },
                        window,
                        cx,
                    ),
                    Err(error) => {
                        app.extensions_ui.discovery[index].feedback = Some(error.to_string())
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
fn note(value: String) -> impl IntoElement {
    div()
        .text_size(ui_text_size(12.))
        .text_color(rgb(palette().muted))
        .child(value)
}
