//! An editor owns its callbacks. Navigation never impersonates another pane.
use super::*;
use crate::app::files::Pane;

#[derive(Clone)]
pub(in crate::app) struct ComposerBinding {
    pub(in crate::app) pane: Pane,
    pub(in crate::app) target: Option<ComposerTarget>,
    pub(in crate::app) input: Entity<TextareaState>,
}

pub(in crate::app) struct EditorState {
    binding: ComposerBinding,
    registered: bool,
    scope_count: usize,
    mentions: mentions::FileMentions,
    commands: commands::CommandEditor,
    history: Option<usize>,
    provider: Provider,
    selection: Option<ComposerSelection>,
    permission: PermissionLevel,
    _subscription: Subscription,
    _change_subscription: Subscription,
}

impl ConduitApp {
    pub(in crate::app) fn composer_binding_for_session(
        &self,
        id: u64,
        input: Entity<TextareaState>,
    ) -> Option<ComposerBinding> {
        let (index, _) = self.session_control.locate(&self.store.workspaces, id)?;
        Some(ComposerBinding {
            pane: Pane::Reference,
            target: Some(ComposerTarget {
                workspace: self.store.workspaces[index].path.clone(),
                session: Some(id),
            }),
            input,
        })
    }

    pub(in crate::app) fn composer_binding(&self) -> ComposerBinding {
        self.composer_scope
            .clone()
            .unwrap_or_else(|| ComposerBinding {
                pane: Pane::Main,
                target: self
                    .selected_workspace
                    .and_then(|index| self.store.workspaces.get(index))
                    .map(|workspace| ComposerTarget {
                        workspace: workspace.path.clone(),
                        session: self.selected_session,
                    }),
                input: self.input.clone(),
            })
    }

    pub(in crate::app) fn composer_binding_live(&self, binding: &ComposerBinding) -> bool {
        if binding.pane == Pane::Main {
            return binding.input == self.input
                && binding.target
                    == self
                        .selected_workspace
                        .and_then(|index| self.store.workspaces.get(index))
                        .map(|workspace| ComposerTarget {
                            workspace: workspace.path.clone(),
                            session: self.selected_session,
                        });
        }
        binding.target.as_ref().is_some_and(|target| {
            // Only the view lifecycle may establish an input's identity. Render
            // and delayed callbacks cannot reclaim a closed or moved editor.
            if !self
                .composer_editors
                .get(&binding.input.entity_id())
                .is_some_and(|editor| {
                    editor.registered && editor.binding.target.as_ref() == Some(target)
                })
            {
                return false;
            }
            target.session.is_some_and(|id| {
                self.session_control
                    .locate(&self.store.workspaces, id)
                    .is_some_and(|(index, _)| {
                        self.store.workspaces[index].path == target.workspace
                            && self.document_pane(DocumentTab::Chat(id)) == Pane::Reference
                    })
            })
        })
    }

