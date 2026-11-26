//! Composer attachments and file selection.

use super::*;

mod binding;
pub(in crate::app) use binding::{ComposerBinding, EditorState};

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

pub(in crate::app) fn composer_input_state(
    draft: String,
    window: &mut Window,
    cx: &mut Context<TextareaState>,
) -> TextareaState {
    TextareaState::new(window, cx)
        .placeholder(conduit::i18n::text_static(
            "localization.describe_a_task_type_to_reference_files",
        ))
        .auto_grow(3, 8)
        .default_value(draft)
}

pub(in crate::app) fn composer_shell(id: impl Into<ElementId>, focused: bool) -> Stateful<Div> {
    v_flex()
        .id(id)
        .w_full()
        .min_w_0()
        .gap_1()
        .rounded(px(11.))
        .bg(rgb(palette().surface))
        .border_1()
        .border_color(rgb(if focused {
            palette().focus
        } else {
            palette().border
        }))
}

pub(in crate::app) fn composer_textarea(
    input: &Entity<TextareaState>,
    typography: &super::typography::ChatTypography,
    enabled: bool,
    empty: bool,
) -> Textarea {
    Textarea::new(input)
        .with_size(Size::Medium)
        .appearance(false)
        .bordered(false)
        .text_size(px(typography.body_size))
        .line_height(relative(typography.line_height))
        .when(empty, |input| input.opacity(0.65))
        .disabled(!enabled)
        .when_some(typography.body_font.clone(), |input, family| {
            input.font_family(family)
        })
}

pub(in crate::app) fn composer_actions() -> Div {
    h_flex()
        .px(px(chat::CHAT_CARD_INSET_Y))
        .pb(px(chat::CHAT_CARD_INSET_Y))
        .gap_2()
        .items_center()
}

/// Show actionable file names, bounding the message for large multi-selections.
pub(super) fn rejection_summary(rejected: &[RejectedFile]) -> String {
    let mut details = rejected
        .iter()
        .take(3)
        .map(|file| {
            let name = file
                .path
                .file_name()
                .unwrap_or(file.path.as_os_str())
                .to_string_lossy();
            format!("{name}：{}", file.reason.label())
        })
        .collect::<Vec<_>>();
    if rejected.len() > 3 {
        details.push(conduit::i18n::format(
            "composer.more",
            &[("arg0", format!("{}", rejected.len() - 3))],
        ));
    }
    conduit::i18n::format(
        "composer.items",
        &[
            ("arg0", format!("{}", rejected.len())),
            ("arg1", details.join("；").to_string()),
        ],
    )
}

/// A file selection belongs to a conversation, including a workspace's new-chat draft.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ComposerTarget {
    pub(super) workspace: PathBuf,
    pub(super) session: Option<u64>,
}

#[derive(Default)]
pub(super) struct ComposerFiles {
    pub(super) references: Vec<FileReference>,
    pub(super) feedback: Option<String>,
}

