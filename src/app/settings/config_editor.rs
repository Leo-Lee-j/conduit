//! Native configuration editing with explicit save and conflict protection.
use super::*;
use crate::extensions::{Kind, edit};

struct ConfigEditor {
    app: Entity<ConduitApp>,
    path: PathBuf,
    original: Option<String>,
    text: Entity<TextareaState>,
    name: Option<Entity<InputState>>,
    error: Option<String>,
    saving: bool,
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{ConduitApp, InputState, McpForm, Provider, Store, TextareaState, show_editor};
    use gpui_kit::component::WindowExt;
    use gpui_kit::{AppContext as _, ParentElement as _, TestAppContext, px};

    #[gpui_kit::test]
    fn config_and_mcp_forms_keep_actions_visible_on_short_windows(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app =
                cx.new(|cx| ConduitApp::with_store(window, cx, Ok(Store::default()), false, false));
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        for (width, height) in [(850., 500.), (1280., 900.)] {
            cx.simulate_resize(gpui_kit::size(px(width), px(height)));
            for new_skill in [false, true] {
                cx.update(|window, cx| {
                    show_editor(
                        app.clone(),
                        "C:/example/skills".into(),
                        None,
                        "long line\n".repeat(50),
                        new_skill,
                        window,
                        cx,
                    )
                });
                cx.run_until_parked();
                cx.update(|window, cx| {
                    let _ = window.draw(cx);
                });
                let save = cx.debug_bounds("native-config-save").unwrap();
                let editor = cx.debug_bounds("native-config-editor").unwrap();
                assert!(save.bottom() <= px(height - 16.) && save.right() < px(width));
                assert!(save.top() >= editor.bottom());
                cx.update(|window, cx| window.close_dialog(cx));
            }
            cx.update(|window, cx| {
                let form = cx.new(|cx| McpForm {
                    app: app.clone(),
                    engine: Provider::Codex,
                    path: "C:/example/.codex/config.toml".into(),
                    original: None,
                    fields: [
                        cx.new(|cx| InputState::new(window, cx)),
                        cx.new(|cx| InputState::new(window, cx)),
                    ],
                    arguments: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 4)),
                    remote: false,
                    error: Some("请输入服务名称".into()),
                });
                window.open_dialog(cx, move |dialog, _, _| {
                    let form = form.clone();
                    dialog
                        .title("添加 MCP 服务")
                        .w(px(620.))
                        .margin_top(px(32.))
                        .content(move |body, _, _| body.child(form.clone()))
                });
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            let action = cx.debug_bounds("mcp-form-preview").unwrap();
            assert!(action.bottom() <= px(height - 16.));
            cx.update(|window, cx| window.close_dialog(cx));
        }
    }
}

struct McpForm {
    app: Entity<ConduitApp>,
    engine: Provider,
    path: PathBuf,
    original: Option<String>,
    fields: [Entity<InputState>; 2],
    arguments: Entity<TextareaState>,
    remote: bool,
    error: Option<String>,
}

