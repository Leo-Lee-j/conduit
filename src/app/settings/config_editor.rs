//! Native configuration editing with explicit save and conflict protection.
use super::*;
use crate::extensions::{Kind, edit};

struct ConfigEditor {
    app: Entity<ConduitApp>,
    path: PathBuf,
    original: Option<String>,
    initial_text: String,
    confirm_discard: bool,
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
    fn config_editor_requires_explicit_discard_after_edits_and_keeps_failures(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        cx.add_window_view(|window, cx| {
            let app =
                cx.new(|cx| ConduitApp::with_store(window, cx, Ok(Store::default()), false, false));
            let editor = cx.new(|cx| super::ConfigEditor {
                app: app.clone(),
                path: Store::path().with_file_name("hfe-config.json"),
                original: None,
                initial_text: "{}".into(),
                confirm_discard: false,
                text: cx.new(|cx| TextareaState::new(window, cx).default_value("{}")),
                name: None,
                error: None,
                saving: false,
            });
            editor.update(cx, |editor, cx| {
                assert!(editor.can_close(cx));
                editor
                    .text
                    .update(cx, |text, cx| text.set_value("{ invalid", window, cx));
                assert!(!editor.can_close(cx));
                assert!(editor.confirm_discard);
                editor.confirm_discard = false;
                editor.save(window, cx);
                assert!(editor.error.is_some());
                assert_eq!(editor.text.read(cx).value().as_str(), "{ invalid");
                assert!(!editor.can_close(cx));
                editor
                    .text
                    .update(cx, |text, cx| text.set_value("{}", window, cx));
                assert!(editor.can_close(cx));
                editor.saving = true;
                assert!(!editor.can_close(cx));
            });
            gpui_kit::component::Root::new(app, window, cx)
        });
    }

