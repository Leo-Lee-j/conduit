//! Child navigation with independent editors. Routes never own the parent connection.
use super::*;
use gpui_kit::component::skeleton::Skeleton;
mod controls;
mod editor;
mod history;
pub(in crate::app) use editor::{SavedEditor, SentChild};

#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub(in crate::app) struct TaskKey {
    pub root: u64,
    pub run: String,
    pub agent: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Target {
    Summary,
    SavedTranscript(u64),
}

struct Page {
    target: Target,
    focus: FocusHandle,
    scroll: gpui_kit::ScrollHandle,
    messages: ListState,
    count: usize,
    used: u64,
    restore: Option<(usize, f32)>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(in crate::app) struct SavedPage {
    key: TaskKey,
    item: usize,
    offset: f32,
}

#[derive(Default)]
pub(in crate::app) struct Navigation {
    routes: HashMap<u64, TaskKey>,
    pages: HashMap<TaskKey, Page>,
    positions: HashMap<TaskKey, (usize, f32)>,
    clock: u64,
    reads: HashMap<TaskKey, history::Load>,
    final_reads: HashSet<TaskKey>,
    history_errors: HashMap<TaskKey, String>,
    refreshed: HashMap<TaskKey, Instant>,
    editors: HashMap<editor::EditorKey, editor::Editor>,
    recovered: Vec<editor::SavedEditor>,
    live: HashMap<(u64, String), history::Live>,
    commands: HashMap<TaskKey, controls::Pending>,
}

impl Navigation {
    pub(in crate::app) fn show_root(&mut self, root: u64) {
        self.routes.remove(&root);
    }

    pub(in crate::app) fn forget_root(&mut self, root: u64) {
        self.routes.remove(&root);
        self.pages.retain(|key, _| key.root != root);
        self.positions.retain(|key, _| key.root != root);
        self.reads.retain(|key, _| key.root != root);
        self.final_reads.retain(|key| key.root != root);
        self.history_errors.retain(|key, _| key.root != root);
        self.refreshed.retain(|key, _| key.root != root);
        self.editors.retain(|key, _| key.root != root);
        self.recovered.retain(|entry| entry.key.root != root);
        self.live.retain(|(id, _), _| *id != root);
        self.commands.retain(|key, _| key.root != root);
    }
}

impl ConduitApp {
    pub(in crate::app) fn remeasure_subagent_message(&self, id: u64, index: usize) {
        if let Some(page) = self
            .session_control
            .agent_navigation
            .pages
            .values()
            .find(|page| page.target == Target::SavedTranscript(id))
            && index < page.count
        {
            page.messages.remeasure_items(index..index + 1);
        }
    }

    pub(in crate::app) fn finish_child_observation(&mut self, root: u64) {
        let keys: Vec<_> = self
            .session_control
            .agent_navigation
            .refreshed
            .keys()
            .filter(|key| key.root == root)
            .cloned()
            .collect();
        self.session_control
            .agent_navigation
            .final_reads
            .extend(keys);
    }

    pub(in crate::app) fn saved_child_pages(&self) -> Vec<SavedPage> {
        let navigation = &self.session_control.agent_navigation;
        let mut positions = navigation.positions.clone();
        for (key, page) in &navigation.pages {
            let top = page.messages.logical_scroll_top();
            positions.insert(
                key.clone(),
                page.restore
                    .unwrap_or((top.item_ix, f32::from(top.offset_in_item))),
            );
        }
        positions
            .into_iter()
            .map(|(key, (item, offset))| SavedPage { key, item, offset })
            .collect()
    }

    pub(in crate::app) fn saved_child_routes(&self) -> Vec<TaskKey> {
        self.session_control
            .agent_navigation
            .routes
            .values()
            .cloned()
            .collect()
    }