impl Render for McpForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let form = v_flex().id("mcp-form").debug_selector(|| "mcp-form".into()).gap_3()
            .max_h((window.viewport_size().height - px(210.)).max(px(160.))).overflow_y_scroll()
            .child(div().text_size(px(12.)).text_color(rgb(palette().muted)).child(format!("{} · {}", self.engine.label(), display_path(&self.path))))
            .child(v_flex().gap_1().child("服务名称").child(Input::new(&self.fields[0]).aria_label("MCP 服务名称")))
            .child(h_flex().gap_2()
                .child(Button::new("mcp-local").small().outline().label("本地命令").selected(!self.remote).on_click(cx.listener(|this, _, _, cx| { this.remote = false; cx.notify(); })))
                .child(Button::new("mcp-remote").small().outline().label("远程 HTTP").selected(self.remote).on_click(cx.listener(|this, _, _, cx| { this.remote = true; cx.notify(); }))))
            .child(v_flex().gap_1().child(if self.remote { "服务 URL" } else { "可执行命令（如 npx、uvx 或完整路径）" })
                .child(Input::new(&self.fields[1]).aria_label("MCP 命令或地址")))
            .when(!self.remote, |body| body.child(v_flex().gap_1().child("参数（每行一个，无需额外引号）")
                .child(Textarea::new(&self.arguments).h(px(90.)).aria_label("MCP 命令参数"))))
            .child(div().text_size(px(11.)).text_color(rgb(palette().muted)).child("下一步预览合并后的配置；可继续添加环境变量、认证头等高级选项。保存后由 CLI 在会话中加载。"))
            .when_some(self.error.clone(), |body, error| body.child(div().text_size(px(12.)).text_color(rgb(palette().danger_text)).child(error)));
        v_flex().gap_3().child(form).child(
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("mcp-form-cancel")
                        .ghost()
                        .label("取消")
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .child(
                    Button::new("mcp-form-preview")
                        .debug_selector(|| "mcp-form-preview".into())
                        .primary()
                        .label("预览配置")
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !this.app.read(cx).writable || this.app.read(cx).closing {
                                return;
                            }
                            let result = crate::extensions::mcp::add_preview(
                                this.engine,
                                this.original.as_deref(),
                                this.fields[0].read(cx).value().trim(),
                                this.remote,
                                this.fields[1].read(cx).value().trim(),
                                this.arguments.read(cx).value().as_ref(),
                            );
                            match result {
                                Ok(text) => {
                                    window.close_dialog(cx);
                                    show_editor(
                                        this.app.clone(),
                                        this.path.clone(),
                                        this.original.clone(),
                                        text,
                                        false,
                                        window,
                                        cx,
                                    );
                                }
                                Err(error) => {
                                    this.error = Some(error.to_string());
                                    cx.notify();
                                }
                            }
                        })),
                ),
        )
    }
}

fn show_editor(
    app: Entity<ConduitApp>,
    path: PathBuf,
    original: Option<String>,
    text: String,
    new_skill: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let editor = cx.new(|cx| ConfigEditor {
        app,
        path,
        original,
        text: cx.new(|cx| {
            TextareaState::new(window, cx)
                .default_value(text)
                .auto_grow(6, 20)
        }),
        name: new_skill.then(|| cx.new(|cx| InputState::new(window, cx).default_value("my-skill"))),
        error: None,
        saving: false,
    });
    window.open_dialog(cx, move |dialog, window, cx| {
        let editor = editor.clone();
        let saving = editor.read(cx).saving;
        dialog
            .margin_top(px(32.))
            .w(px(
                (f32::from(window.viewport_size().width) - 60.).clamp(0., 820.)
            ))
            .title(if new_skill {
                "创建 Skill"
            } else {
                "编辑原生配置"
            })
            .overlay_closable(false)
            .keyboard(!saving)
            .close_button(!saving)
            .content(move |body, _, _| body.child(editor.clone()))
    });
}

