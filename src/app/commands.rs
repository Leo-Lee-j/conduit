//! Composer interaction and application bridge for source-owned chat commands.
use super::*;
use crate::commands::{
    self as core, Catalog, CommandContext, CommandId, Descriptor, Execution, HostAction, Query,
    RequestIntent, TurnInput,
};
use std::hash::{Hash, Hasher};

#[derive(Default)]
pub(super) struct ChatCommands {
    pub catalog: Catalog,
    executables: HashMap<Provider, (Option<PathBuf>, u64)>,
    active: Option<(CommandContext, Query)>,
    dismissed: Option<(CommandContext, Query)>,
    results: Vec<Descriptor>,
    selected: Option<CommandId>,
    accepted: Option<(CommandContext, Descriptor)>,
    scroll: ScrollHandle,
}

impl ConduitApp {
    /// Called only at launch validation. Composer queries use this cached identity.
    pub(super) fn remember_command_executable(
        &mut self,
        provider: Provider,
        path: &Path,
    ) -> Result<(), String> {
        let metadata =
            std::fs::metadata(path).map_err(|_| conduit::i18n::text("commands.stale"))?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        path.canonicalize()
            .map_err(|_| conduit::i18n::text("commands.stale"))?
            .hash(&mut hash);
        metadata.len().hash(&mut hash);
        metadata.modified().ok().hash(&mut hash);
        self.chat_commands.executables.insert(
            provider,
            (
                self.store.cli.path(provider).map(Path::to_path_buf),
                hash.finish(),
            ),
        );
        Ok(())
    }

    pub(super) fn command_context_for(
        &self,
        index: usize,
        target: Option<u64>,
        provider: Provider,
    ) -> Option<CommandContext> {
        let workspace = self.store.workspaces.get(index)?;
        let session = target.and_then(|id| workspace.sessions.iter().find(|s| s.id == id));
        if target.is_some() && session.is_none() {
            return None;
        }
        let provider = session.map_or(provider, |s| s.provider);
        let selection = session
            .map(ComposerSelection::from_session)
            .unwrap_or_else(|| self.composer_selection(self.selected_session.unwrap_or_default()));
        let permission = session.map_or(self.pending_permission, |s| s.permission);
        // Only hash in-memory configuration here: typing and repainting do no I/O.
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_vec(&(
            self.store.cli.path(provider),
            self.store.cli.enabled(provider),
            &self.store.providers,
            &selection.channel,
            &selection.model,
            selection.effort,
            &selection.opencode.variant,
            permission,
        ))
        .ok()?
        .hash(&mut hash);
        self.diagnostics.get(&provider).hash(&mut hash);
        self.chat_commands
            .executables
            .get(&provider)
            .filter(|(configured, _)| configured.as_deref() == self.store.cli.path(provider))
            .map(|(_, identity)| identity)
            .hash(&mut hash);
        let engine = provider.descriptor();
        Some(CommandContext {
            workspace: workspace.path.clone(),
            session: target,
            native_id: session.and_then(|s| s.native_id.clone()),
            source: engine.commands.map_or(engine.id, |source| source.id).into(),
            registration: engine.commands.map_or(0, |source| source.registration),
            configuration: hash.finish(),
        })
    }

    fn chat_command_context(&self) -> Option<CommandContext> {
        self.command_context_for(
            self.selected_workspace?,
            self.send_target(true),
            self.new_session_provider,
        )
    }

    pub(super) fn session_command_context(&self, id: u64) -> Option<CommandContext> {
        let index = self
            .store
            .workspaces
            .iter()
            .position(|w| w.sessions.iter().any(|s| s.id == id))?;
        self.command_context_for(index, Some(id), self.session(id)?.provider)
    }

    fn chat_command_query(&self, cx: &App) -> Option<(CommandContext, Query)> {
        if self.closing || !self.writable {
            return None;
        }
        let input = self.input.read(cx);
        Some((
            self.chat_command_context()?,
            core::query_at(&input.value(), input.selected_range())?,
        ))
    }

    fn command_items(&self, context: &CommandContext) -> Vec<Descriptor> {
        let mut items = core::host_commands();
        if let Some(snapshot) = self.chat_commands.catalog.get(context) {
            items.extend(snapshot.commands.iter().cloned());
        }
        items
    }

