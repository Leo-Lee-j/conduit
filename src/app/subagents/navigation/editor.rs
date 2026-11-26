use super::*;
use crate::app::composer::{
    composer_actions, composer_input_state, composer_shell, composer_textarea,
};

#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct EditorKey {
    pub(super) root: u64,
    agent: String,
}
impl From<&TaskKey> for EditorKey {
    fn from(key: &TaskKey) -> Self {
        Self {
            root: key.root,
            agent: key.agent.clone(),
        }
    }
}

pub(super) struct Editor {
    pub(super) input: Entity<TextareaState>,
    _subscription: Subscription,
    references: Vec<FileReference>,
    root_native: Option<String>,
    source: Option<PathBuf>,
    revision: u64,
    pub(super) error: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(in crate::app) struct SavedEditor {
    pub(super) key: EditorKey,
    text: String,
    selection: std::ops::Range<usize>,
    paths: Vec<PathBuf>,
    #[serde(default)]
    root_native: Option<String>,
    #[serde(default)]
    source: Option<PathBuf>,
}

pub(in crate::app) struct SentChild {
    pub(super) key: EditorKey,
    pub(super) text: String,
    pub(super) references: Vec<FileReference>,
    pub(super) revision: u64,
}

impl ConduitApp {
    pub(in crate::app) fn ensure_child_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(route) = self.subagent_route().cloned() else {
            return;
        };
        let key = EditorKey::from(&route);
        if self
            .session_control
            .agent_navigation
            .editors
            .contains_key(&key)
        {
            return;
        }
        let saved = self
            .session_control
            .agent_navigation
            .recovered
            .iter()
            .find(|entry| entry.key == key)
            .cloned();
        let input = cx.new(|cx| {
            let mut input = composer_input_state(String::new(), window, cx)
                .placeholder(conduit::i18n::text_static("subagents.draft_placeholder"));
            if let Some(saved) = &saved {
                input.set_value(saved.text.clone(), window, cx);
                let start = saved.selection.start.min(saved.text.len());
                let end = saved.selection.end.min(saved.text.len());
                if saved.text.is_char_boundary(start) && saved.text.is_char_boundary(end) {
                    input.set_selected_range(start..end, cx);
                }
            }
            input
        });
        let owner = key.clone();
        let subscription = cx.subscribe(&input, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let Some(editor) = this
                    .session_control
                    .agent_navigation
                    .editors
                    .get_mut(&owner)
            {
                editor.revision = editor.revision.wrapping_add(1);
            }
            // Focus and blur also repaint the shared composer border.
            cx.notify();
        });
        let root_native = saved
            .as_ref()
            .map(|entry| entry.root_native.clone())
            .unwrap_or_else(|| {
                self.session(key.root)
                    .and_then(|session| session.native_id.clone())
            });
        let source = saved
            .as_ref()
            .map(|entry| entry.source.clone())
            .unwrap_or_else(|| self.child_source(key.root));
        let references = saved
            .as_ref()
            .and_then(|saved| {
                self.session_control
                    .locate(&self.store.workspaces, key.root)
                    .map(|(w, _)| {
                        let (mut references, rejected) =
                            file_ref::resolve(&saved.paths, &self.store.workspaces[w].path);
                        // Keep missing attachments visible and recoverable. Send revalidates
                        // every path; a restart must never silently omit an attachment.
                        references.extend(rejected.into_iter().map(|file| FileReference {
                            mention: file_ref::display_path(&file.path),
                            path: file.path,
                        }));
                        references
                    })
            })
            .unwrap_or_default();
        self.session_control.agent_navigation.editors.insert(
            key,
            Editor {
                input,
                _subscription: subscription,
                references,
                root_native,
                source,
                revision: 0,
                error: None,
            },
        );
    }

    pub(in crate::app) fn saved_child_editors(&self, cx: &App) -> Vec<SavedEditor> {
        let mut entries = self.session_control.agent_navigation.recovered.clone();
        for (key, editor) in &self.session_control.agent_navigation.editors {
            entries.retain(|entry| &entry.key != key);
            entries.push(SavedEditor {
                key: key.clone(),
                root_native: editor.root_native.clone(),
                source: editor.source.clone(),
                text: editor.input.read(cx).value().to_string(),
                selection: editor.input.read(cx).selected_range(),
                paths: editor
                    .references
                    .iter()
                    .map(|file| file.path.clone())
                    .collect(),
            });
        }
        entries
    }

    pub(in crate::app) fn child_draft_pending(&self, root: u64, cx: &App) -> bool {
        let navigation = &self.session_control.agent_navigation;
        navigation.editors.iter().any(|(key, editor)| {
            key.root == root
                && (!editor.input.read(cx).value().is_empty() || !editor.references.is_empty())
        }) || navigation.recovered.iter().any(|saved| {
            saved.key.root == root
                && !navigation.editors.contains_key(&saved.key)
                && (!saved.text.is_empty() || !saved.paths.is_empty())
        })
    }

    pub(in crate::app) fn child_attachment_paths(&self) -> Vec<PathBuf> {
        self.session_control
            .agent_navigation
            .editors
            .values()
            .flat_map(|editor| editor.references.iter().map(|file| file.path.clone()))
            .chain(
                self.session_control
                    .agent_navigation
                    .recovered
                    .iter()
                    .flat_map(|entry| entry.paths.iter().cloned()),
            )
            .collect()
    }

    /// Rechecked by every start and by the save gate, regardless of desktop selection.
    pub(in crate::app) fn child_resume_blocked(&self, id: u64) -> Option<String> {
        let session = self.session(id)?;
        if !session.is_subagent {
            return self
                .store
                .child_bindings
                .iter()
                .any(|(child, binding)| {
                    binding.root == id && self.session_control.runs.contains_key(child)
                })
                .then(|| conduit::i18n::text("subagents.child_owns_connection"));
        }
        let blocked = || Some(conduit::i18n::text("subagents.control_unavailable"));
        let Some(binding) = self.store.child_bindings.get(&id) else {
            return blocked();
        };
        let Some(root) = self.session(binding.root) else {
            return blocked();
        };
        if !matches!(session.provider, Provider::Codex | Provider::OpenCode)
            || root.provider != session.provider
            || session.native_id.as_deref() != Some(binding.native.as_str())
            || root.native_id.as_deref() != Some(binding.root_native.as_str())
            || self.child_source(binding.root).as_ref() != Some(&binding.source)
            || session.channel != root.channel
            || !binding.idle
        {
            return blocked();
        }
        if session.provider == Provider::OpenCode
            && !root.subagents.iter().any(|agent| {
                agent.id == binding.native
                    && (agent.status.terminal() || agent.status == Status::Idle)
            })
        {
            return blocked();
        }
        if self.session_control.runs.keys().any(|running| {
            *running == binding.root
                || self
                    .store
                    .child_bindings
                    .get(running)
                    .is_some_and(|other| other.root == binding.root && *running != id)
        }) {
            return Some(conduit::i18n::text("subagents.parent_owns_connection"));
        }
        None
    }

    fn child_send_reason(&self, key: &TaskKey) -> Option<String> {
        if let Some(editor) = self
            .session_control
            .agent_navigation
            .editors
            .get(&EditorKey::from(key))
            && (editor.root_native
                != self
                    .session(key.root)
                    .and_then(|session| session.native_id.clone())
                || editor.source != self.child_source(key.root))
        {
            return Some(conduit::i18n::text("subagents.identity_changed"));
        }
        if self
            .session_control
            .agent_navigation
            .commands
            .contains_key(key)
        {
            return Some(conduit::i18n::text("subagents.submitting"));
        }
        if self
            .store
            .child_commands
            .iter()
            .any(|pending| pending.root == key.root && pending.command.thread == key.agent)
        {
            return Some(conduit::i18n::text("subagents.command_unknown"));
        }
        if self.closing || !self.writable {
            return Some(conduit::i18n::text("subagents.read_only"));
        }
        let Some(id) = self.saved_child(key) else {
            return Some(conduit::i18n::text("subagents.history_unavailable"));
        };
        if self.session_busy(id) {
            return Some(conduit::i18n::text("subagents.child_busy"));
        }
        if self
            .session_control
            .agent_navigation
            .reads
            .contains_key(key)
            || self
                .session_control
                .agent_navigation
                .history_errors
                .contains_key(key)
            || !self
                .session_control
                .agent_navigation
                .refreshed
                .contains_key(key)
        {
            return Some(conduit::i18n::text("subagents.refresh_before_send"));
        }
        if self.live_child_owner(key).is_some() {
            return self
                .routed_agent(key)
                .filter(|agent| {
                    matches!(agent.status, Status::WaitingApproval | Status::WaitingInput)
                })
                .map(|_| conduit::i18n::text("subagents.awaiting_approval"));
        }
        self.child_resume_blocked(id)
    }

    pub(super) fn send_child(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.subagent_route().cloned() else {
            return;
        };
        if self.child_send_reason(&key).is_some() {
            return;
        }
        let owner = EditorKey::from(&key);
        let Some(editor) = self.session_control.agent_navigation.editors.get(&owner) else {
            return;
        };
        if editor.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        let sent = SentChild {
            key: owner.clone(),
            text: editor.input.read(cx).value().to_string(),
            references: editor.references.clone(),
            revision: editor.revision,
        };
        let Some((workspace, _)) = self
            .session_control
            .locate(&self.store.workspaces, key.root)
        else {
            return;
        };
        let paths: Vec<_> = sent
            .references
            .iter()
            .map(|file| file.path.clone())
            .collect();
        let (_, rejected) = file_ref::resolve(&paths, &self.store.workspaces[workspace].path);
        if !rejected.is_empty() {
            self.session_control
                .agent_navigation
                .editors
                .get_mut(&owner)
                .unwrap()
                .error = Some(conduit::i18n::text("subagents.attachments_unavailable"));
            cx.notify();
            return;
        }
        let prompt = file_ref::compose_prompt(&sent.text, &sent.references);
        let target = self.saved_child(&key);
        if self.live_child_owner(&key).is_some() {
            self.start_child_control(key, Some(prompt), Some(sent), cx);
            return;
        }
        let Some(provider) = target.and_then(|id| self.session(id)).map(|s| s.provider) else {
            return;
        };
        match self.start_turn(workspace, target, provider, prompt, true, cx) {
            Ok(id) => {
                if let Some(pending) = self
                    .session_control
                    .runs
                    .get_mut(&id)
                    .and_then(|run| run.pending_save.as_mut())
                {
                    pending.child_composer = Some(sent);
                }
                self.session_control
                    .agent_navigation
                    .editors
                    .get_mut(&owner)
                    .unwrap()
                    .error = None;
            }
            Err(error) => {
                self.session_control
                    .agent_navigation
                    .editors
                    .get_mut(&owner)
                    .unwrap()
                    .error = Some(error)
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn clear_child_draft(
        &mut self,
        sent: &SentChild,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = self
            .session_control
            .agent_navigation
            .editors
            .get_mut(&sent.key)
            && editor.revision == sent.revision
            && editor.input.read(cx).value().as_ref() == sent.text
            && editor.references == sent.references
        {
            editor
                .input
                .update(cx, |input, cx| input.set_value("", window, cx));
            editor.references.clear();
        }
    }

    fn pick_child_files(&mut self, owner: EditorKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || !self.writable {
            return;
        }
        let Some((workspace, _)) = self
            .session_control
            .locate(&self.store.workspaces, owner.root)
        else {
            return;
        };
        let workspace = self.store.workspaces[workspace].path.clone();
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(
                conduit::i18n::text_static("composer.attach_files_to_conversation").into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let (references, rejected) = file_ref::resolve(&paths, &workspace);
                let _ = this.update(cx, |this, cx| {
                    if this.closing || !this.writable {
                        return;
                    }
                    if let Some(editor) = this
                        .session_control
                        .agent_navigation
                        .editors
                        .get_mut(&owner)
                    {
                        file_ref::merge_references(&mut editor.references, references);
                        editor.revision = editor.revision.wrapping_add(1);
                        editor.error = (!rejected.is_empty())
                            .then(|| conduit::i18n::text("subagents.attachments_unavailable"));
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn paste_child_files(
        &mut self,
        owner: &EditorKey,
        item: &ClipboardItem,
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
        let Some((workspace, _)) = self
            .session_control
            .locate(&self.store.workspaces, owner.root)
        else {
            return true;
        };
        let mut errors = Vec::new();
        if paths.is_empty() {
            for image in images {
                match file_ref::save_clipboard_image(
                    image,
                    &Store::path().with_file_name("attachments"),
                ) {
                    Ok(path) => paths.push(path),
                    Err(error) => errors.push(format!("{error:#}")),
                }
            }
        }
        let (references, rejected) =
            file_ref::resolve(&paths, &self.store.workspaces[workspace].path);
        if !rejected.is_empty() {
            errors.push(conduit::i18n::text("subagents.attachments_unavailable"));
        }
        if let Some(editor) = self.session_control.agent_navigation.editors.get_mut(owner) {
            file_ref::merge_references(&mut editor.references, references);
            editor.revision = editor.revision.wrapping_add(1);
            editor.error = (!errors.is_empty()).then(|| errors.join("; "));
        }
        cx.notify();
        true
    }

    pub(super) fn render_child_editor(
        &mut self,
        key: &TaskKey,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = EditorKey::from(key);
        let paste_owner = owner.clone();
        let paste_app = cx.weak_entity();
        let reason = self.child_send_reason(key);
        let Some(editor) = self.session_control.agent_navigation.editors.get(&owner) else {
            return div().into_any_element();
        };
        let disabled = reason.is_some()
            || (editor.input.read(cx).value().trim().is_empty() && editor.references.is_empty());
        let can_edit = self.writable && !self.closing;
        let error = editor.error.clone();
        let input = editor.input.clone();
        let input_focused = input.read(cx).focus_handle(cx).is_focused(window);
        let typography = self.chat_typography();
        let has_files = !editor.references.is_empty();
        let files = editor.references.iter().enumerate().fold(
            h_flex()
                .flex_wrap()
                .gap_2()
                .px(px(crate::app::chat::CHAT_CARD_INSET_X))
                .pt(px(crate::app::chat::CHAT_CARD_INSET_Y)),
            |view, (index, reference)| {
                let owner = owner.clone();
                let path = reference.path.clone();
                view.child(
                    Button::new(SharedString::from(format!("child-file-{index}")))
                        .ghost()
                        .small()
                        .label(
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                        )
                        .tooltip(conduit::i18n::text_static("subagents.remove_attachment"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = this
                                .session_control
                                .agent_navigation
                                .editors
                                .get_mut(&owner)
                            {
                                editor.references.retain(|file| file.path != path);
                                editor.revision = editor.revision.wrapping_add(1);
                                cx.notify();
                            }
                        })),
                )
            },
        );
        let unresolved = self
            .store
            .child_commands
            .iter()
            .any(|pending| pending.root == key.root && pending.command.thread == key.agent)
            && !self
                .session_control
                .agent_navigation
                .commands
                .contains_key(key);
        let acknowledge = key.clone();
        let child = self.saved_child(key);
        let expected = key.clone();
        let stop_key = key.clone();
        let busy = self.live_child_owner(key).is_some()
            || child.is_some_and(|id| self.session_control.runs.contains_key(&id));
        v_flex()
            .id("child-composer")
            .debug_selector(|| "child-composer".into())
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .px(px(crate::app::chat::CHAT_CONTENT_GUTTER))
            .py(px(crate::app::chat::CHAT_BLOCK_GAP))
            .on_action(cx.listener(|this, _: &SendPrompt, window, cx| {
                cx.stop_propagation();
                this.send_child(window, cx);
            }))
            .on_action(cx.listener(move |this, _: &StopReply, _, cx| {
                cx.stop_propagation();
                this.stop_child(&expected, cx);
            }))
            .on_action(cx.listener(|this, _: &Newline, window, cx| {
                cx.stop_propagation();
                if !this.writable || this.closing {
                    return;
                }
                if let Some(key) = this.subagent_route()
                    && let Some(editor) = this
                        .session_control
                        .agent_navigation
                        .editors
                        .get(&EditorKey::from(key))
                {
                    editor
                        .input
                        .clone()
                        .update(cx, |input, cx| input.insert("\n", window, cx));
                }
            }))
            .child(
                composer_shell("child-composer-shell", input_focused)
                    .debug_selector(|| "child-composer-shell".into())
                    .when(!self.store.ui.full_width_content, |shell| {
                        shell
                            .max_w(px(crate::app::chat::CHAT_CONTENT_MAX_WIDTH))
                            .mx_auto()
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(key) = this.subagent_route()
                            && let Some(editor) = this
                                .session_control
                                .agent_navigation
                                .editors
                                .get(&EditorKey::from(key))
                        {
                            editor.input.read(cx).focus_handle(cx).focus(window, cx);
                        }
                    }))
                    .when(has_files, |view| view.child(files))
                    .child(
                        div()
                            .key_context("ConduitComposer")
                            .px(px(crate::app::chat::CHAT_CARD_INSET_X) - Size::Medium.input_px())
                            .pt(px(if has_files { 0. } else { 4. }))
                            .child(
                                composer_textarea(
                                    &input,
                                    &typography,
                                    can_edit,
                                    input.read(cx).value().is_empty(),
                                )
                                .on_paste(move |item, _, cx| {
                                    paste_app
                                        .update(cx, |this, cx| {
                                            this.paste_child_files(&paste_owner, item, cx)
                                        })
                                        .unwrap_or(true)
                                }),
                            ),
                    )
                    .when(unresolved, |view| {
                        view.child(
                            Button::new("child-reviewed")
                                .ghost()
                                .small()
                                .label(conduit::i18n::text_static("subagents.acknowledge"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.store.child_commands.retain(|pending| {
                                        pending.root != acknowledge.root
                                            || pending.command.thread != acknowledge.agent
                                    });
                                    this.persist();
                                    this.request_child_history(true, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .when_some(error, |view, error| {
                        view.child(div().px_2().text_color(rgb(palette().muted)).child(error))
                    })
                    .child(
                        composer_actions()
                            .id("child-composer-actions")
                            .debug_selector(|| "child-composer-actions".into())
                            .child(div().flex_1())
                            .child(
                                message_action_button(
                                    "child-attach".into(),
                                    Icon::new(FullIconName::Paperclip),
                                    conduit::i18n::text_static("subagents.attach"),
                                )
                                .disabled(!can_edit)
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.pick_child_files(owner.clone(), window, cx)
                                    },
                                )),
                            )
                            .when(busy, |view| {
                                view.child(
                                    Button::new("child-stop")
                                        .small()
                                        .compact()
                                        .w(px(30.))
                                        .h(px(30.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .primary()
                                        .disabled(!can_edit)
                                        .icon(Icon::new(FullIconName::Square).size(px(12.)))
                                        .tooltip(conduit::i18n::text_static("subagents.stop"))
                                        .accessibility_label(conduit::i18n::text_static(
                                            "subagents.stop",
                                        ))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.stop_child(&stop_key, cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("child-send")
                                    .small()
                                    .compact()
                                    .w(px(30.))
                                    .h(px(30.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .primary()
                                    .icon(IconName::ArrowUp)
                                    .tooltip(conduit::i18n::text_static(
                                        if self.live_child_owner(key).is_some() {
                                            "subagents.steer"
                                        } else {
                                            "subagents.send"
                                        },
                                    ))
                                    .accessibility_label(conduit::i18n::text_static(
                                        if self.live_child_owner(key).is_some() {
                                            "subagents.steer"
                                        } else {
                                            "subagents.send"
                                        },
                                    ))
                                    .disabled(disabled || !can_edit)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_child(window, cx)
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}