impl ConfigEditor {
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || !self.app.read(cx).writable || self.app.read(cx).closing {
            return;
        }
        let path = if let Some(name) = &self.name {
            match edit::skill_path(&self.path, name.read(cx).value().trim()) {
                Ok(path) => path,
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            self.path.clone()
        };
        let text = self.text.read(cx).value().to_string();
        if let Err(error) = edit::validate(&path, &text) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let original = self.original.clone();
        self.saving = true;
        self.error = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { edit::save(&path, original.as_deref(), &text) })
                .await;
            let _ = this.update_in(cx, |editor, window, cx| {
                editor.saving = false;
                match result {
                    Ok(()) => {
                        editor.app.update(cx, |app, cx| {
                            app.extensions_ui.invalidate();
                            app.extensions_ui.feedback = Some(
                                if editor.original.is_some() {
                                    "配置已保存，原文件已备份；下次启动会话时加载。"
                                } else {
                                    "配置已创建；下次启动会话时加载。"
                                }
                                .into(),
                            );
                            app.ensure_extensions(window, cx, true);
                            app.reload_all_models_async(true, window, cx);
                            cx.notify();
                        });
                        window.close_dialog(cx);
                    }
                    Err(error) => editor.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for ConfigEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = (f32::from(window.viewport_size().height) - 360.).clamp(90., 390.);
        let body = v_flex()
            .id("native-config-editor")
            .debug_selector(|| "native-config-editor".into())
            .w_full()
            .max_h((window.viewport_size().height - px(190.)).max(px(140.)))
            .overflow_y_scroll()
            .gap_3()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(palette().muted))
                    .child(display_path(&self.path)),
            )
            .when_some(self.name.clone(), |body, name| {
                body.child(
                    v_flex().gap_1().child("技能文件夹名称").child(
                        Input::new(&name)
                            .disabled(self.saving)
                            .aria_label("技能文件夹名称"),
                    ),
                )
            })
            .child(
                div().h(px(height)).flex_shrink_0().w_full().child(
                    Textarea::new(&self.text)
                        .disabled(self.saving)
                        .h_full()
                        .aria_label("配置文件内容"),
                ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(palette().muted))
                    .child("保存前校验格式并保留原文件备份。生效范围以文件路径为准。"),
            )
            .when_some(self.error.clone(), |body, error| {
                body.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette().danger_text))
                        .child(error),
                )
            });
        v_flex().gap_3().child(body).child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("native-config-cancel")
                        .label("取消")
                        .ghost()
                        .disabled(self.saving)
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .child(
                    Button::new("native-config-save")
                        .debug_selector(|| "native-config-save".into())
                        .label(if self.saving {
                            "保存中…"
                        } else {
                            "保存配置"
                        })
                        .primary()
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
        )
    }
}

impl ConduitApp {
    pub(super) fn open_mcp_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable || self.closing {
            return;
        }
        let engine = self.extensions_ui.engine;
        let path = edit::destination(engine, self.extensions_ui.workspace.as_deref(), Kind::Mcp);
        cx.spawn_in(window, async move |this, cx| {
            let read_path = path.clone();
            let original = cx
                .background_executor()
                .spawn(async move { edit::read(&read_path) })
                .await;
            let _ = this.update_in(cx, |app, window, cx| {
                if !app.writable || app.closing {
                    return;
                }
                let original = match original {
                    Ok(original) => original,
                    Err(error) => {
                        app.extensions_ui.feedback = Some(error.to_string());
                        cx.notify();
                        return;
                    }
                };
                let app = cx.entity();
                let form = cx.new(|cx| McpForm {
                    app,
                    engine,
                    path: path.clone(),
                    original,
                    fields: [
                        cx.new(|cx| InputState::new(window, cx).placeholder("my-server")),
                        cx.new(|cx| InputState::new(window, cx)),
                    ],
                    arguments: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 4)),
                    remote: false,
                    error: None,
                });
                window.open_dialog(cx, move |dialog, window, _| {
                    let form = form.clone();
                    dialog
                        .title("添加 MCP 服务")
                        .margin_top(px(32.))
                        .w(px(
                            (f32::from(window.viewport_size().width) - 60.).clamp(0., 620.)
                        ))
                        .overlay_closable(false)
                        .content(move |body, _, _| body.child(form.clone()))
                });
            });
        })
        .detach();
    }

    pub(super) fn open_native_config(
        &mut self,
        path: PathBuf,
        kind: Kind,
        new_skill: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let engine = self.extensions_ui.engine;
        cx.spawn_in(window, async move |this, cx| {
            let read_path = path.clone();
            let original = cx
                .background_executor()
                .spawn(async move {
                    if new_skill {
                        Ok(None)
                    } else {
                        edit::read(&read_path)
                    }
                })
                .await;
            let _ = this.update_in(cx, |app, window, cx| {
                if !app.writable || app.closing {
                    return;
                }
                let original = match original {
                    Ok(original) => original,
                    Err(error) => {
                        app.extensions_ui.feedback = Some(error.to_string());
                        cx.notify();
                        return;
                    }
                };
                let text = original
                    .clone()
                    .unwrap_or_else(|| edit::template(engine, kind).into());
                show_editor(
                    cx.entity(),
                    path.clone(),
                    original,
                    text,
                    new_skill,
                    window,
                    cx,
                );
            });
        })
        .detach();
    }
}
