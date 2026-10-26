//! General settings.

use super::*;

impl ConduitApp {
    pub(in crate::app) fn settings_session_content(&self, view: Entity<Self>) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let provider_picker = self.naming_provider_picker(view.clone(), disabled);
        let model_picker = self.naming_model_picker(view.clone(), disabled);
        let mode_picker =
            self.naming_mode_picker(view.clone(), disabled || !self.store.ui.auto_rename);
        let rename_view = view.clone();
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("会话命名"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child("立即生效"),
                            ),
                    )
                    .child(settings_control_row(
                        "自动命名", "自动生成和更新会话标题；关闭后仍可在会话菜单手动使用 AI 重命名",
                        Switch::new("auto-rename-setting").checked(self.store.ui.auto_rename).disabled(disabled)
                            .on_change(move |checked, _, cx| {
                                rename_view.update(cx, |this, cx| {
                                    if !this.writable || this.closing { return; }
                                    this.store.ui.auto_rename = *checked;
                                    if !*checked {
                                        this.session_namings.retain(|_, naming| {
                                            if naming.automatic { naming.runtime.cancel(); false } else { true }
                                        });
                                    }
                                    this.persist();
                                    cx.notify();
                                });
                            }).into_any_element(), false))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .child(settings_label("命名引擎"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("生成会话标题使用的引擎；默认跟随当前会话"),
                            )
                            .child(provider_picker),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(settings_label("命名模型"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("指定引擎后可固定模型；默认跟随当前会话的模型"),
                            )
                            .child(model_picker),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(settings_label("触发方式"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("选择标题跟随最新对话自动更新，或只按第一条消息生成一次"),
                            )
                            .child(mode_picker),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("命名在后台进行，失败时保留当前标题；命名完成前手动重命名的会话不会被覆盖。右键会话可随时使用 AI 重命名。"),
            )
            .into_any_element()
    }

    pub(in crate::app) fn settings_git_content(&self, view: Entity<Self>) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let provider_picker = self.git_provider_picker(view.clone(), disabled);
        let model_picker = self.git_model_picker(view.clone(), disabled);
        v_flex()
            .w_full()
            .gap_3()
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("生成引擎"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child("立即生效"),
                            ),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .child(settings_label("生成 CLI"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("用于生成 Commit Message 的 CLI；默认跟随当前会话"),
                            )
                            .child(provider_picker),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(settings_label("生成模型"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("指定 CLI 后可固定模型；默认跟随当前会话（或 CLI 默认）"),
                            )
                            .child(model_picker),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().font_semibold().text_size(px(13.)).child("Commit Message 提示词"))
                    .child(
                        Button::new("git-prompt-reset")
                            .label("恢复默认")
                            .small()
                            .ghost()
                            .disabled(disabled)
                            .when(!disabled, |button| button.cursor_pointer())
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.git.commit_message_prompt = crate::store::DEFAULT_COMMIT_PROMPT.into();
                                    this.git_prompt_input.update(cx, |input, cx| {
                                        input.set_value(crate::store::DEFAULT_COMMIT_PROMPT, window, cx);
                                    });
                                    this.persist();
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .child(
                div().text_size(px(12.)).text_color(rgb(palette().muted))
                    .child("设置生成说明的语言、格式和风格，应用于 Git 侧栏的 Generate 按钮。已暂存文件名与 diff 会自动附加，无需填写占位符。"),
            )
            .child(
                Textarea::new(&self.git_prompt_input)
                    .h(px(210.))
                    .w_full()
                    .text_size(px(12.))
                    .aria_label("Commit Message 生成提示词")
                    .disabled(disabled),
            )
            .child(
                div().text_size(px(11.)).text_color(rgb(palette().faint))
                    .child("自动保存，适用于所有工作区；留空时使用默认提示词。修改将在下一次生成时生效。"),
            )
            .into_any_element()
    }

    pub(in crate::app) fn settings_general_content(&self, view: Entity<Self>) -> AnyElement {
        let state_path = Store::path();
        let data_dir = state_path
            .parent()
            .map(display_path)
            .unwrap_or_else(|| "—".into());
        let state_file = display_path(&state_path);
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("本机数据"),
                            ),
                    )
                    .child(settings_row("数据目录", data_dir, false))
                    .child(settings_row("配置文件", state_file, true)),
            )
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("诊断"),
                            ),
                    )
                    .child({
                        let toggle = view.clone();
                        let disabled = !self.writable || self.closing;
                        let enabled = self.store.ui.performance_monitor;
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .child(settings_label("性能监测"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("在底部工具栏显示性能入口并采集指标"),
                            )
                            .child(
                                Switch::new("performance-monitor-setting")
                                    .checked(enabled)
                                    .disabled(disabled)
                                    .on_change(move |checked, _, cx| {
                                        toggle.update(cx, |this, cx| {
                                            if !this.writable || this.closing {
                                                return;
                                            }
                                            this.store.ui.performance_monitor = *checked;
                                            if *checked {
                                                this.performance
                                                    .update(cx, |monitor, _| monitor.reset());
                                            } else {
                                                this.performance_open = false;
                                            }
                                            this.persist();
                                            cx.notify();
                                        });
                                    }),
                            )
                    }),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("配置只保存在本机；外观与诊断设置随本地状态保存。"),
            )
            .into_any_element()
    }
}
