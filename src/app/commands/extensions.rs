//! Read-only discovery for a captured composer target, independent of settings and runtimes.
use super::*;
use crate::extensions::{Catalog, Context as ExtensionContext, Entry, Kind, Probe};
use std::sync::mpsc::TryRecvError;

impl ConduitApp {
    pub(super) fn chat_extension_context(&self, command: &CommandContext) -> ExtensionContext {
        let engine = self.composer_provider();
        ExtensionContext {
            engine,
            workspace: Some(command.workspace.clone()),
            cli_path: cli::resolve(engine, &self.store.cli)
                .ok()
                .map(|cli| cli.path),
        }
    }
}

pub(super) struct ExtensionDialog {
    context: ExtensionContext,
    kind: Kind,
    search: Entity<InputState>,
    catalog: Catalog,
    rows: Arc<Vec<Row>>,
    count: usize,
    list: ListState,
    probe: Option<Probe>,
    task: Option<Task<()>>,
    closed: bool,
    _search_subscription: Subscription,
}

enum Row {
    Entry(Entry),
    Error(String),
    Empty(&'static str),
}

pub(super) fn show(
    context: ExtensionContext,
    kind: Kind,
    probe: Probe,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ExtensionDialog> {
    let dialog = cx.new(|cx| ExtensionDialog::new(context, kind, window, cx));
    dialog.update(cx, |dialog, cx| dialog.load(probe, cx));
    let view = dialog.clone();
    let initial_focus = std::cell::Cell::new(true);
    window.open_dialog(cx, move |builder, window, cx| {
        if initial_focus.replace(false) {
            view.read(cx)
                .search
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        }
        let close = view.clone();
        let body = view.clone();
        builder
            .title(conduit::i18n::format(
                match kind {
                    Kind::Skill => "commands.extensions.skills_title",
                    Kind::Mcp => "commands.extensions.mcp_title",
                },
                &[("engine", view.read(cx).context.engine.label().to_owned())],
            ))
            .w((window.viewport_size().width - px(48.)).clamp(px(0.), px(620.)))
            .margin_top(px(32.))
            .on_ok(|_, _, _| false)
            .on_close(move |_, _, cx| close.update(cx, |dialog, _| dialog.close()))
            .content(move |container, _, _| container.child(body.clone()))
    });
    dialog
}

impl ExtensionDialog {
    fn new(
        context: ExtensionContext,
        kind: Kind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(conduit::i18n::text_static("commands.extensions.search"))
        });
        let subscription = cx.subscribe(&search, |dialog, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                dialog.filter(cx);
                cx.notify();
            }
        });
        Self {
            context,
            kind,
            search,
            catalog: Catalog::default(),
            rows: Arc::default(),
            count: 0,
            list: ListState::new(0, ListAlignment::Top, px(100.)),
            probe: None,
            task: None,
            closed: false,
            _search_subscription: subscription,
        }
    }

    fn close(&mut self) {
        self.closed = true;
        self.task = None;
        // Dropping discovery cancels its worker. There is no MCP connection probe here.
        self.probe = None;
    }

    fn load(&mut self, probe: Probe, cx: &mut Context<Self>) {
        if self.closed || probe.context != self.context {
            return;
        }
        self.task = None;
        self.probe = Some(probe);
        self.catalog = Catalog::default();
        self.filter(cx);
        self.task = Some(cx.spawn(async move |this, cx| {
            loop {
                if this
                    .update(cx, |dialog, cx| dialog.poll(cx))
                    .unwrap_or(true)
                {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
            }
        }));
        cx.notify();
    }

    /// True when this discovery has completed or the dialog has been dismissed.
    fn poll(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(probe) = &self.probe else {
            return true;
        };
        let catalog = match probe.events.try_recv() {
            Ok(catalog) => catalog,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Catalog {
                errors: vec![conduit::i18n::text(
                    "settings.extensions.scan_thread_exited_refresh_to_retry",
                )],
                ..Default::default()
            },
        };
        self.probe = None;
        self.catalog = catalog;
        self.filter(cx);
        cx.notify();
        true
    }

    fn filter(&mut self, cx: &App) {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut entries: Vec<_> = self
            .catalog
            .entries
            .iter()
            .filter(|entry| {
                entry.kind == self.kind
                    && (query.is_empty()
                        || [&entry.name, &entry.description, &entry.scope, &entry.target]
                            .iter()
                            .any(|value| value.to_lowercase().contains(&query))
                        || display_path(&entry.path).to_lowercase().contains(&query)
                        || conduit::i18n::source(&entry.scope)
                            .to_lowercase()
                            .contains(&query))
            })
            .cloned()
            .collect();
        entries.sort_by_key(|entry| (entry.name.to_lowercase(), entry.path.clone()));
        self.count = entries.len();
        let mut rows: Vec<_> = self
            .catalog
            .errors
            .iter()
            .cloned()
            .map(Row::Error)
            .collect();
        rows.extend(entries.into_iter().map(Row::Entry));
        if self.count == 0 && self.probe.is_none() {
            rows.push(Row::Empty(if !query.is_empty() {
                "commands.extensions.no_matches"
            } else if !self.catalog.errors.is_empty() {
                "commands.extensions.incomplete"
            } else {
                match self.kind {
                    Kind::Skill => "commands.extensions.no_skills",
                    Kind::Mcp => "commands.extensions.no_mcp",
                }
            }));
        }
        self.list = ListState::new(rows.len(), ListAlignment::Top, px(100.));
        self.rows = Arc::new(rows);
    }
}