    pub(super) fn refresh_chat_commands(&mut self, cx: &mut Context<Self>) {
        let query = self.chat_command_query(cx);
        if query != self.chat_commands.dismissed {
            self.chat_commands.dismissed = None;
        }
        let active = query.filter(|query| Some(query) != self.chat_commands.dismissed.as_ref());
        let results = active
            .as_ref()
            .map(|(context, query)| core::filter(self.command_items(context), &query.name))
            .unwrap_or_default();
        let changed = active != self.chat_commands.active || results != self.chat_commands.results;
        self.chat_commands.active = active;
        self.chat_commands.results = results;
        if !self
            .chat_commands
            .results
            .iter()
            .any(|item| Some(&item.id) == self.chat_commands.selected.as_ref())
        {
            self.chat_commands.selected = self
                .chat_commands
                .results
                .first()
                .map(|item| item.id.clone());
        }
        if let Some((_, accepted)) = &self.chat_commands.accepted
            && core::parse(&self.input.read(cx).value())
                .is_none_or(|query| !accepted.matches_name(&query.name))
        {
            self.chat_commands.accepted = None;
        }
        if changed {
            cx.notify();
        }
    }

    pub(super) fn chat_commands_open(&self, window: &Window, cx: &App) -> bool {
        self.chat_commands.active.is_some()
            && !self.chat_commands.results.is_empty()
            && self.chat_commands.active == self.chat_command_query(cx)
            && self.input.read(cx).focus_handle(cx).is_focused(window)
    }

    pub(super) fn dismiss_chat_commands(&mut self, cx: &mut Context<Self>) {
        self.chat_commands.dismissed = self.chat_commands.active.take();
        cx.notify();
    }

    pub(super) fn move_chat_command(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_is_composing(window, cx) {
            return;
        }
        let items = &self.chat_commands.results;
        if items.is_empty() {
            return;
        }
        let index = items
            .iter()
            .position(|item| Some(&item.id) == self.chat_commands.selected.as_ref())
            .unwrap_or(0);
        let next = (index as isize + delta).rem_euclid(items.len() as isize) as usize;
        self.chat_commands.selected = Some(items[next].id.clone());
        self.chat_commands.scroll.scroll_to_item(next);
        cx.notify();
    }