    pub(in crate::app) fn restore_child_navigation(
        &mut self,
        editors: Vec<SavedEditor>,
        routes: Vec<TaskKey>,
        pages: Vec<SavedPage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.session_control.agent_navigation.routes.clear();
        self.session_control.agent_navigation.pages.clear();
        self.session_control.agent_navigation.positions.clear();
        self.session_control.agent_navigation.editors.clear();
        self.session_control.agent_navigation.recovered = editors;
        for key in routes
            .iter()
            .cloned()
            .chain(pages.iter().map(|page| page.key.clone()))
        {
            if self.routed_agent(&key).is_none() {
                continue;
            }
            let target = self
                .saved_child(&key)
                .map_or(Target::Summary, Target::SavedTranscript);
            self.session_control
                .agent_navigation
                .pages
                .entry(key.clone())
                .or_insert_with(|| Page {
                    target,
                    focus: cx.focus_handle(),
                    scroll: gpui_kit::ScrollHandle::new(),
                    messages: ListState::new(0, ListAlignment::Bottom, px(240.)),
                    count: 0,
                    used: 0,
                    restore: pages.iter().find(|page| page.key == key).map(|page| {
                        (
                            page.item,
                            if page.offset.is_finite() {
                                page.offset
                            } else {
                                0.
                            },
                        )
                    }),
                });
            if routes.contains(&key) {
                self.session_control
                    .agent_navigation
                    .routes
                    .insert(key.root, key);
            }
        }
        self.ensure_child_editor(window, cx);
    }

    pub(in crate::app) fn subagent_route(&self) -> Option<&TaskKey> {
        if self.settings_ui.open
            || self.active_file.is_some()
            || self.active_diff.is_some()
            || self.usage_ui.active.is_some()
            || self.github_ui.active.is_some()
        {
            return None;
        }
        self.session_control
            .agent_navigation
            .routes
            .get(&self.selected_session?)
    }

    pub(in crate::app) fn viewing_subagent(&self) -> bool {
        self.subagent_route().is_some()
    }

    fn routed_agent(&self, key: &TaskKey) -> Option<&Agent> {
        self.session(key.root)?
            .subagents
            .iter()
            .find(|agent| agent.run == key.run && agent.id == key.agent)
    }

    // Only these adapters expose native conversation IDs as agent IDs. Claude
    // task/tool-use IDs must never be interpreted as conversation IDs.
    fn child_native_id(&self, key: &TaskKey) -> Option<String> {
        let root = self.session(key.root)?;
        match root.provider {
            Provider::Claude => self.routed_agent(key)?.conversation_id.clone(),
            Provider::Codex | Provider::OpenCode => Some(key.agent.clone()),
            Provider::Qoder => None,
        }
    }

    fn saved_child(&self, key: &TaskKey) -> Option<u64> {
        let root = self.session(key.root)?;
        let native_child = self.child_native_id(key)?;
        let native_root = root.native_id.as_deref().filter(|id| !id.is_empty())?;
        let mut agent = self.routed_agent(key)?;
        let mut seen = HashSet::new();
        while agent.parent_id != native_root {
            if agent.parent_id.is_empty() || !seen.insert(agent.id.as_str()) {
                return None;
            }
            agent = root
                .subagents
                .iter()
                .find(|parent| parent.run == key.run && parent.id == agent.parent_id)?;
        }
        let (workspace, _) = self
            .session_control
            .locate(&self.store.workspaces, key.root)?;
        let mut candidates = self.store.workspaces[workspace]
            .sessions
            .iter()
            .filter(|child| {
                child.is_subagent
                    && child.id != key.root
                    && child.provider == root.provider
                    && child.native_id.as_deref() == Some(native_child.as_str())
                    && (root.provider != Provider::Claude
                        || self
                            .store
                            .child_bindings
                            .get(&child.id)
                            .is_some_and(|binding| {
                                binding.root == key.root
                                    && Some(&binding.root_native) == root.native_id.as_ref()
                            }))
            });
        let id = candidates.next()?.id;
        if candidates.next().is_some() {
            return None;
        }
        if let Some(binding) = self.store.child_bindings.get(&id)
            && (binding.root != key.root
                || binding.root_native != native_root
                || self.child_source(key.root).as_ref() != Some(&binding.source))
        {
            return None;
        }
        Some(id)
    }