impl Render for ExtensionDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        let loading = self.probe.is_some();
        let height = (window.viewport_size().height - px(320.)).clamp(px(80.), px(360.));
        v_flex()
            .id("chat-extension-dialog")
            .debug_selector(|| "chat-extension-dialog".into())
            .w_full()
            .gap_3()
            .child(path_note(
                "chat-extensions-workspace",
                self.context
                    .workspace
                    .as_deref()
                    .map(display_path)
                    .unwrap_or_default(),
            ))
            .child(note(conduit::i18n::text_static(
                "commands.extensions.snapshot",
            )))
            .child(
                Input::new(&self.search)
                    .aria_label(conduit::i18n::text_static("commands.extensions.search")),
            )
            .child(
                div()
                    .id("chat-extension-list")
                    .debug_selector(|| "chat-extension-list".into())
                    .h(height)
                    .w_full()
                    .when(loading, |body| {
                        body.child(h_flex().gap_2().p_3().child(Spinner::new()).child(note(
                            conduit::i18n::text_static("commands.extensions.loading"),
                        )))
                    })
                    .when(!loading, |body| {
                        body.child(
                            list(self.list.clone(), move |index, _, _| {
                                render_row(&rows[index], index).into_any_element()
                            })
                            .size_full(),
                        )
                        .vertical_scrollbar(&self.list)
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(note(if loading {
                        String::new()
                    } else {
                        conduit::i18n::format(
                            "commands.extensions.count",
                            &[("count", self.count.to_string())],
                        )
                    }))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("chat-extensions-refresh")
                                    .debug_selector(|| "chat-extensions-refresh".into())
                                    .small()
                                    .outline()
                                    .label(conduit::i18n::text_static("catalog.refresh"))
                                    .disabled(loading)
                                    .on_click(cx.listener(|dialog, _, _, cx| {
                                        dialog.load(Probe::start(dialog.context.clone()), cx);
                                    })),
                            )
                            .child(
                                Button::new("chat-extensions-close")
                                    .debug_selector(|| "chat-extensions-close".into())
                                    .small()
                                    .outline()
                                    .label(conduit::i18n::text_static(
                                        "settings.extensions.close_tools",
                                    ))
                                    .on_click(cx.listener(|dialog, _, window, cx| {
                                        dialog.close();
                                        window.close_dialog(cx);
                                    })),
                            ),
                    ),
            )
    }
}

fn note(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(ui_text_size(12.))
        .text_color(rgb(palette().muted))
        .whitespace_normal()
        .child(text.into())
}

fn path_note(id: impl Into<ElementId>, path: String) -> Stateful<Div> {
    let hint = path.clone();
    note(path)
        .id(id)
        .min_w_0()
        .truncate()
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(hint.clone()).build(window, cx)
        })
}

fn render_row(row: &Row, index: usize) -> Div {
    match row {
        Row::Entry(entry) => v_flex()
            .w_full()
            .min_w_0()
            .px_2()
            .py_3()
            .gap_1()
            .border_b_1()
            .border_color(rgb(palette().border))
            .child(
                div()
                    .font_semibold()
                    .whitespace_normal()
                    .child(entry.name.clone()),
            )
            .child(note(format!(
                "{} · {}",
                conduit::i18n::source(&entry.scope),
                entry.status()
            )))
            .child(note(match entry.kind {
                Kind::Skill => entry.description.clone(),
                Kind::Mcp => format!("{} · {}", entry.transport, entry.target),
            }))
            .child(path_note(
                ("chat-extension-path", index),
                display_path(&entry.path),
            )),
        Row::Error(message) => note(message.clone())
            .p_2()
            .text_color(rgb(palette().warning_text)),
        Row::Empty(key) => note(conduit::i18n::text_static(key)).p_3(),
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