    pub(super) fn accept_chat_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.composer_is_composing(window, cx) {
            return false;
        }
        let Some((context, query)) = self.chat_commands.active.clone() else {
            return false;
        };
        if Some((context.clone(), query.clone())) != self.chat_command_query(cx) {
            return false;
        }
        let Some(item) = self
            .chat_commands
            .results
            .iter()
            .find(|item| Some(&item.id) == self.chat_commands.selected.as_ref())
            .cloned()
        else {
            return false;
        };
        let (text, caret) = core::complete(&self.input.read(cx).value(), &query, &item);
        self.chat_commands.accepted = Some((context, item));
        self.dismiss_chat_commands(cx);
        self.input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_selected_range(caret..caret, cx);
        });
        cx.notify();
        true
    }

    /// True means this submission is owned by the command path, including errors.
    pub(super) fn submit_chat_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let text = self.input.read(cx).value().to_string();
        let Some(query) = core::parse(&text) else {
            return false;
        };
        let result = self.prepare_chat_submission(&text, &query);
        match result {
            Ok(None) => return false,
            Ok(Some((command, context))) => {
                if self
                    .current_files()
                    .is_some_and(|files| !files.references.is_empty())
                {
                    self.message = Some(conduit::i18n::text("commands.attachments"));
                } else if let Some(reason) = &command.unavailable {
                    self.message = Some(reason.clone());
                } else if !command.accepts(&text[query.range.end..]) {
                    self.message = Some(conduit::i18n::text("commands.arguments"));
                } else {
                    match command.execution {
                        Execution::Host(action) => {
                            let sent = self.sent_composer(cx);
                            match action {
                                HostAction::Settings => self.open_settings(window, cx),
                                HostAction::NewChat => {
                                    if let Some(index) = self.selected_workspace {
                                        // Opening a draft is navigation; the existing send
                                        // gate creates and persists its session on first use.
                                        self.switch(index, None, window, cx);
                                    }
                                }
                            }
                            if let Some(sent) = sent {
                                self.clear_saved_composer(&sent, window, cx);
                            }
                            self.chat_commands.accepted = None;
                            self.dismiss_chat_commands(cx);
                        }
                        Execution::Engine => {
                            match self.chat_commands.catalog.prepare(
                                &context,
                                &command,
                                text.clone(),
                            ) {
                                Ok(call) => {
                                    self.send_input(
                                        TurnInput {
                                            text,
                                            intent: RequestIntent::Command(Box::new(call)),
                                        },
                                        true,
                                        window,
                                        cx,
                                    );
                                }
                                Err(error) => self.message = Some(error),
                            }
                        }
                    }
                }
            }
            Err(error) => self.message = Some(error),
        }
        cx.notify();
        true
    }

    fn prepare_chat_submission(
        &self,
        text: &str,
        query: &Query,
    ) -> Result<Option<(Descriptor, CommandContext)>, String> {
        let context = self
            .chat_command_context()
            .ok_or_else(|| conduit::i18n::text("commands.stale"))?;
        if let Some((captured, item)) = &self.chat_commands.accepted
            && item.matches_name(&query.name)
        {
            if captured != &context {
                return Err(conduit::i18n::text("commands.stale"));
            }
            return Ok(Some((item.clone(), context)));
        }
        let mut exact = self
            .command_items(&context)
            .into_iter()
            .filter(|item| item.matches_name(&query.name));
        let Some(command) = exact.next() else {
            return Ok(None);
        };
        if exact.next().is_some() {
            return Err(conduit::i18n::text("commands.ambiguous"));
        }
        if text.len() > 256 * 1024 {
            return Err(conduit::i18n::text("commands.arguments"));
        }
        Ok(Some((command, context)))
    }

    fn connect_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_is_composing(window, cx)
            || self.new_session_provider.descriptor().commands.is_none()
        {
            return;
        }
        self.send_input(
            TurnInput {
                text: conduit::i18n::text("commands.connecting"),
                intent: RequestIntent::Discover,
            },
            true,
            window,
            cx,
        );
    }

    pub(super) fn render_chat_commands(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let context = &self.chat_commands.active.as_ref().unwrap().0;
        let snapshot = self.chat_commands.catalog.get(context);
        let engine = self.new_session_provider.descriptor();
        let busy = self.send_blocked(true);
        let state = if engine.commands.is_none() {
            conduit::i18n::text("commands.unsupported")
        } else if let Some(error) = snapshot.and_then(|s| s.error.clone()) {
            error
        } else if snapshot.is_some_and(|s| s.loading) {
            conduit::i18n::text("commands.loading")
        } else if snapshot.is_some_and(|s| s.loaded) {
            conduit::i18n::text("commands.keys")
        } else {
            conduit::i18n::text("commands.needs_connection")
        };
        let rows = self
            .chat_commands
            .results
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let selected = Some(&item.id) == self.chat_commands.selected.as_ref();
                let id = item.id.clone();
                let hint = match &item.arguments {
                    core::Arguments::None => String::new(),
                    core::Arguments::TailText { hint, .. } if !hint.is_empty() => hint.clone(),
                    _ => conduit::i18n::text("commands.optional_arguments"),
                };
                let unavailable = if item.execution == Execution::Engine {
                    busy
                } else {
                    None
                };
                let detail = item
                    .unavailable
                    .clone()
                    .or_else(|| unavailable.map(str::to_owned))
                    .unwrap_or_else(|| item.summary.clone());
                let label = format!("/{}  {} · {}", item.name, hint, item.id.source);
                v_flex()
                    .id(("chat-command", index))
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{label}. {detail}"))
                    .debug_selector(move || format!("chat-command-{index}"))
                    .min_w_0()
                    .w_full()
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(rgb(palette().surface_hover)))
                    .hover(|row| row.bg(rgb(palette().surface_hover)))
                    .child(div().truncate().text_size(ui_text_size(12.)).child(label))
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(detail.clone()),
                    )
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(detail.clone()).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.chat_commands.selected = Some(id.clone());
                        this.accept_chat_command(window, cx);
                    }))
            });
        let menu = v_flex()
            .id("chat-command-menu")
            .debug_selector(|| "chat-command-menu".into())
            .role(Role::ListBox)
            .aria_label(conduit::i18n::text_static("commands.title"))
            .w((window.viewport_size().width - px(32.)).min(px(540.)))
            .min_w_0()
            .p_1()
            .gap_1()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .text_color(rgb(palette().text))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_chat_commands(cx)))
            .child(
                div()
                    .px_2()
                    .text_size(ui_text_size(12.))
                    .child(conduit::i18n::text_static("commands.title")),
            )
            .child(
                v_flex()
                    .id("chat-command-rows")
                    .max_h((window.viewport_size().height * 0.4).min(px(260.)))
                    .overflow_y_scroll()
                    .track_scroll(&self.chat_commands.scroll)
                    .children(rows),
            )
            .child(
                div()
                    .px_2()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(state),
            )
            .when(engine.commands.is_some(), |menu| {
                menu.child(
                    Button::new("connect-chat-commands")
                        .ghost()
                        .small()
                        .label(conduit::i18n::text_static("commands.connect"))
                        .disabled(busy.is_some())
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.connect_commands(window, cx);
                        })),
                )
            });
        deferred(
            gpui_kit::base::Positioner::side(
                self.input
                    .read(cx)
                    .cursor_layout()
                    .map(|(mut bounds, _)| {
                        bounds.origin.y += self.input.read(cx).scroll_offset().y;
                        bounds
                    })
                    .unwrap_or_default(),
            )
            .placement(gpui_kit::base::Placement::Top)
            .align(gpui_kit::base::Align::Start)
            .offset(px(8.))
            .margin(px(8.))
            .occlude()
            .child(menu),
        )
        .with_priority(gpui_kit::base::POPUP_PRIORITY)
        .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
