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

    /// All three entry points share validation, deduplication and inline feedback.
    pub(super) fn attach_paths(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing || !self.writable || paths.is_empty() {
            return;
        }
        let Some(target) = self.composer_target() else {
            self.message = Some("请先在左上角选择工作区。".into());
            cx.notify();
            return;
        };
        let (references, rejected) = file_ref::resolve(paths, &target.workspace);
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
        let provider = selected.as_ref().map(|session| session.provider);
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
        let selected_model = selected.as_ref().and_then(|session| session.model.clone());
        let model_picker = provider
            .map(|provider| {
                self.composer_model_picker(
                    cx.entity(),
                    session_id,
                    provider,
                    selected_model,
                    !can_edit,
                )
            })
            .unwrap_or_else(|| div().into_any_element());
        // One composer for every state: an active session, a workspace without
        // a session (the first send creates one with the last engine), and no
        // workspace at all. Keeping the same shell in all three states keeps
        // the chat area from changing size when the session state changes.
        let composer_enabled = can_edit && has_workspace;
        let usage_parts = self.usage_status_parts();
        let mut composer = v_flex()
            .id("composer")
            .key_context("ConduitComposer")
            .on_action(cx.listener(|this, _: &SendPrompt, window, cx| {
                cx.stop_propagation();
                this.send_prompt(window, cx)
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
                    .child(self.engine_picker(app_entity.clone()))
                    .when(selected.is_some(), |row| {
                        row.child(self.permission_picker(app_entity.clone()))
                    })
                    .when_some(usage_parts, |row, parts| {
                        row.child(h_flex().gap_3().items_center().min_w_0().children(parts))
                    })
                    .child(div().flex_1())
                    .when(selected.is_some(), |row| row.child(model_picker))
                    .when(composer_enabled, |row| {
                        row.child(
                            message_action_button(
                                "attach-file".into(),
                                Icon::new(FullIconName::Paperclip),
                                "添加文件（支持拖入或粘贴文件）",
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
        // The "jump to latest" arrow floats at the lower-right of the message
        // area, just above the composer's top-right corner, and only exists
        // while the list is scrolled away from its end.
        let turn_stats = self
            .selected_session
            .and_then(|id| self.turn_stats_pill(id, cx));
        let area = div().px_2().pt_1().pb(px(5.)).child(
            v_flex()
                .w_full()
                .items_center()
                .when_some(turn_stats, |column, stats| column.child(stats))
                .child(
                    div()
                        .relative()
                        .w_full()
                        .when(!self.store.ui.full_width_content, |card| {
                            card.max_w(px(840.)).mx_auto()
                        })
                        .child(composer)
                        .when(show_latest_button, |area| {
                            area.child(
                                Button::new("scroll-to-latest")
                                    .ghost()
                                    .tooltip("转到最新消息")
                                    .accessibility_label("转到最新消息")
                                    .absolute()
                                    .top(px(-40.))
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