impl ConduitApp {
    /// A deduplicated image can be shared by drafts and sent messages across
    /// workspaces. Rebuild reachability only when attachments are discarded.
    pub(super) fn cleanup_attachments(&mut self, cx: &mut Context<Self>) {
        if !self.writable
            || self.persistence.busy()
            || self.persistence.error.is_some()
            || self.dirty
            || self
                .session_control
                .runs
                .values()
                .any(|run| run.pending_save.is_some())
            || !self.session_control.queue_saves.is_empty()
            || Store::path()
                .with_file_name("update-recovery.json")
                .exists()
            || Store::path().with_file_name("draft-recovery.json").exists()
        {
            return;
        }
        let mut referenced = self
            .composer_files
            .values()
            .flat_map(|files| files.references.iter().map(|file| file.path.clone()))
            .collect::<Vec<_>>();
        referenced.extend(self.child_attachment_paths());
        referenced.extend(
            self.queued_messages
                .values()
                .flat_map(|message| message.references.iter().map(|file| file.path.clone())),
        );
        for workspace in &self.store.workspaces {
            for session in &workspace.sessions {
                let paths = if session.transcript.loaded {
                    crate::transcript::attachment_paths(&session.messages)
                } else {
                    session
                        .transcript
                        .reference
                        .as_ref()
                        .map(|body| body.attachments.clone())
                        .unwrap_or_default()
                };
                referenced.extend(paths.into_iter().map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        workspace.path.join(path)
                    }
                }));
            }
        }
        if let Err(error) = file_ref::prune_clipboard_images(
            &Store::path().with_file_name("attachments"),
            &referenced,
        ) {
            self.notify_feedback(
                NoticeSource::App,
                NoticeLevel::Warning,
                conduit::i18n::format(
                    "composer.failed_to_clean_up_image_attachments",
                    &[("error", format!("{:#}", error))],
                ),
                None,
                cx,
            );
        }
    }

    /// Never delete files while the saved transcript still contains old refs.
    pub(super) fn persist_and_cleanup_attachments(&mut self) {
        self.attachments_cleanup_pending = true;
        self.persist();
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn discard_composer_attachments(&mut self, cx: &mut Context<Self>) {
        self.composer_files.clear();
        // Draft attachments are in memory only. The close path has already
        // waited for every transcript/queue save; do not enqueue another save
        // immediately before exiting.
        self.cleanup_attachments(cx);
    }

    /// Let the input perform its normal text paste unless files or pixels exist.
    pub(super) fn paste_attachments(
        &mut self,
        item: &ClipboardItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut paths = Vec::new();
        let mut images = Vec::new();
        for entry in &item.entries {
            match entry {
                ClipboardEntry::ExternalPaths(files) => paths.extend_from_slice(files.paths()),
                ClipboardEntry::Image(image) => images.push(image),
                ClipboardEntry::String(_) => {}
            }
        }
        if paths.is_empty() && images.is_empty() {
            return false;
        }
        if self.closing || !self.writable {
            return true;
        }
        let Some(target) = self.composer_target() else {
            self.notify_composer_warning(
                conduit::i18n::text_static("composer.select_a_workspace_in_the_upper_left"),
                cx,
            );
            return true;
        };
        let mut errors = Vec::new();
        // File managers may expose a thumbnail alongside file paths. Prefer the
        // original files instead of attaching a second copy of their pixels.
        if paths.is_empty() {
            let directory = Store::path().with_file_name("attachments");
            for image in images {
                match file_ref::save_clipboard_image(image, &directory) {
                    Ok(path) => paths.push(path),
                    Err(error) => errors.push(conduit::i18n::format(
                        "composer.failed_to_paste_image",
                        &[("error", format!("{:#}", error))],
                    )),
                }
            }
        }
        if !self.attach_paths(&paths, window, cx) {
            self.persist_and_cleanup_attachments();
        }
        if !errors.is_empty() {
            let draft = self.composer_files.entry(target).or_default();
            if let Some(feedback) = draft.feedback.take() {
                errors.insert(0, feedback);
            }
            draft.feedback = Some(errors.join("；"));
            cx.notify();
        }
        true
    }

    pub(super) fn composer_target(&self) -> Option<ComposerTarget> {
        self.composer_binding().target
    }

    pub(super) fn current_files(&self) -> Option<&ComposerFiles> {
        self.composer_target()
            .and_then(|target| self.composer_files.get(&target))
    }

    /// All entry points share validation, deduplication and inline feedback.
    /// Returns whether any path is valid, including an already attached file.
    pub(super) fn attach_paths(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.closing || !self.writable || paths.is_empty() {
            return false;
        }
        let Some(target) = self.composer_target() else {
            self.notify_composer_warning(
                conduit::i18n::text_static("composer.select_a_workspace_in_the_upper_left"),
                cx,
            );
            return false;
        };
        let (references, rejected) = file_ref::resolve(paths, &target.workspace);
        let accepted = !references.is_empty();
        let draft = self.composer_files.entry(target).or_default();
        let added = file_ref::merge_references(&mut draft.references, references);
        draft.feedback = if !rejected.is_empty() {
            Some(conduit::i18n::format(
                "composer.added_files_skipped",
                &[
                    ("added", format!("{}", added)),
                    ("arg0", rejection_summary(&rejected).to_string()),
                ],
            ))
        } else if added == 0 {
            Some(conduit::i18n::text_static("composer.these_files_are_already_attached").into())
        } else {
            None
        };
        self.composer_input()
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
        accepted
    }

    pub(super) fn remove_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || !self.writable {
            return;
        }
        if let Some(target) = self.composer_target()
            && let Some(draft) = self.composer_files.get_mut(&target)
        {
            draft.references.retain(|reference| reference.path != path);
            draft.feedback = None;
        }
        self.persist_and_cleanup_attachments();
        self.composer_input()
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    /// The prompts already sent in the selected conversation, newest first.
    /// Recall browses the prose the user typed: the stored item keeps the
    /// trailing file-reference block added at send time, and an
    /// attachment-only message has no prose to browse.
    pub(super) fn sent_prompts(&self) -> Vec<String> {
        const LIMIT: usize = 100;
        let Some(session) = self.composer_session().and_then(|id| self.session(id)) else {
            return Vec::new();
        };
        session
            .messages
            .iter()
            .rev()
            .filter(|item| item.kind == ItemKind::User)
            .filter_map(|item| {
                let text = file_ref::split_prompt(&item.text).0;
                (!text.trim().is_empty()).then(|| text.to_string())
            })
            .take(LIMIT)
            .collect()
    }

    fn set_composer_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.composer_input().update(cx, |input, cx| {
            input.set_value(text.clone(), window, cx);
            // set_value leaves a multi-line caret at the start; recalled
            // prompts read and continue best with the caret at the end.
            input.set_selected_range(text.len()..text.len(), cx);
        });
        cx.notify();
    }

    /// Up arrow: browse backward through the sent prompts. Only an empty
    /// composer starts browsing, so a typed draft is never replaced; while
    /// a file-mention menu is open the arrows keep moving its selection.
    pub(super) fn previous_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || !self.writable {
            return;
        }
        if self.file_mentions_open(window, cx) {
            self.move_file_mention(-1, window, cx);
            return;
        }
        if self.composer_is_composing(window, cx) {
            return;
        }
        let history = self.sent_prompts();
        let next = match self.prompt_history {
            Some(index) => index + 1,
            None if history.is_empty() || !self.composer_input().read(cx).value().is_empty() => {
                // Nothing to recall yet, or the composer holds a draft: the
                // arrow keeps moving the caret.
                window.dispatch_action(Box::new(MoveUp), cx);
                return;
            }
            None => 0,
        };
        // Browsing stops at the oldest prompt; `get` also absorbs a history
        // refreshed by a background CLI import without panicking.
        if let Some(text) = history.get(next).cloned() {
            self.prompt_history = Some(next);
            self.set_composer_text(text, window, cx);
        }
    }

    /// Down arrow: move toward newer prompts, and past the newest back to
    /// the empty composer that browsing started from.
    pub(super) fn next_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || !self.writable {
            return;
        }
        if self.file_mentions_open(window, cx) {
            self.move_file_mention(1, window, cx);
            return;
        }
        if self.composer_is_composing(window, cx) {
            return;
        }
        match self.prompt_history {
            None => {
                window.dispatch_action(Box::new(MoveDown), cx);
            }
            Some(0) => {
                self.prompt_history = None;
                self.set_composer_text(String::new(), window, cx);
            }
            Some(index) => match self.sent_prompts().get(index - 1).cloned() {
                Some(text) => {
                    self.prompt_history = Some(index - 1);
                    self.set_composer_text(text, window, cx);
                }
                None => {
                    // The history shrank underneath (background import);
                    // end browsing rather than pointing at a stale rank.
                    self.prompt_history = None;
                    self.set_composer_text(String::new(), window, cx);
                }
            },
        }
    }

    pub(super) fn render_composer_files(
        &self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let files = self.current_files()?;
        if files.references.is_empty() && files.feedback.is_none() {
            return None;
        }
        let mut tray = v_flex()
            .id("composer-files")
            .w_full()
            .min_w_0()
            .px(px(chat::CHAT_CARD_INSET_X))
            .pt(px(chat::CHAT_BLOCK_GAP))
            .gap_2();
        if !files.references.is_empty() {
            let cards = files
                .references
                .iter()
                .enumerate()
                .map(|(index, reference)| {
                    let path = reference.path.clone();
                    let full_path = display_path(&path);
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    h_flex()
                        .id(("composer-file", index))
                        .debug_selector(|| format!("composer-file-{index}"))
                        .max_w(px(220.))
                        .h(px(26.))
                        .flex_shrink_0()
                        .min_w_0()
                        .gap_1()
                        .px_2()
                        .items_center()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(rgb(palette().border))
                        .bg(rgb(palette().surface_strong))
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(full_path.clone())
                                .build(window, cx)
                        })
                        .child(
                            Icon::new(file_ref::icon_for(&path))
                                .small()
                                .text_color(rgb(palette().muted)),
                        )
                        .child(
                            div()
                                .flex_shrink_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(rgb(palette().text))
                                .child(name),
                        )
                        .child(
                            Button::new(("remove-composer-file", index))
                                .ghost()
                                .small()
                                .compact()
                                .w(px(24.))
                                .h(px(24.))
                                .flex_shrink_0()
                                .icon(FullIconName::X)
                                .tooltip(conduit::i18n::text_static("composer.remove_file"))
                                .accessibility_label(conduit::i18n::text_static(
                                    "composer.remove_file",
                                ))
                                .disabled(!enabled)
                                .when(enabled, |button| button.cursor_pointer())
                                .on_click(self.composer_listener(
                                    cx,
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.remove_file(&path, window, cx);
                                    },
                                )),
                        )
                });
            tray = tray.child(
                h_flex()
                    .id("composer-file-cards")
                    .flex_wrap()
                    .gap_1()
                    .max_h(px(144.))
                    .overflow_y_scroll()
                    .children(cards),
            );
        }
        if let Some(feedback) = &files.feedback {
            tray = tray.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(rgb(palette().muted))
                            .child(feedback.clone()),
                    )
                    .child(
                        Button::new("dismiss-file-feedback")
                            .ghost()
                            .small()
                            .compact()
                            .icon(FullIconName::X)
                            .tooltip(conduit::i18n::text_static("composer.dismiss"))
                            .accessibility_label(conduit::i18n::text_static(
                                "composer.dismiss_file_hint",
                            ))
                            .on_click(self.composer_listener(cx, |this, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(target) = this.composer_target()
                                    && let Some(files) = this.composer_files.get_mut(&target)
                                {
                                    files.feedback = None;
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        Some(tray.into_any_element())
    }

    pub(super) fn pick_files(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing || !self.writable {
            return;
        }
        let Some(target) = self.composer_target() else {
            return;
        };
        let binding = self.composer_binding();
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(
                conduit::i18n::text_static("composer.attach_files_to_conversation").into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                let _ = this.update_in(cx, |this, window, cx| {
                    // Native dialogs are asynchronous: a later selection must never
                    // land in a different workspace or conversation.
                    this.with_composer(binding.clone(), cx, |this, cx| {
                        this.attach_paths(&paths, window, cx);
                    });
                });
            }
            Ok(Ok(None)) => {}
            other => {
                let _ = this.update(cx, |this, cx| {
                    if this.composer_binding_live(&binding) {
                        this.composer_files.entry(target).or_default().feedback =
                            Some(conduit::i18n::format(
                                "composer.failed_to_open_file_picker",
                                &[("other", format!("{:?}", other))],
                            ));
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }
}

impl ConduitApp {
    pub(super) fn stop_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_read_only() || self.closing || self.composer_is_composing(window, cx) {
            return;
        }
        if let Some(id) = self.composer_session()
            && self.stop_session_run(id)
        {
            self.composer_input()
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            cx.notify();
        }
    }

    pub(super) fn stop_session_run(&mut self, id: u64) -> bool {
        if !self.session_control.runs.contains_key(&id) {
            return false;
        }
        self.pause_queued_message(
            id,
            conduit::i18n::text_static("composer.the_current_response_has_stopped_queued_messages"),
        );
        if let Some(run) = self.session_control.runs.get_mut(&id) {
            run.cancel();
        }
        true
    }

    pub(super) fn render_composer(
        &mut self,
        _selected: Option<&SessionSummary>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_composer_for(self.composer_binding(), window, cx)
    }

    fn render_bound_composer(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let session_id = self.composer_session().unwrap_or_default();
        let has_session = self.composer_session().is_some();
        let input = self.composer_input();
        let main = self.composer_is_main();
        let app_entity = cx.entity();
        let typography = self.chat_typography();
        let host_command_ready = self.host_chat_command_ready(cx);
        let blocked = if host_command_ready {
            None
        } else {
            self.composer_send_blocked()
        };
        let busy = blocked.is_some();
        let has_workspace = self.composer_workspace().is_some();
        let can_edit = self.writable && !self.closing && !self.composer_read_only();
        let input_focused = self
            .composer_input()
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        self.refresh_chat_commands(cx);
        self.refresh_file_mentions(cx);
        let commands_open = self.chat_commands_open(window, cx);
        let command_menu_open = self.chat_command_menu_open(window, cx);
        let mentions_open = !command_menu_open && self.file_mentions_open(window, cx);
        let has_content = !self.composer_input().read(cx).value().trim().is_empty()
            || self
                .current_files()
                .is_some_and(|files| !files.references.is_empty());
        let send_ready = can_edit && !busy && has_workspace && has_content;
        let selected_run = self
            .composer_session()
            .and_then(|id| self.session_control.runs.get(&id));
        let show_stop = selected_run.is_some() && !host_command_ready;
        let stopping = selected_run.is_some_and(|run| run.runtime.is_cancelled());
        let model_picker = self.composer_model_picker(cx.entity(), session_id, !can_edit);
        // One composer for every state: an active session, a workspace without
        // a session (the first send creates one with the last engine), and no
        // workspace at all. Keeping the same shell in all three states keeps
        // the chat area from changing size when the session state changes.
        let composer_enabled = can_edit && has_workspace;
        let subagents = self
            .composer_session()
            .and_then(|id| self.subagent_panel(id, cx));
        let composer_files = self.render_composer_files(composer_enabled, cx);
        let has_files = composer_files.is_some();
        let mut composer = composer_shell(("composer", input.entity_id()), input_focused)
            .debug_selector(move || {
                if main {
                    "composer"
                } else {
                    "reference-composer"
                }
                .into()
            })
            .on_action(
                self.composer_listener(cx, |this, _: &SendPrompt, window, cx| {
                    cx.stop_propagation();
                    if this.composer_is_composing(window, cx) {
                        return;
                    }
                    if this.chat_commands_open(window, cx) && this.accept_chat_command(window, cx) {
                        return;
                    }
                    if this.file_mentions_open(window, cx) {
                        this.accept_file_mention(window, cx);
                    } else {
                        this.send_prompt(window, cx);
                    }
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &StopReply, window, cx| {
                    cx.stop_propagation();
                    this.stop_reply(window, cx);
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &PreviousFileMention, window, cx| {
                    cx.stop_propagation();
                    this.move_file_mention(-1, window, cx);
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &PreviousChatCommand, window, cx| {
                    cx.stop_propagation();
                    this.move_chat_command(-1, window, cx);
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &NextChatCommand, window, cx| {
                    cx.stop_propagation();
                    this.move_chat_command(1, window, cx);
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &AcceptChatCommand, window, cx| {
                    cx.stop_propagation();
                    this.accept_chat_command(window, cx);
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &DismissChatCommand, window, cx| {
                    cx.stop_propagation();
                    if !this.composer_is_composing(window, cx) {
                        this.dismiss_chat_commands(cx);
                    }
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &NextFileMention, window, cx| {
                    cx.stop_propagation();
                    this.move_file_mention(1, window, cx);
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &AcceptFileMention, window, cx| {
                    cx.stop_propagation();
                    this.accept_file_mention(window, cx);
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &DismissFileMention, window, cx| {
                    cx.stop_propagation();
                    if !this.composer_is_composing(window, cx) {
                        this.dismiss_file_mentions(cx);
                    }
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &PreviousPrompt, window, cx| {
                    cx.stop_propagation();
                    this.previous_prompt(window, cx);
                }),
            )
            .on_action(
                self.composer_listener(cx, |this, _: &NextPrompt, window, cx| {
                    cx.stop_propagation();
                    this.next_prompt(window, cx);
                }),
            )
            .on_action(self.composer_listener(cx, |this, _: &Newline, window, cx| {
                cx.stop_propagation();
                this.composer_input()
                    .update(cx, |input, cx| input.insert("\n", window, cx));
                cx.notify();
            }))
            .on_click(self.composer_listener(cx, |this, _, window, cx| {
                if this.composer_workspace().is_some() {
                    this.composer_input()
                        .read(cx)
                        .focus_handle(cx)
                        .focus(window, cx);
                }
            }))
            // 资源管理器拖入的文件由 gpui 翻译成 ExternalPaths 载荷的拖拽事件，
            // 与右栏调宽、文本选择这些内部拖拽按载荷类型隔离，不会互相抢事件。
            .on_drop(
                self.composer_listener(cx, |this, dropped: &ExternalPaths, window, cx| {
                    this.attach_paths(dropped.paths(), window, cx);
                }),
            )
            .on_drop(
                self.composer_listener(cx, |this, dropped: &files::FileDrag, window, cx| {
                    this.attach_paths(std::slice::from_ref(&dropped.path), window, cx);
                }),
            )
            .drag_over::<files::FileDrag>(|style, _, _, _| {
                style
                    .bg(rgb(palette().surface_hover))
                    .border_color(rgb(palette().focus))
            })
            .drag_over::<ExternalPaths>(|style, _, _, _| {
                style
                    .bg(rgb(palette().surface_hover))
                    .border_color(rgb(palette().focus))
            })
            .when_some(subagents, |composer, panel| composer.child(panel))
            .when_some(composer_files, |composer, files| composer.child(files))
            .child(
                div()
                    // Input shortcuts must not shadow Enter/Space activation
                    // on sibling subagent and composer action buttons.
                    .key_context(if commands_open {
                        "ConduitComposer ChatCommand"
                    } else if command_menu_open {
                        "ConduitComposer EmptyChatCommand"
                    } else if mentions_open {
                        "ConduitComposer FileMention"
                    } else {
                        "ConduitComposer"
                    })
                    // Textarea adds its own editor padding; account for it once
                    // so typed text and attachment chips share the same inset.
                    .px(px(chat::CHAT_CARD_INSET_X) - Size::Medium.input_px())
                    .pt(px(if has_files { 0. } else { 4. }))
                    .child({
                        let composer_input = composer_textarea(
                            &input,
                            &typography,
                            !self.closing && composer_enabled,
                            input.read(cx).value().is_empty(),
                        );
                        let paste_app = app_entity.clone();
                        let paste_binding = self.composer_binding();
                        composer_input.on_paste(move |item, window, cx| {
                            paste_app.update(cx, |this, cx| {
                                this.with_composer(paste_binding.clone(), cx, |this, cx| {
                                    this.paste_attachments(item, window, cx)
                                })
                                .unwrap_or(false)
                            })
                        })
                    }),
            )
            .child(
                composer_actions()
                    .id("composer-actions")
                    .debug_selector(move || {
                        if main {
                            "composer-actions"
                        } else {
                            "reference-composer-actions"
                        }
                        .into()
                    })
                    .child(
                        Button::new("chat-new-session")
                            .ghost()
                            .small()
                            .compact()
                            .h(px(28.))
                            .icon(IconName::Plus)
                            .tooltip(conduit::i18n::format(
                                "composer.new_session_in_this_workspace",
                                &[("arg0", self.composer_provider().label().to_string())],
                            ))
                            .accessibility_label(conduit::i18n::text_static(
                                "composer.new_session_in_this_workspace_ba056f",
                            ))
                            .disabled(!composer_enabled)
                            .when(composer_enabled, |button| button.cursor_pointer())
                            .on_click(self.composer_listener(cx, |this, _, window, cx| {
                                cx.stop_propagation();
                                this.new_composer_session(window, cx);
                            })),
                    )
                    .when(has_session, |row| {
                        row.child(self.permission_picker(app_entity.clone()))
                    })
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .justify_end()
                            .child(model_picker),
                    )
                    .when(composer_enabled, |row| {
                        row.child(
                            message_action_button(
                                "attach-file".into(),
                                Icon::new(FullIconName::Paperclip),
                                conduit::i18n::text_static(
                                    "composer.attach_files_or_type_drag_files_or",
                                ),
                            )
                            .on_click(self.composer_listener(cx, Self::pick_files)),
                        )
                    })
                    .child(if show_stop {
                        Button::new("stop")
                            .debug_selector(move || {
                                if main { "stop" } else { "reference-stop" }.into()
                            })
                            .icon(Icon::new(FullIconName::Square).size(px(12.)))
                            .accessibility_label(if stopping {
                                conduit::i18n::text_static("composer.stopping_the_current_response")
                            } else {
                                conduit::i18n::text_static("composer.stop_the_current_response")
                            })
                            .tooltip(if stopping {
                                conduit::i18n::text_static(
                                    "composer.stopping_the_current_response_303aa5",
                                )
                                .to_string()
                            } else {
                                conduit::i18n::format(
                                    "composer.stop_the_current_response_193a82",
                                    &[(
                                        "arg0",
                                        shortcuts::display("stop_reply", &self.store.ui.shortcuts)
                                            .to_string(),
                                    )],
                                )
                            })
                            .small()
                            .compact()
                            .w(px(30.))
                            .h(px(30.))
                            .flex_shrink_0()
                            .rounded_full()
                            .primary()
                            .loading(stopping)
                            .disabled(self.closing || stopping)
                            .on_click(self.composer_listener(cx, |this, _, window, cx| {
                                cx.stop_propagation();
                                this.stop_reply(window, cx);
                            }))
                    } else {
                        Button::new("send")
                            .debug_selector(move || {
                                if main { "send" } else { "reference-send" }.into()
                            })
                            .accessibility_label(conduit::i18n::text_static(
                                "composer.send_message",
                            ))
                            .tooltip(blocked.unwrap_or(conduit::i18n::text_static(
                                "composer.send_message_enter",
                            )))
                            .icon(IconName::ArrowUp)
                            .small()
                            .compact()
                            .w(px(30.))
                            .h(px(30.))
                            .flex_shrink_0()
                            .rounded_full()
                            .primary()
                            .disabled(!send_ready)
                            .on_click(self.composer_listener(cx, |this, event, window, cx| {
                                cx.stop_propagation();
                                this.send_message(event, window, cx);
                            }))
                    }),
            );
        if !self.store.ui.full_width_content {
            composer = composer.max_w(px(chat::CHAT_CONTENT_MAX_WIDTH)).mx_auto();
        }
        let composer: AnyElement = composer.into_any_element();
        // Engine activity and step/file tags reserve their own status row.
        // The latest-message control floats over the transcript, so showing it
        // never changes the viewport during a wheel gesture.
        let turn_activity = self
            .composer_session()
            .and_then(|id| self.turn_activity_strip(id));
        let turn_stats = self
            .composer_session()
            .and_then(|id| self.turn_stats_tags(id, cx));
        let has_status_row = turn_activity.is_some() || turn_stats.is_some();
        // Status, queued prompts, and the composer share the transcript column.
        let status_row = div().w_full().px(px(chat::CHAT_CONTENT_GUTTER)).child(
            h_flex()
                .id("turn-status-row")
                .debug_selector(|| "turn-status-row".into())
                .w_full()
                .min_w_0()
                .when(!self.store.ui.full_width_content, |row| {
                    row.max_w(px(chat::CHAT_CONTENT_MAX_WIDTH)).mx_auto()
                })
                .items_center()
                .justify_end()
                .gap(px(6.))
                .pb(px(8.))
                .when_some(turn_activity, |row, activity| row.child(activity))
                .when_some(turn_stats, |row, tags| row.child(tags)),
        );
        let area = div()
            .pt(px(chat::CHAT_BLOCK_GAP))
            .pb(px(chat::CHAT_BLOCK_GAP))
            .child(
                v_flex()
                    .w_full()
                    .items_center()
                    .when(has_status_row, |column| column.child(status_row))
                    .when_some(
                        self.render_queued_message(composer_enabled, cx),
                        |column, bubble| {
                            column.child(
                                div()
                                    .w_full()
                                    .px(px(chat::CHAT_CONTENT_GUTTER))
                                    .pb_2()
                                    .when(!self.store.ui.full_width_content, |row| {
                                        row.max_w(px(chat::CHAT_CONTENT_MAX_WIDTH
                                            + 2. * chat::CHAT_CONTENT_GUTTER))
                                    })
                                    .child(bubble),
                            )
                        },
                    )
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .px(px(chat::CHAT_CONTENT_GUTTER))
                            .when(!self.store.ui.full_width_content, |card| {
                                card.max_w(px(
                                    chat::CHAT_CONTENT_MAX_WIDTH + 2. * chat::CHAT_CONTENT_GUTTER
                                ))
                                .mx_auto()
                            })
                            .child(composer)
                            .when(mentions_open, |area| {
                                area.child(self.render_file_mentions(window, cx))
                            })
                            .when(command_menu_open, |area| {
                                area.child(self.render_chat_commands(window, cx))
                            }),
                    ),
            );
        area.into_any_element()
    }
}
