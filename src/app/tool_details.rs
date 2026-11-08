//! Session-scoped tool selection and content, independent of the right container.
use super::plugin_ui::right::{RightBody, RightView};
use super::*;
use conduit::plugins::{
    builtins,
    context::{ScopeKey, SessionKey},
    state::ViewInstanceKey,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct ToolDetailState {
    selected: BTreeMap<SessionKey, String>,
    scrolls: BTreeMap<(SessionKey, String), (ScrollHandle, u64)>,
    clock: u64,
}
impl ToolDetailState {
    pub(super) fn selected(&self, session: &SessionKey) -> Option<&str> {
        self.selected.get(session).map(String::as_str)
    }
    pub(super) fn select(&mut self, session: &SessionKey, message: &str) {
        self.selected.insert(session.clone(), message.into());
        self.scroll(session, message);
    }
    pub(super) fn scroll(&mut self, session: &SessionKey, message: &str) -> ScrollHandle {
        self.clock = self.clock.saturating_add(1);
        let key = (session.clone(), message.to_owned());
        let entry = self.scrolls.entry(key.clone()).or_default();
        entry.1 = self.clock;
        let scroll = entry.0.clone();
        // Keep the active tool and at most 31 hidden tool scroll positions.
        while self.scrolls.len() > 32 {
            let oldest = self
                .scrolls
                .iter()
                .filter(|(candidate, _)| **candidate != key)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone())
                .unwrap();
            self.scrolls.remove(&oldest);
            if self.selected.get(&oldest.0) == Some(&oldest.1) {
                self.selected.remove(&oldest.0);
            }
        }
        scroll
    }
    pub(super) fn forget_session(&mut self, id: u64) {
        self.selected.retain(|session, _| session.id != id);
        self.scrolls.retain(|(session, _), _| session.id != id);
    }
}

fn detail_section(
    scope: &str,
    name: &str,
    label: &str,
    text: &str,
    empty: &str,
    typography: &ChatTypography,
) -> AnyElement {
    let text = format_tool_text(text);
    let copy = text.clone();
    v_flex()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .overflow_hidden()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface))
        .child(
            h_flex()
                .h(px(36.))
                .px_3()
                .items_center()
                .border_b_1()
                .border_color(rgb(palette().border))
                .bg(rgb(palette().surface_strong))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .font_semibold()
                        .child(label.to_owned()),
                )
                .child(
                    Button::new(SharedString::from(format!("{scope}-{name}-copy")))
                        .ghost()
                        .small()
                        .compact()
                        .icon(FullIconName::Copy)
                        .tooltip(conduit::i18n::format(
                            "tool_details.copy",
                            &[("label", label.to_string())],
                        ))
                        .accessibility_label(conduit::i18n::format(
                            "tool_details.copy",
                            &[("label", label.to_string())],
                        ))
                        .disabled(text.trim().is_empty())
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        }),
                ),
        )
        .child(
            div()
                .debug_selector({
                    let name = name.to_owned();
                    move || format!("detail-{name}")
                })
                .w_full()
                .min_w_0()
                .p_3()
                .font_family(typography.code_font.clone())
                .text_size(px(typography.code_size))
                .line_height(relative(1.6))
                .text_color(rgb(if text.trim().is_empty() {
                    palette().muted
                } else {
                    palette().text
                }))
                .child(gpui_kit::base::SelectableText::new(
                    SharedString::from(format!("{scope}-{name}-text")),
                    if text.trim().is_empty() {
                        empty.to_owned()
                    } else {
                        text
                    },
                )),
        )
        .into_any_element()
}

impl ConduitApp {
    pub(super) fn open_tool_details(
        &mut self,
        session_id: u64,
        message: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_plugin_context();
        let Some(session) = self.plugins.session_key().filter(|s| s.id == session_id) else {
            return;
        };
        if !self
            .session(session_id)
            .is_some_and(|s| s.messages.iter().any(|m| m.key == message))
        {
            return;
        }
        if let Some(command) = self.plugins.host_command(builtins::SHOW_TOOL_DETAILS)
            && self.dispatch_plugin_command(&command, window, cx)
        {
            self.tool_details.select(&session, message);
        }
    }