    #[gpui_kit::test]
    fn config_and_mcp_forms_keep_actions_visible_on_short_windows(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
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
        let form = v_flex()
            .id("mcp-form")
            .debug_selector(|| "mcp-form".into())
            .gap_3()
            .max_h((window.viewport_size().height - px(210.)).max(px(160.)))
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(format!(
                        "{} · {}",
                        self.engine.label(),
                        display_path(&self.path)
                    )),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(conduit::i18n::text_static(
                        "settings.config_editor.service_name",
                    ))
                    .child(
                        Input::new(&self.fields[0]).aria_label(conduit::i18n::text_static(
                            "settings.config_editor.mcp_service_name",
                        )),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("mcp-local")
                            .small()
                            .outline()
                            .label(conduit::i18n::text_static(
                                "settings.config_editor.local_command",
                            ))
                            .selected(!self.remote)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.remote = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("mcp-remote")
                            .small()
                            .outline()
                            .label(conduit::i18n::text_static(
                                "settings.config_editor.remote_http",
                            ))
                            .selected(self.remote)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.remote = true;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(if self.remote {
                        conduit::i18n::text_static("settings.config_editor.service_url")
                    } else {
                        conduit::i18n::text_static(
                            "settings.config_editor.executable_command_e_g_npx_uvx_or",
                        )
                    })
                    .child(
                        Input::new(&self.fields[1]).aria_label(conduit::i18n::text_static(
                            "settings.config_editor.mcp_command_or_url",
                        )),
                    ),
            )
            .when(!self.remote, |body| {
                body.child(
                    v_flex()
                        .gap_1()
                        .child(conduit::i18n::text_static(
                            "settings.config_editor.arguments_one_per_line_no_extra_quotes",
                        ))
                        .child(Textarea::new(&self.arguments).h(px(90.)).aria_label(
                            conduit::i18n::text_static(
                                "settings.config_editor.mcp_command_arguments",
                            ),
                        )),
                )
            })
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static(
                        "settings.config_editor.preview_the_merged_configuration_next_you_can",
                    )),
            )
            .when_some(self.error.clone(), |body, error| {
                body.child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().danger_text))
                        .child(error),
                )
            });
        v_flex().gap_3().child(form).child(
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("mcp-form-cancel")
                        .ghost()
                        .label(conduit::i18n::text_static("git_branches.cancel"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .child(
                    Button::new("mcp-form-preview")
                        .debug_selector(|| "mcp-form-preview".into())
                        .primary()
                        .label(conduit::i18n::text_static(
                            "settings.config_editor.preview_configuration",
                        ))
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

pub(super) fn show_editor(
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
        initial_text: text.clone(),
        confirm_discard: false,
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
        let dismiss = editor.clone();
        let saving = editor.read(cx).saving;
        dialog
            .margin_top(px(32.))
            .w(px(
                (f32::from(window.viewport_size().width) - 60.).clamp(0., 820.)
            ))
            .title(if new_skill {
                conduit::i18n::text_static("settings.config_editor.create_skill")
            } else {
                conduit::i18n::text_static("settings.config_editor.edit_native_configuration")
            })
            .overlay_closable(false)
            .keyboard(!saving)
            .close_button(!saving)
            .on_cancel(move |_, _, cx| dismiss.update(cx, |editor, cx| editor.can_close(cx)))
            .content(move |body, _, _| body.child(editor.clone()))
    });
}

impl ConfigEditor {
    fn can_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.saving {
            return false;
        }
        let dirty = self.text.read(cx).value().as_str() != self.initial_text
            || self
                .name
                .as_ref()
                .is_some_and(|name| name.read(cx).value().as_str() != "my-skill");
        if dirty {
            self.confirm_discard = true;
            cx.notify();
        }
        !dirty
    }

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
                                    conduit::i18n::text_static("settings.config_editor.configuration_saved_original_backed_up_loaded_when")
                                } else {
                                    conduit::i18n::text_static("settings.config_editor.configuration_created_loaded_when_the_next_session")
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
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(display_path(&self.path)),
            )
            .when_some(self.name.clone(), |body, name| {
                body.child(
                    v_flex()
                        .gap_1()
                        .child(conduit::i18n::text_static(
                            "settings.config_editor.skill_folder_name",
                        ))
                        .child(Input::new(&name).disabled(self.saving).aria_label(
                            conduit::i18n::text_static("settings.config_editor.skill_folder_name"),
                        )),
                )
            })
            .child(
                div().h(px(height)).flex_shrink_0().w_full().child(
                    Textarea::new(&self.text)
                        .disabled(self.saving)
                        .h_full()
                        .aria_label(conduit::i18n::text_static(
                            "settings.config_editor.configuration_contents",
                        )),
                ),
            )
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static(
                        "settings.config_editor.validate_the_format_and_back_up_the",
                    )),
            )
            .when_some(self.error.clone(), |body, error| {
                body.child(
                    div()
                        .text_size(ui_text_size(12.))
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
                        .label(if self.confirm_discard {
                            conduit::i18n::text_static("settings.config_editor.continue_editing")
                        } else {
                            conduit::i18n::text_static("git_branches.cancel")
                        })
                        .ghost()
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.confirm_discard {
                                this.confirm_discard = false;
                                this.text.read(cx).focus_handle(cx).focus(window, cx);
                                cx.notify();
                            } else if this.can_close(cx) {
                                window.close_dialog(cx);
                            }
                        })),
                )
                .when(self.confirm_discard, |row| {
                    row.child(
                        Button::new("native-config-discard")
                            .label(conduit::i18n::text_static(
                                "settings.config_editor.discard_unsaved_changes",
                            ))
                            .ghost()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                })
                .child(
                    Button::new("native-config-save")
                        .debug_selector(|| "native-config-save".into())
                        .label(if self.saving {
                            conduit::i18n::text_static("settings.config_editor.saving")
                        } else {
                            conduit::i18n::text_static("settings.config_editor.save_configuration")
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
                        .title(conduit::i18n::text_static(
                            "settings.config_editor.add_mcp_service",
                        ))
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
