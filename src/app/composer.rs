//! Composer attachments and file selection.

use super::*;

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
        details.push(format!("另 {} 项", rejected.len() - 3));
    }
    format!(" {} 项（{}）", rejected.len(), details.join("；"))
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
    pub(super) fn composer_target(&self) -> Option<ComposerTarget> {
        self.selected_workspace
            .and_then(|index| self.store.workspaces.get(index))
            .map(|workspace| ComposerTarget {
                workspace: workspace.path.clone(),
                session: self.selected_session,
            })
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
            self.message = Some("请先在左上角选择工作区。".into());
            cx.notify();
            return false;
        };
        let (references, rejected) = file_ref::resolve(paths, &target.workspace);
        let accepted = !references.is_empty();
        let draft = self.composer_files.entry(target).or_default();
        let added = file_ref::merge_references(&mut draft.references, references);
        draft.feedback = if !rejected.is_empty() {
            Some(format!(
                "已添加 {added} 个文件，跳过{}",
                rejection_summary(&rejected)
            ))
        } else if added == 0 {
            Some("这些文件已在待发送列表中，无需重复添加。".into())
        } else {
            None
        };
        self.input.read(cx).focus_handle(cx).focus(window, cx);
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
        self.input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// The prompts already sent in the selected conversation, newest first.
    /// Recall browses the prose the user typed: the stored item keeps the
    /// trailing file-reference block added at send time, and an
    /// attachment-only message has no prose to browse.
    pub(super) fn sent_prompts(&self) -> Vec<String> {
        const LIMIT: usize = 100;
        let Some(session) = self.selected_session.and_then(|id| self.session(id)) else {
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
        self.input.update(cx, |input, cx| {
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
            None if history.is_empty() || !self.input.read(cx).value().is_empty() => {
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
        let mut tray = v_flex().id("composer-files").px(px(3.)).pt(px(3.)).gap_2();
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
                                .w(px(18.))
                                .h(px(18.))
                                .flex_shrink_0()
                                .icon(FullIconName::X)
                                .tooltip("移除文件")
                                .accessibility_label("移除文件")
                                .disabled(!enabled)
                                .when(enabled, |button| button.cursor_pointer())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.remove_file(&path, window, cx);
                                })),
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
                            .tooltip("关闭提示")
                            .accessibility_label("关闭文件提示")
                            .on_click(cx.listener(|this, _, _, cx| {
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
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("添加文件到对话".into()),
        });
        cx.spawn_in(window, async move |this, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                let _ = this.update_in(cx, |this, window, cx| {
                    // Native dialogs are asynchronous: a later selection must never
                    // land in a different workspace or conversation.
                    if this.composer_target().as_ref() == Some(&target) {
                        this.attach_paths(&paths, window, cx);
                    }
                });
            }
            Ok(Ok(None)) => {}
            other => {
                let _ = this.update(cx, |this, cx| {
                    if this.composer_target().as_ref() == Some(&target) {
                        this.composer_files.entry(target).or_default().feedback =
                            Some(format!("打开文件选择器失败：{other:?}"));
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }
}

impl ConduitApp {
    pub(super) fn render_composer(
        &self,
        selected: Option<&SessionSummary>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app_entity = cx.entity();
        let typography = self.chat_typography();
        let busy = self.send_blocked(true).is_some();
        let has_workspace = self.selected_workspace.is_some();
        let can_edit = self.writable && !self.closing;
        let input_focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let mentions_open = self.file_mentions_open(window, cx);
        let has_content = !self.input.read(cx).value().trim().is_empty()
            || self
                .current_files()
                .is_some_and(|files| !files.references.is_empty());
        let send_ready = can_edit && !busy && has_workspace && has_content;
        let composer_active = self
            .send_target(true)
            .is_some_and(|id| self.runs.contains_key(&id));
        // The latest-message arrow appears
        // only when the user has scrolled away from the end of the message list.
        let show_latest_button = !self.at_bottom
            && selected
                .as_ref()
                .is_some_and(|session| session.message_count > 0);
        let session_id = selected
            .as_ref()
            .map(|session| session.id)
            .unwrap_or_default();
        let model_picker = self.composer_model_picker(cx.entity(), session_id, !can_edit);
        // One composer for every state: an active session, a workspace without
        // a session (the first send creates one with the last engine), and no
        // workspace at all. Keeping the same shell in all three states keeps
        // the chat area from changing size when the session state changes.
        let composer_enabled = can_edit && has_workspace;
        let usage_parts = self.usage_status_parts();
        let mut composer = v_flex()
            .id("composer")
            .key_context(if mentions_open {
                "ConduitComposer FileMention"
            } else {
                "ConduitComposer"
            })
            .on_action(cx.listener(|this, _: &SendPrompt, window, cx| {
                cx.stop_propagation();
                if this.composer_is_composing(window, cx) {
                    return;
                }
                if this.file_mentions_open(window, cx) {
                    this.accept_file_mention(window, cx);
                } else {
                    this.send_prompt(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousFileMention, window, cx| {
                cx.stop_propagation();
                this.move_file_mention(-1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextFileMention, window, cx| {
                cx.stop_propagation();
                this.move_file_mention(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &AcceptFileMention, window, cx| {
                cx.stop_propagation();
                this.accept_file_mention(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DismissFileMention, window, cx| {
                cx.stop_propagation();
                if !this.composer_is_composing(window, cx) {
                    this.dismiss_file_mentions(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousPrompt, window, cx| {
                cx.stop_propagation();
                this.previous_prompt(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextPrompt, window, cx| {
                cx.stop_propagation();
                this.next_prompt(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Newline, window, cx| {
                cx.stop_propagation();
                this.input
                    .update(cx, |input, cx| input.insert("\n", window, cx));
                cx.notify();
            }))
            .w_full()
            .px(px(3.))
            .py(px(3.))
            .gap_1()
            .rounded(px(22.))
            .bg(rgb(palette().surface))
            .border_1()
            .border_color(if input_focused {
                rgb(palette().focus)
            } else {
                rgb(palette().border)
            })
            .on_click(cx.listener(|this, _, window, cx| {
                if this.selected_workspace.is_some() {
                    this.input.read(cx).focus_handle(cx).focus(window, cx);
                }
            }))
            // 资源管理器拖入的文件由 gpui 翻译成 ExternalPaths 载荷的拖拽事件，
            // 与右栏调宽、文本选择这些内部拖拽按载荷类型隔离，不会互相抢事件。
            .on_drop(cx.listener(|this, dropped: &ExternalPaths, window, cx| {
                this.attach_paths(dropped.paths(), window, cx);
            }))
            .drag_over::<ExternalPaths>(|style, _, _, _| {
                style
                    .bg(rgb(palette().surface_hover))
                    .border_color(rgb(palette().focus))
            })
            .when_some(
                self.render_composer_files(composer_enabled, cx),
                |composer, files| composer.child(files),
            )
            .child(div().px(px(3.)).pt(px(3.)).child({
                let mut composer_input = Textarea::new(&self.input)
                    .appearance(false)
                    .bordered(false)
                    .text_size(px(typography.body_size))
                    .line_height(relative(typography.line_height))
                    .disabled(self.closing || !composer_enabled);
                if let Some(family) = typography.body_font.clone() {
                    composer_input = composer_input.font_family(family);
                }
                // 资源管理器 Ctrl+C 文件后在这里 Ctrl+V，剪贴板会以 ExternalPaths
                // 报出路径；没有文件时返回 false，交回默认文本粘贴。
                let paste_app = app_entity.clone();
                composer_input.on_paste(move |item, window, cx| {
                    let paths: Vec<PathBuf> = item
                        .entries
                        .iter()
                        .filter_map(|entry| match entry {
                            ClipboardEntry::ExternalPaths(dropped) => {
                                Some(dropped.paths().to_vec())
                            }
                            _ => None,
                        })
                        .flatten()
                        .collect();
                    if paths.is_empty() {
                        return false;
                    }
                    paste_app.update(cx, |this, cx| {
                        this.attach_paths(&paths, window, cx);
                    });
                    true
                })
            }))
            .child(
                h_flex()
                    .px(px(3.))
                    .pb(px(3.))
                    .pt(px(3.))
                    .gap_2()
                    .items_center()
                    .when(selected.is_some(), |row| {
                        row.child(self.permission_picker(app_entity.clone()))
                    })
                    .when_some(usage_parts, |row, parts| {
                        row.child(h_flex().gap_3().items_center().min_w_0().children(parts))
                    })
                    .child(div().flex_1())
                    .child(model_picker)
                    .when(composer_enabled, |row| {
                        row.child(
                            message_action_button(
                                "attach-file".into(),
                                Icon::new(FullIconName::Paperclip),
                                "添加文件（也可输入 @、拖入或粘贴文件）",
                            )
                            .on_click(cx.listener(Self::pick_files)),
                        )
                    })
                    .child(if composer_active {
                        h_flex()
                            .id("stop")
                            .w(px(30.))
                            .h(px(30.))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(palette().text))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(palette().control_hover)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(run) =
                                    this.selected_session.and_then(|id| this.runs.get_mut(&id))
                                {
                                    run.cancel();
                                }
                                this.input.read(cx).focus_handle(cx).focus(window, cx);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .w(px(8.))
                                    .h(px(8.))
                                    .rounded(px(2.))
                                    .bg(rgb(palette().on_text)),
                            )
                            .into_any_element()
                    } else {
                        h_flex()
                            .id("send")
                            .w(px(30.))
                            .h(px(30.))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(if send_ready {
                                palette().text
                            } else {
                                palette().surface_strong
                            }))
                            .when(send_ready, |button| {
                                button
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(palette().control_hover)))
                                    .on_click(cx.listener(Self::send_message))
                            })
                            .child(Icon::new(IconName::ArrowUp).small().text_color(rgb(
                                if send_ready {
                                    palette().on_text
                                } else {
                                    palette().faint
                                },
                            )))
                            .into_any_element()
                    }),
            );
        if !self.store.ui.full_width_content {
            composer = composer.max_w(px(840.)).mx_auto();
        }
        let composer: AnyElement = composer.into_any_element();
        // Live engine activity stays on the left; the step and file-churn tags
        // dock to the right edge, just above the composer's top-right corner.
        let turn_activity = self
            .selected_session
            .and_then(|id| self.turn_activity_strip(id));
        let turn_stats = self
            .selected_session
            .and_then(|id| self.turn_stats_tags(id));
        let has_status_row = turn_activity.is_some() || turn_stats.is_some();
        // Live engine activity and the step/file tags share the transcription's
        // content width: the same 24px gutter message rows add inside the 8px
        // list padding, so the hint never looks wider than the chat content.
        let status_row = div().w_full().px_6().child(
            h_flex()
                .id("turn-status-row")
                .debug_selector(|| "turn-status-row".into())
                .w_full()
                .min_w_0()
                .when(!self.store.ui.full_width_content, |row| {
                    row.max_w(px(840.)).mx_auto()
                })
                .items_center()
                .justify_end()
                .gap_2()
                .pb(px(8.))
                .when_some(turn_activity, |row, activity| row.child(activity))
                .when_some(turn_stats, |row, tags| row.child(tags)),
        );
        // The "jump to latest" arrow floats at the lower-right of the message
        // area, just above the composer's top-right corner, and only exists
        // while the list is scrolled away from its end. It clears the status
        // row when that row is present.
        let latest_top = px(if has_status_row { -72. } else { -40. });
        let area = div().px_2().pt(px(12.)).pb_4().child(
            v_flex()
                .w_full()
                .items_center()
                .when(has_status_row, |column| column.child(status_row))
                .child(
                    div()
                        .relative()
                        .w_full()
                        .px_2()
                        // The extra 16px cap compensates for the doubled side
                        // gap so the composer keeps the messages' 840px column.
                        .when(!self.store.ui.full_width_content, |card| {
                            card.max_w(px(856.)).mx_auto()
                        })
                        .child(composer)
                        .when(mentions_open, |area| {
                            area.child(self.render_file_mentions(window, cx))
                        })
                        .when(show_latest_button, |area| {
                            area.child(
                                Button::new("scroll-to-latest")
                                    .ghost()
                                    .tooltip("转到最新消息")
                                    .accessibility_label("转到最新消息")
                                    .absolute()
                                    .top(latest_top)
                                    .right(px(14.))
                                    .w(px(30.))
                                    .h(px(30.))
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(palette().border))
                                    .bg(rgb(palette().surface_strong))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.scroll_to_end();
                                        this.at_bottom = true;
                                        this.message_focus = None;
                                        this.pending_message_jump = false;
                                        cx.notify();
                                    }))
                                    .child(
                                        Icon::new(gpui_kit::assets::IconName::ArrowDown)
                                            .small()
                                            .text_color(rgb(palette().muted)),
                                    ),
                            )
                        }),
                ),
        );
        area.into_any_element()
    }
}