    pub(in crate::app) fn visible_subagent_transcript(&self) -> Option<u64> {
        let key = self.subagent_route()?;
        match self.session_control.agent_navigation.pages.get(key)?.target {
            Target::SavedTranscript(id) if self.saved_child(key) == Some(id) => Some(id),
            _ => None,
        }
    }

    pub(in crate::app) fn open_subagent(
        &mut self,
        root: u64,
        run: String,
        agent: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Reject clicks left over from another root/tab or an expired task row.
        if self.selected_session != Some(root)
            || self.settings_ui.open
            || self.active_file.is_some()
            || self.active_diff.is_some()
            || self.usage_ui.active.is_some()
            || self.github_ui.active.is_some()
        {
            return;
        }
        let key = TaskKey { root, run, agent };
        if self.routed_agent(&key).is_none() {
            return;
        }
        let target = self
            .saved_child(&key)
            .map_or(Target::Summary, Target::SavedTranscript);
        let navigation = &mut self.session_control.agent_navigation;
        navigation.clock = navigation.clock.wrapping_add(1);
        let used = navigation.clock;
        // Page caches contain layout/scroll state only; evicting them never
        // drops runtime handles, drafts or persisted conversation records.
        if navigation.pages.len() >= 64 && !navigation.pages.contains_key(&key) {
            let oldest = navigation
                .pages
                .iter()
                .filter(|(key, _)| navigation.routes.get(&key.root) != Some(key))
                .min_by_key(|(_, page)| page.used)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest
                && let Some(page) = navigation.pages.remove(&oldest)
            {
                let top = page.messages.logical_scroll_top();
                navigation.positions.insert(
                    oldest,
                    page.restore
                        .unwrap_or((top.item_ix, f32::from(top.offset_in_item))),
                );
            }
        }
        let restore = navigation.positions.remove(&key);
        let page = navigation.pages.entry(key.clone()).or_insert_with(|| Page {
            target,
            focus: cx.focus_handle(),
            scroll: gpui_kit::ScrollHandle::new(),
            messages: ListState::new(0, ListAlignment::Bottom, px(240.)),
            count: 0,
            used,
            restore,
        });
        page.used = used;
        // A summary can acquire a verified stored body on a later explicit open.
        if page.target == Target::Summary {
            page.target = target;
        }
        page.focus.focus(window, cx);
        self.session_control
            .agent_navigation
            .routes
            .insert(root, key);
        TextSelection::clear(window, cx);
        self.ensure_child_editor(window, cx);
        self.request_child_history(false, cx);
        self.dismiss_chat_commands(cx);
        if let Some(id) = self.visible_subagent_transcript() {
            self.request_transcript(id, cx);
        }
        cx.notify();
    }