    pub(super) fn render_tool_details(
        &mut self,
        key: &ViewInstanceKey,
        _: &Window,
        _: &mut Context<Self>,
    ) -> RightView {
        let ScopeKey::Session { workspace, id } = &key.scope else {
            return RightBody::Failed(
                conduit::i18n::text_static("tool_details.tool_details_require_session_context")
                    .into(),
            )
            .into();
        };
        let session = SessionKey {
            workspace: workspace.clone(),
            id: *id,
        };
        if self.plugins.session_key().as_ref() != Some(&session) {
            return RightBody::Failed(
                conduit::i18n::text_static("tool_details.session_context_changed").into(),
            )
            .into();
        }
        let Some(record) = self.session(session.id) else {
            return RightBody::Failed(
                conduit::i18n::text_static("tool_details.the_session_no_longer_exists").into(),
            )
            .into();
        };
        if !record.transcript.loaded {
            return self
                .transcripts
                .errors
                .get(&session.id)
                .cloned()
                .map(RightBody::Failed)
                .unwrap_or(RightBody::Loading)
                .into();
        }
        let native_id = record.native_id.clone();
        let item = self
            .tool_details
            .selected(&session)
            .and_then(|key| record.messages.iter().find(|m| m.key == key))
            .cloned();
        let typography = self.chat_typography();
        let mut actions = Vec::new();
        if let Some(item) = &item {
            let copy = conduit::i18n::format(
                "tools.input_output_result_eddfa0",
                &[
                    ("arg0", format_tool_text(&item.input).to_string()),
                    ("arg1", format_tool_text(&item.detail).to_string()),
                ],
            );
            actions.push(
                Button::new("copy-details")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::Copy)
                    .tooltip(conduit::i18n::text_static(
                        "tool_details.copy_all_input_and_output",
                    ))
                    .accessibility_label(conduit::i18n::text_static(
                        "tool_details.copy_all_input_and_output",
                    ))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                    })
                    .into_any_element(),
            );
        }
        let body: AnyElement = if let Some(item) = item {
            let scope = format!("dock-{}-{}", session.id, item.key);
            let scroll = self.tool_details.scroll(&session, &item.key);
            let running = tool_running(&item);
            let failed = tool_failed(&item);
            let status = if running {
                conduit::i18n::text_static("tool_details.running")
            } else if failed {
                conduit::i18n::text_static("tool_details.unsuccessful")
            } else if item.status.is_empty() {
                conduit::i18n::text_static("tool_details.no_status_provided")
            } else {
                &item.status
            };
            let color = if running {
                palette().activity_running
            } else if failed {
                palette().diff_remove_text
            } else {
                palette().diff_add_text
            };
            let path = self
                .selected_workspace
                .map(|i| display_path(&self.store.workspaces[i].path))
                .unwrap_or_default();
            let mut content = v_flex()
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .p_3()
                .gap_3()
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(14.))
                                        .font_semibold()
                                        .child(tool_category(&item)),
                                )
                                .child(
                                    h_flex()
                                        .flex_shrink_0()
                                        .items_center()
                                        .gap_1()
                                        .px_2()
                                        .py_1()
                                        .rounded(px(6.))
                                        .bg(rgb(palette().surface_strong))
                                        .text_size(px(11.))
                                        .text_color(rgb(color))
                                        .child(
                                            Icon::new(if running {
                                                FullIconName::Clock
                                            } else if failed {
                                                FullIconName::CircleX
                                            } else {
                                                FullIconName::CircleCheck
                                            })
                                            .small(),
                                        )
                                        .child(conduit::i18n::source(status).to_owned()),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(palette().muted))
                                .child(match item.text.as_str() {
                                    "修改文件" => conduit::i18n::text("history.modify_file"),
                                    "命令输出" => {
                                        conduit::i18n::text("protocol.command_output")
                                    }
                                    _ => item.text.clone(),
                                }),
                        )
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .text_size(px(11.))
                                .text_color(rgb(palette().muted))
                                .child(path),
                        ),
                )
                .child(detail_section(
                    &scope,
                    "input",
                    conduit::i18n::text_static("tools.input"),
                    &item.input,
                    conduit::i18n::text_static("tools.no_input_provided"),
                    &typography,
                ))
                .child(detail_section(
                    &scope,
                    "output",
                    conduit::i18n::text_static("tools.output_result"),
                    &item.detail,
                    if running {
                        conduit::i18n::text_static("tools.waiting_for_tool_output")
                    } else {
                        conduit::i18n::text_static("tools.no_output")
                    },
                    &typography,
                ));
            if let Some(native_id) = native_id {
                content = content.child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(palette().muted))
                                .child(conduit::i18n::text_static(
                                    "tool_details.native_session_id",
                                )),
                        )
                        .child(gpui_kit::base::SelectableText::new(
                            SharedString::from(format!("{scope}-native-id")),
                            native_id,
                        )),
                );
            }
            div()
                .id(SharedString::from(scope))
                .test_support()
                .role(Role::Group)
                .aria_label(conduit::i18n::text_static("tool_details.tool_call_details"))
                .debug_selector(|| "detail-content".into())
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .min_w_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(content)
                .vertical_scrollbar(&scroll)
                .into_any_element()
        } else {
            return RightBody::Empty {
                title: conduit::i18n::text_static("tool_details.select_a_tool_call"),
                description: conduit::i18n::text_static(
                    "tool_details.expand_a_tool_card_in_chat_and",
                ),
            }
            .into();
        };
        RightView {
            body: RightBody::Content(body),
            actions,
        }
    }
}