    /// Called when a reference view is created, before any render or callback.
    /// The input is supplied by its owner so we never read a borrowed view.
    pub(in crate::app) fn register_reference_composer(
        &mut self,
        id: u64,
        input: Entity<TextareaState>,
        cx: &mut Context<Self>,
    ) {
        let Some(binding) = self.composer_binding_for_session(id, input) else {
            return;
        };
        if self
            .composer_editors
            .get(&binding.input.entity_id())
            .is_some_and(|editor| editor.registered && editor.binding.target == binding.target)
        {
            return;
        }
        self.release_composer_editor(id);
        let provider = binding
            .target
            .as_ref()
            .and_then(|t| t.session)
            .and_then(|id| self.session(id))
            .map_or(self.new_session_provider, |s| s.provider);
        let owner = binding.clone();
        let subscription = cx.observe(&binding.input, move |this, _, cx| {
            this.with_composer(owner.clone(), cx, |this, cx| {
                this.refresh_chat_commands(cx);
                this.refresh_file_mentions(cx);
            });
        });
        let change_subscription = cx.subscribe(&binding.input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        self.composer_editors.insert(
            binding.input.entity_id(),
            EditorState {
                binding: binding.clone(),
                registered: true,
                scope_count: 0,
                mentions: Default::default(),
                commands: Default::default(),
                history: None,
                provider,
                selection: None,
                permission: self.store.ui.default_session_permission,
                _subscription: subscription,
                _change_subscription: change_subscription,
            },
        );
    }

    /// Invalidate callbacks immediately; scoped state survives only long enough
    /// to restore the caller's editor when a close/move occurs inside a callback.
    pub(in crate::app) fn release_composer_editor(&mut self, id: u64) {
        self.composer_editors.retain(|_, editor| {
            if editor.binding.target.as_ref().and_then(|t| t.session) == Some(id) {
                editor.registered = false;
            }
            editor.registered || editor.scope_count > 0
        });
    }

    fn swap_composer_editor(&mut self, binding: &ComposerBinding) {
        if binding.pane == Pane::Reference {
            let editor = self
                .composer_editors
                .get_mut(&binding.input.entity_id())
                .expect("registered composer");
            std::mem::swap(&mut self.file_mentions, &mut editor.mentions);
            std::mem::swap(&mut self.chat_commands.editor, &mut editor.commands);
            std::mem::swap(&mut self.prompt_history, &mut editor.history);
        }
    }

    pub(in crate::app) fn prune_composer_editors(&mut self) {
        let stale: Vec<_> = self
            .composer_editors
            .iter()
            .filter_map(|(id, editor)| {
                let retained = editor
                    .binding
                    .target
                    .as_ref()
                    .and_then(|target| target.session)
                    .is_some_and(|session| {
                        self.session(session).is_some()
                            && self.document_pane(DocumentTab::Chat(session)) == Pane::Reference
                    });
                (!retained || !editor.registered).then_some(*id)
            })
            .collect();
        for id in stale {
            let editor = self.composer_editors.get_mut(&id).unwrap();
            editor.registered = false;
            if editor.scope_count == 0 {
                self.composer_editors.remove(&id);
            }
        }
    }

    /// Only editor-local UI state is scoped. Selection, workspace, input entity,
    /// command catalogs and runtime ownership remain untouched, even on unwind.
    pub(in crate::app) fn with_composer<R>(
        &mut self,
        binding: ComposerBinding,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut Self, &mut Context<Self>) -> R,
    ) -> Option<R> {
        if !self.composer_binding_live(&binding) {
            return None;
        }
        if binding.pane == Pane::Reference {
            self.composer_editors
                .get_mut(&binding.input.entity_id())
                .expect("registered composer")
                .scope_count += 1;
        }
        let previous = self.composer_scope.take();
        if let Some(previous) = &previous {
            self.swap_composer_editor(previous);
        }
        self.swap_composer_editor(&binding);
        self.composer_scope = Some(binding.clone());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(self, cx)));
        self.swap_composer_editor(&binding);
        if let Some(previous) = &previous {
            self.swap_composer_editor(previous);
        }
        self.composer_scope = previous;
        if binding.pane == Pane::Reference {
            let id = binding.input.entity_id();
            let editor = self.composer_editors.get_mut(&id).unwrap();
            editor.scope_count -= 1;
            if !editor.registered && editor.scope_count == 0 {
                self.composer_editors.remove(&id);
            }
        }
        match result {
            Ok(result) => Some(result),
            Err(error) => std::panic::resume_unwind(error),
        }
    }

    pub(in crate::app) fn composer_listener<E: 'static>(
        &self,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &E, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        let binding = self.composer_binding();
        cx.listener(move |this, event, window, cx| {
            if !this.composer_binding_live(&binding) {
                // A stale pane action must not fall through to the window's
                // main-composer shortcuts after navigation or a tab move.
                cx.stop_propagation();
                return;
            }
            this.with_composer(binding.clone(), cx, |this, cx| {
                action(this, event, window, cx)
            });
        })
    }

    pub(in crate::app) fn composer_input(&self) -> Entity<TextareaState> {
        self.composer_scope
            .as_ref()
            .map_or_else(|| self.input.clone(), |b| b.input.clone())
    }

    pub(in crate::app) fn composer_session(&self) -> Option<u64> {
        self.composer_scope
            .as_ref()
            .map_or(self.selected_session, |b| {
                b.target.as_ref().and_then(|t| t.session)
            })
    }

    pub(in crate::app) fn composer_workspace(&self) -> Option<usize> {
        self.composer_scope
            .as_ref()
            .map_or(self.selected_workspace, |b| {
                b.target.as_ref().and_then(|t| {
                    self.store
                        .workspaces
                        .iter()
                        .position(|w| w.path == t.workspace)
                })
            })
    }

    pub(in crate::app) fn composer_is_main(&self) -> bool {
        self.composer_scope
            .as_ref()
            .is_none_or(|b| b.pane == Pane::Main)
    }

    pub(in crate::app) fn composer_read_only(&self) -> bool {
        if self.composer_is_main() {
            self.viewing_subagent()
        } else {
            self.composer_session()
                .and_then(|id| self.session(id))
                .is_none_or(|s| s.is_subagent || s.archived)
        }
    }

    pub(in crate::app) fn composer_provider_for(&self, binding: &ComposerBinding) -> Provider {
        if binding.pane == Pane::Main {
            return self.new_session_provider;
        }
        self.composer_editors
            .get(&binding.input.entity_id())
            .map(|e| e.provider)
            .or_else(|| {
                binding
                    .target
                    .as_ref()
                    .and_then(|t| t.session)
                    .and_then(|id| self.session(id))
                    .map(|s| s.provider)
            })
            .unwrap_or(self.new_session_provider)
    }

    pub(in crate::app) fn composer_provider(&self) -> Provider {
        self.composer_provider_for(&self.composer_binding())
    }

    pub(in crate::app) fn set_composer_provider(&mut self, provider: Provider) {
        if self.composer_is_main() {
            self.new_session_provider = provider;
        } else if let Some(editor) = self
            .composer_scope
            .as_ref()
            .and_then(|b| self.composer_editors.get_mut(&b.input.entity_id()))
        {
            editor.provider = provider;
        }
    }

    pub(in crate::app) fn composer_pending_selection(&self) -> Option<&ComposerSelection> {
        if self.composer_is_main() {
            self.pending_model_selection.as_ref()
        } else {
            self.composer_scope
                .as_ref()
                .and_then(|b| self.composer_editors.get(&b.input.entity_id()))
                .and_then(|e| e.selection.as_ref())
        }
    }

    pub(in crate::app) fn set_composer_pending_selection(
        &mut self,
        selection: Option<ComposerSelection>,
    ) {
        if self.composer_is_main() {
            self.pending_model_selection = selection;
        } else if let Some(editor) = self
            .composer_scope
            .as_ref()
            .and_then(|b| self.composer_editors.get_mut(&b.input.entity_id()))
        {
            editor.selection = selection;
        }
    }

    pub(in crate::app) fn composer_permission(&self) -> PermissionLevel {
        if self.composer_is_main() {
            self.pending_permission
        } else {
            self.composer_scope
                .as_ref()
                .and_then(|b| self.composer_editors.get(&b.input.entity_id()))
                .map_or(self.store.ui.default_session_permission, |e| e.permission)
        }
    }

    pub(in crate::app) fn set_composer_permission(&mut self, permission: PermissionLevel) {
        if self.composer_is_main() {
            self.pending_permission = permission;
        } else if let Some(editor) = self
            .composer_scope
            .as_ref()
            .and_then(|b| self.composer_editors.get_mut(&b.input.entity_id()))
        {
            editor.permission = permission;
        }
    }

    pub(in crate::app) fn composer_selection_for(
        &self,
        binding: &ComposerBinding,
    ) -> ComposerSelection {
        let provider = self.composer_provider_for(binding);
        if let Some(session) = binding
            .target
            .as_ref()
            .and_then(|t| t.session)
            .and_then(|id| self.session(id))
            .filter(|s| s.provider == provider)
        {
            return ComposerSelection::from_session(session);
        }
        let pending = if binding.pane == Pane::Main {
            self.pending_model_selection.as_ref()
        } else {
            self.composer_editors
                .get(&binding.input.entity_id())
                .and_then(|e| e.selection.as_ref())
        };
        pending
            .filter(|s| s.engine == provider)
            .cloned()
            .unwrap_or_else(|| {
                binding
                    .target
                    .as_ref()
                    .and_then(|t| {
                        self.store
                            .workspaces
                            .iter()
                            .position(|w| w.path == t.workspace)
                    })
                    .map_or_else(
                        || self.default_selection(provider),
                        |index| self.workspace_default_selection(index, provider),
                    )
            })
    }

    pub(in crate::app) fn composer_session_for(&self, binding: &ComposerBinding) -> Option<u64> {
        let provider = self.composer_provider_for(binding);
        binding
            .target
            .as_ref()
            .and_then(|t| t.session)
            .filter(|id| self.session(*id).is_some_and(|s| s.provider == provider))
    }

    pub(in crate::app) fn render_composer_for(
        &mut self,
        binding: ComposerBinding,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.prune_composer_editors();
        self.with_composer(binding, cx, |this, cx| {
            this.render_bound_composer(window, cx)
        })
        .unwrap_or_else(|| div().into_any_element())
    }

    pub(in crate::app) fn send_prompt_for(
        &mut self,
        binding: ComposerBinding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_composer(binding, cx, |this, cx| this.send_prompt(window, cx));
    }

    pub(in crate::app) fn select_composer_engine(
        &mut self,
        provider: Provider,
        cx: &mut Context<Self>,
    ) {
        if !self.writable
            || self.closing
            || !self.store.cli.enabled(provider)
            || self.composer_save_pending()
        {
            return;
        }
        if self.composer_provider() != provider {
            self.set_composer_pending_selection(None);
            let target = self.composer_session().filter(|id| {
                !self.session_busy(*id)
                    && !self.session_control.queue_saves.contains_key(id)
                    && !self.session_draft_save_pending(*id)
                    && self.session(*id).is_some_and(|s| {
                        s.transcript.loaded
                            && s.messages.is_empty()
                            && s.native_id.is_none()
                            && s.exit_code.is_none()
                            && !s.interrupted
                    })
            });
            if let Some(id) = target {
                let selection = self.default_selection(provider);
                if let Some(session) = self.session_mut(id) {
                    session.provider = provider;
                    selection.apply(session);
                }
                self.persist();
            }
        }
        self.set_composer_provider(provider);
        self.remember_provider(provider);
        if provider == Provider::OpenCode {
            self.ensure_composer_opencode_models(false, cx);
        }
        cx.notify();
    }

    pub(in crate::app) fn new_composer_session(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.composer_workspace() else {
            return;
        };
        if !self.writable || self.closing {
            return;
        }
        if self.composer_is_main() {
            self.new_session_here(index, window, cx);
            return;
        }
        let current_provider = self.composer_provider();
        let explicit = self
            .composer_pending_selection()
            .is_some_and(|s| s.engine == current_provider)
            || self
                .composer_session()
                .and_then(|id| self.session(id))
                .is_some_and(|s| s.provider != current_provider);
        let provider = if explicit {
            current_provider
        } else {
            self.workspace_latest_selection(index, None)
                .map_or(current_provider, |s| s.engine)
        };
        let selection = if explicit {
            self.composer_selection_for(&self.composer_binding())
        } else {
            self.workspace_default_selection(index, provider)
        };
        let mut session = self.prepare_workspace_session(index, provider, false);
        selection.apply(&mut session);
        session.permission = self.store.ui.default_session_permission;
        let id = session.id;
        self.next_id += 1;
        self.store.workspaces[index].sessions.insert(0, session);
        self.session_control.changed(id);
        self.persist();
        self.show_chat_beside(id, window, cx);
    }

    /// Resolve the currently visible owner after a send, including pane moves.
    pub(in crate::app) fn live_composer_for_target(
        &self,
        target: &ComposerTarget,
    ) -> Option<ComposerBinding> {
        let main = ComposerBinding {
            pane: Pane::Main,
            target: self
                .selected_workspace
                .and_then(|i| self.store.workspaces.get(i))
                .map(|w| ComposerTarget {
                    workspace: w.path.clone(),
                    session: self.selected_session,
                }),
            input: self.input.clone(),
        };
        if main.target.as_ref() == Some(target) {
            return Some(main);
        }
        self.composer_editors
            .values()
            .map(|editor| &editor.binding)
            .find(|binding| {
                binding.target.as_ref() == Some(target) && self.composer_binding_live(binding)
            })
            .cloned()
    }
}