    pub(in crate::app) fn return_from_subagent(
        &mut self,
        root: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_session != Some(root) {
            return;
        }
        if self.session(root).is_none() {
            // Keep the orphaned route read-only; never reinterpret its retained
            // root draft as a workspace's new conversation.
            cx.notify();
            return;
        }
        self.session_control.agent_navigation.show_root(root);
        self.session_control.unread_completed.remove(&root);
        TextSelection::clear(window, cx);
        // Keep the same input entity: no set_value, no loss of undo/selection.
        self.input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn focus_subagent_page(&self, window: &mut Window, cx: &mut App) {
        if let Some(key) = self.subagent_route()
            && let Some(page) = self.session_control.agent_navigation.pages.get(key)
        {
            page.focus.focus(window, cx);
        }
    }

    pub(in crate::app) fn render_subagent_page(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = self.subagent_route().cloned().expect("visible child route");
        let agent = self.routed_agent(&key).cloned();
        let label = agent
            .as_ref()
            .map(|agent| title(agent).0.to_owned())
            .unwrap_or_else(|| conduit::i18n::text("subagents.task_unavailable"));
        let saved = self.visible_subagent_transcript();
        if let Some(id) = saved {
            self.request_transcript(id, cx);
        }
        let focus = self.session_control.agent_navigation.pages[&key]
            .focus
            .clone();
        let root = key.root;
        let mut heading = h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .items_center()
            .child(
                Button::new("subagent-back")
                    .ghost()
                    .compact()
                    .flex_shrink_0()
                    .icon(IconName::ArrowLeft)
                    .label(conduit::i18n::text_static("subagents.back"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.return_from_subagent(root, window, cx)
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_semibold()
                    .child(label),
            );
        let siblings: Vec<_> = self
            .session(root)
            .into_iter()
            .flat_map(|session| &session.subagents)
            .filter(|agent| agent.run == key.run)
            .map(|agent| agent.id.clone())
            .collect();
        if let Some(index) = siblings.iter().position(|id| id == &key.agent) {
            for (name, next, icon) in [
                ("previous", index.checked_sub(1), IconName::ChevronLeft),
                (
                    "next",
                    (index + 1 < siblings.len()).then_some(index + 1),
                    IconName::ChevronRight,
                ),
            ] {
                let next = next.and_then(|index| siblings.get(index)).cloned();
                let run = key.run.clone();
                heading = heading.child(
                    Button::new(SharedString::from(format!("subagent-{name}")))
                        .ghost()
                        .compact()
                        .flex_shrink_0()
                        .icon(icon)
                        .disabled(next.is_none())
                        .accessibility_label(conduit::i18n::text(&format!("subagents.{name}")))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(agent) = &next {
                                this.open_subagent(root, run.clone(), agent.clone(), window, cx);
                            }
                        })),
                );
            }
        }
        if let Some(agent) = &agent {
            heading = heading.child(
                div()
                    .flex_shrink_0()
                    .text_color(rgb(status_color(agent.status)))
                    .child(agent.status.label()),
            );
        }
        heading = heading.child(
            Button::new("subagent-refresh")
                .ghost()
                .compact()
                .label(conduit::i18n::text_static("subagents.refresh"))
                .disabled(
                    self.session_control
                        .agent_navigation
                        .reads
                        .contains_key(&key),
                )
                .on_click(cx.listener(|this, _, _, cx| this.request_child_history(true, cx))),
        );
        let history_error = self
            .session_control
            .agent_navigation
            .history_errors
            .get(&key)
            .cloned();
        let editor = self.render_child_editor(&key, cx);
        let approvals = self.child_approvals(&key, window, cx);
        let body = if self.session(root).is_none() {
            div()
                .flex_1()
                .min_h_0()
                .p_4()
                .child(conduit::i18n::text_static("subagents.root_unavailable"))
                .into_any_element()
        } else if let Some(id) = saved {
            self.subagent_saved_body(&key, id, cx)
        } else {
            self.subagent_summary_body(&key, agent.as_ref(), cx)
        };
        v_flex()
            .id("subagent-page")
            .debug_selector(|| "subagent-page".into())
            .track_focus(&focus)
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(
                v_flex()
                    .flex_shrink_0()
                    .p_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(heading),
            )
            .when_some(history_error, |view, error| {
                view.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(rgb(palette().muted))
                        .child(error),
                )
            })
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(840.))
                    .mx_auto()
                    .min_h_0()
                    .flex_1()
                    .child(body),
            )
            .child(approvals)
            .child(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .child(div().w_full().max_w(px(840.)).mx_auto().child(editor)),
            )
            .into_any_element()
    }

    fn subagent_summary_body(
        &self,
        key: &TaskKey,
        agent: Option<&Agent>,
        _cx: &Context<Self>,
    ) -> AnyElement {
        let page = &self.session_control.agent_navigation.pages[key];
        let mut content = v_flex()
            .id("subagent-summary")
            .debug_selector(|| "subagent-summary".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&page.scroll)
            .p_4()
            .gap_3()
            .text_size(ui_text_size(13.));
        if let Some(agent) = agent {
            for (name, text) in [
                ("task", &agent.task),
                ("activity", &agent.activity),
                ("output", &agent.result),
            ] {
                if !text.is_empty() {
                    let identity = format!(
                        "child-summary-{}-{}-{}-{name}",
                        key.root, key.run, key.agent
                    );
                    content = content
                        .child(
                            div()
                                .font_semibold()
                                .child(conduit::i18n::text(&format!("subagents.{name}"))),
                        )
                        .child(chat_markdown_view(
                            identity.clone().into(),
                            text.clone(),
                            &self.chat_typography(),
                            &identity,
                            true,
                        ));
                }
            }
        }
        content.into_any_element()
    }

    fn subagent_saved_body(
        &mut self,
        key: &TaskKey,
        id: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let error = self.transcripts.errors.get(&id).cloned();
        let loaded = self
            .session(id)
            .is_some_and(|session| session.transcript.loaded);
        if !loaded || error.is_some() {
            if error.is_none() {
                return v_flex()
                    .id("subagent-history-skeleton")
                    .debug_selector(|| "subagent-history-skeleton".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .justify_center()
                    .items_center()
                    .child(
                        v_flex().w_full().max_w(px(560.)).gap_5().px_4().children(
                            [0.74, 0.88, 0.62]
                                .into_iter()
                                .enumerate()
                                .map(|(index, width)| {
                                    let mut message = v_flex()
                                        .debug_selector(move || {
                                            format!("subagent-history-skeleton-message-{index}")
                                        })
                                        .gap_2()
                                        .w(relative(width));
                                    for line in [0.94, 0.78, 0.58] {
                                        message = message.child(
                                            Skeleton::new()
                                                .h(px(12.))
                                                .w(relative(line))
                                                .rounded(px(4.)),
                                        );
                                    }
                                    message
                                }),
                        ),
                    )
                    .into_any_element();
            }
            let mut body = v_flex()
                .flex_1()
                .min_h_0()
                .p_4()
                .gap_2()
                .child(error.clone().unwrap());
            if error.is_some() {
                let expected = key.clone();
                body = body.child(
                    Button::new("subagent-retry-history")
                        .ghost()
                        .label(conduit::i18n::text_static("subagents.retry_history"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.subagent_route() == Some(&expected)
                                && this.visible_subagent_transcript() == Some(id)
                            {
                                this.retry_transcript(id, cx);
                            }
                        })),
                );
            }
            return body.into_any_element();
        }
        let count = self.session(id).map_or(0, |session| session.messages.len());
        let page = self
            .session_control
            .agent_navigation
            .pages
            .get_mut(key)
            .unwrap();
        if page.count != count {
            let first_load = page.count == 0;
            if count >= page.count {
                page.messages
                    .splice(page.count..page.count, count - page.count);
            } else {
                page.messages.splice(count..page.count, 0);
            }
            page.count = count;
            if first_load {
                if let Some((item_ix, offset)) = page.restore.take() {
                    page.messages.scroll_to(ListOffset {
                        item_ix: item_ix.min(count.saturating_sub(1)),
                        offset_in_item: px(offset),
                    });
                } else {
                    page.messages.scroll_to_end();
                }
            }
        }
        let app = cx.weak_entity();
        let expected = key.clone();
        let list = gpui_kit::list(page.messages.clone(), move |index, window, cx| {
            app.update(cx, |this, cx| {
                if this.subagent_route() != Some(&expected)
                    || this.visible_subagent_transcript() != Some(id)
                {
                    return div().into_any_element();
                }
                this.subagent_message(id, index, window, cx)
            })
            .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full();
        v_flex()
            .id("subagent-message-list")
            .debug_selector(|| "subagent-message-list".into())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(list)
            .into_any_element()
    }

    fn subagent_message(
        &mut self,
        id: u64,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(session) = self.session(id) else {
            return div().into_any_element();
        };
        let provider = session.provider;
        let Some(item) = session.messages.get(index).cloned() else {
            return div().into_any_element();
        };
        if !super::super::chat::should_render_message_item(&item, provider) {
            return div().h(px(0.)).into_any_element();
        }
        if item.kind == ItemKind::Reasoning {
            let text = super::super::chat::message_markdown(&item, provider);
            let state = window.use_keyed_state(
                SharedString::from(format!("chat-markdown-{id}-{}", item.key)),
                cx,
                |_, cx| gpui_kit::base::text::TextViewState::markdown(text.as_ref(), cx),
            );
            state.update(cx, |state, cx| state.set_text(text.as_ref(), cx));
            return v_flex()
                .w_full()
                .min_w_0()
                .px_4()
                .py_2()
                .child(self.render_reasoning_item(id, index, &item, provider, Some(&state), cx))
                .into_any_element();
        }
        if item.kind == ItemKind::Tool {
            let expanded = self.expanded_tools.contains(&(id, item.key.clone()));
            let mut body = v_flex()
                .w_full()
                .min_w_0()
                .px_4()
                .py_2()
                .child(self.activity_row(id, index, &item, cx));
            if expanded {
                let detail_key = (id, item.key.clone());
                let scroll = self
                    .tool_scrolls
                    .entry(detail_key.clone())
                    .or_default()
                    .clone();
                let typography = self.chat_typography();
                let panel = tool_detail_panel(
                    &detail_key,
                    &item,
                    &scroll,
                    &typography,
                    &mut self.tool_diffs,
                    &mut self.tool_analysis,
                    cx,
                );
                body = body.child(div().w_full().min_w_0().pl(px(27.)).child(panel));
            }
            return body.into_any_element();
        }
        let identity = format!("child-message-{id}-{}", item.key);
        let text = super::super::chat::message_markdown(&item, provider).into_owned();
        let app = cx.weak_entity();
        let message = chat_markdown_view(
            identity.clone().into(),
            text,
            &self.chat_typography(),
            &identity,
            true,
        )
        .on_link_click(move |url, event, window, cx| {
            if super::super::chat::activates_link(event) {
                let _ = app.update(cx, |this, cx| this.open_chat_link(id, url, window, cx));
            }
        });
        match item.kind {
            ItemKind::User => v_flex()
                .w_full()
                .min_w_0()
                .items_end()
                .px_4()
                .py_2()
                .child(
                    v_flex()
                        .max_w(px(680.))
                        .min_w_0()
                        .rounded(px(8.))
                        .bg(rgb(palette().surface_hover))
                        .px(px(super::super::chat::CHAT_CARD_INSET_X))
                        .py(px(super::super::chat::CHAT_CARD_INSET_Y))
                        .child(message),
                )
                .into_any_element(),
            ItemKind::Assistant => v_flex()
                .w_full()
                .min_w_0()
                .px_4()
                .py_2()
                .child(message)
                .into_any_element(),
            ItemKind::Notice | ItemKind::Error => v_flex()
                .w_full()
                .min_w_0()
                .px_4()
                .py_3()
                .gap_2()
                .child(
                    div()
                        .font_semibold()
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text(&format!(
                            "subagents.message_{}",
                            if item.kind == ItemKind::Notice {
                                "notice"
                            } else {
                                "error"
                            }
                        ))),
                )
                .child(message)
                .into_any_element(),
            ItemKind::Reasoning | ItemKind::Tool => unreachable!(),
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
