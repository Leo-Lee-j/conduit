//! Language changes update presentation while preserving all user state.
use super::*;
use conduit::i18n;

impl ConduitApp {
    pub(super) fn select_language(
        &mut self,
        language: i18n::Locale,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.store.ui.language == language {
            return;
        }
        self.store.ui.language = language.clone();
        i18n::set_locale(&language);
        self.refresh_localized_inputs(window, cx);
        self.tool_analysis = Default::default();
        self.files.refresh_language(window, cx);
        if let Some(view) = &self.branch_manager {
            view.update(cx, |view, cx| view.refresh_language(window, cx));
        }
        for select in [
            &self.ui_font_select,
            &self.chat_font_select,
            &self.code_font_select,
        ] {
            select.update(cx, |_, cx| cx.notify());
        }
        self.timeline.update(cx, |_, cx| cx.notify());
        self.sidebar_view.update(cx, |_, cx| cx.notify());
        self.file_view.update(cx, |_, cx| cx.notify());
        if let Some(view) = &self.notifications.view {
            view.update(cx, |_, cx| cx.notify());
        }
        self.persist();
        cx.refresh_windows();
        cx.notify();
    }

    fn refresh_localized_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (input, original) in [
            (&self.search, "搜索会话"),
            (&self.settings_ui.search, "搜索设置…"),
            (
                &self.connection_settings.model_search,
                "搜索模型名称、ID 或备注…",
            ),
            (&self.extensions_ui.searches[0], "搜索名称或简介…"),
            (&self.extensions_ui.searches[1], "搜索 MCP 名称…"),
        ] {
            input.update(cx, |input, cx| {
                input.set_placeholder(i18n::source(original).to_string(), window, cx)
            });
        }
        for draft in self.git.drafts.values() {
            draft.input.update(cx, |input, cx| {
                input.set_placeholder(i18n::source("填写 Commit Message…").to_string(), window, cx)
            });
            draft.tag.update(cx, |input, cx| {
                input.set_placeholder(
                    i18n::source("Tag（可选，例如 v1.2.3）").to_string(),
                    window,
                    cx,
                )
            });
        }
        self.input.update(cx, |input, cx| {
            input.set_placeholder(
                i18n::source(
                    "描述任务，输入 @ 引用文件…\nEnter 发送（运行中自动排队）· Shift+Enter 换行",
                )
                .to_string(),
                window,
                cx,
            )
        });
        self.git_prompt_input.update(cx, |input, cx| {
            input.set_placeholder(
                i18n::source("填写 Commit Message 生成提示词…").to_string(),
                window,
                cx,
            )
        });
    }
}
