use super::*;
use catalog::{
    PreparedSkill,
    mcp::{self, OptionSpec},
    receipts,
};
use std::collections::BTreeMap;

pub(super) enum Prepared {
    Skill(Box<PreparedSkill>),
    Mcp(Option<String>, bool),
}
pub(super) struct Installation {
    pub engine: Provider,
    pub workspace: Option<PathBuf>,
    pub destination: PathBuf,
    pub item: Item,
    pub prepared: Prepared,
}
struct InstallDialog {
    app: Entity<ConduitApp>,
    engine: Provider,
    workspace: Option<PathBuf>,
    destination: PathBuf,
    item: Item,
    prepared: Prepared,
    options: Vec<OptionSpec>,
    selected: usize,
    name: Entity<InputState>,
    fields: Vec<Entity<InputState>>,
    preview: Option<String>,
    saving: bool,
    cancel: Arc<AtomicBool>,
    error: Option<String>,
    discard: bool,
}
pub(super) fn show(
    app: Entity<ConduitApp>,
    installation: Installation,
    window: &mut Window,
    cx: &mut App,
) {
    let Installation {
        engine,
        workspace,
        destination,
        item,
        prepared,
    } = installation;
    let options = mcp::options(&item);
    let dialog = cx.new(|cx| {
        let fields = option_fields(options.first(), window, cx);
        InstallDialog {
            app,
            engine,
            workspace,
            destination,
            name: cx.new(|cx| InputState::new(window, cx).default_value(mcp::alias(&item))),
            item,
            prepared,
            options,
            selected: 0,
            fields,
            preview: None,
            saving: false,
            cancel: Arc::new(AtomicBool::new(false)),
            error: None,
            discard: false,
        }
    });
    window.open_dialog(cx, move |builder, window, cx| {
        let body = dialog.clone();
        let dismiss = dialog.clone();
        let saving = dialog.read(cx).saving;
        builder
            .title(text("install_details"))
            .w(px(
                (f32::from(window.viewport_size().width) - 48.).clamp(0., 760.)
            ))
            .margin_top(px(24.))
            .overlay_closable(false)
            .keyboard(!saving)
            .close_button(!saving)
            .on_cancel(move |_, _, cx| dismiss.update(cx, |dialog, cx| dialog.can_close(cx)))
            .content(move |container, _, _| container.child(body.clone()))
    });
}
fn option_fields(
    option: Option<&OptionSpec>,
    window: &mut Window,
    cx: &mut Context<InstallDialog>,
) -> Vec<Entity<InputState>> {
    option
        .map(|option| {
            option
                .fields
                .iter()
                .map(|field| {
                    cx.new(|cx| {
                        InputState::new(window, cx)
                            .default_value(field.default.clone())
                            .masked(field.secret)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
impl InstallDialog {
    fn can_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.saving {
            return false;
        }
        let dirty = matches!(self.prepared, Prepared::Mcp(..))
            && (self.preview.is_some()
                || self.name.read(cx).value().as_str() != mcp::alias(&self.item)
                || self.options.get(self.selected).is_some_and(|option| {
                    option
                        .fields
                        .iter()
                        .zip(&self.fields)
                        .any(|(field, input)| input.read(cx).value().as_str() != field.default)
                }));
        if dirty && !self.discard {
            self.discard = true;
            cx.notify();
            return false;
        }
        true
    }
    fn spec(&self, cx: &App) -> anyhow::Result<mcp::InstallSpec> {
        let option = self
            .options
            .get(self.selected)
            .ok_or_else(|| anyhow::anyhow!(text("unsupported")))?;
        anyhow::ensure!(
            option.remote || !matches!(self.prepared, Prepared::Mcp(_, false)),
            text("node_missing")
        );
        let values: BTreeMap<_, _> = option
            .fields
            .iter()
            .zip(&self.fields)
            .map(|(field, input)| (field.key.clone(), input.read(cx).value().to_string()))
            .collect();
        option.resolve(&values)
    }
    fn preview(&mut self, cx: &mut Context<Self>) {
        let result = self.spec(cx).and_then(|spec| {
            // Preview only the added block; mask all credential-bearing values, including static ones.
            let mut safe = spec;
            for value in safe.values.values_mut() {
                *value = "••••••".into();
            }
            if self.options[self.selected]
                .fields
                .iter()
                .any(|field| field.secret)
            {
                if safe.remote {
                    safe.target = "https://redacted.invalid/mcp".into();
                } else {
                    safe.args = vec![text("arguments_hidden")];
                }
            }
            mcp::preview(self.engine, None, self.name.read(cx).value().trim(), &safe)
        });
        match result {
            Ok(preview) => {
                self.preview = Some(preview);
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving
            || !self.app.read(cx).writable
            || self.app.read(cx).closing
            || matches!(self.engine, Provider::Qoder | Provider::Gemini)
        {
            return;
        }
        let item = self.item.clone();
        let destination = self.destination.clone();
        let cancel = self.cancel.clone();
        let root = receipts::root();
        let task = match &self.prepared {
            Prepared::Skill(skill) => Save::Skill(skill.clone()),
            Prepared::Mcp(original, _) => {
                if self.preview.is_none() {
                    self.preview(cx);
                    return;
                }
                let name = self.name.read(cx).value().trim().to_owned();
                let result = self
                    .spec(cx)
                    .and_then(|spec| mcp::preview(self.engine, original.as_deref(), &name, &spec));
                match result {
                    Ok(preview) => Save::Mcp {
                        original: original.clone(),
                        preview,
                        name,
                    },
                    Err(error) => {
                        self.error = Some(error.to_string());
                        cx.notify();
                        return;
                    }
                }
            }
        };
        self.saving = true;
        self.error = None;
        self.discard = false;
        self.cancel.store(false, Ordering::Release);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match task {
                        Save::Skill(skill) => catalog::install_skill(
                            &catalog::Client::new(&cancel),
                            &skill,
                            &destination,
                            &root,
                        )
                        .map(|_| ()),
                        Save::Mcp {
                            original,
                            preview,
                            name,
                        } => receipts::save_mcp(
                            &item,
                            &destination,
                            &name,
                            original.as_deref(),
                            &preview,
                            &root,
                        ),
                    }
                })
                .await;
            let _ = this.update_in(cx, |dialog, window, cx| {
                dialog.saving = false;
                match result {
                    Ok(()) => {
                        let index = usize::from(matches!(dialog.prepared, Prepared::Mcp(..)));
                        let message = format!(
                            "{} · {} · {}",
                            text(if index == 0 {
                                "skill_installed"
                            } else {
                                "mcp_added"
                            }),
                            dialog.engine.label(),
                            display_path(&dialog.destination)
                        );
                        dialog.app.update(cx, |app, cx| {
                            app.extensions_ui.feedback = Some(message.clone());
                            let state = &mut app.extensions_ui.discovery[index];
                            if state.target.as_ref()
                                != Some(&(dialog.engine, dialog.workspace.clone()))
                            {
                                state.installed.clear();
                            }
                            state.installed.insert(dialog.item.id.clone());
                            state.target = Some((dialog.engine, dialog.workspace.clone()));
                            app.extensions_ui.invalidate();
                            app.ensure_extensions(window, cx, true);
                            app.notify_user(
                                Notice::new(
                                    NoticeSource::Settings,
                                    NoticeLevel::Success,
                                    text("install_result"),
                                    message,
                                )
                                .target(NoticeTarget::Settings),
                                cx,
                            );
                            cx.notify();
                        });
                        window.close_dialog(cx);
                    }
                    Err(error) => dialog.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
enum Save {
    Skill(Box<PreparedSkill>),
    Mcp {
        original: Option<String>,
        preview: String,
        name: String,
    },
}
impl Render for InstallDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let writable = self.app.read(cx).writable
            && !matches!(self.engine, Provider::Qoder | Provider::Gemini)
            && !self.app.read(cx).closing;
        let mut content = v_flex()
            .text_size(ui_text_size(14.))
            .gap_2()
            .id("extension-install-content")
            .max_h((window.viewport_size().height - px(230.)).max(px(100.)))
            .overflow_y_scroll()
            .child(div().font_semibold().child(self.item.name.clone()))
            .child(note(self.item.description.clone()))
            .child(note(format!(
                "{} · {}",
                self.engine.label(),
                display_path(&self.destination)
            )))
            .when(!writable, |body| body.child(note(text("read_only"))));
        match &self.prepared {
            Prepared::Skill(skill) => {
                content = content
                    .child(note(format!(
                        "{} · {} · {} {} · {} KiB",
                        skill.repository,
                        skill.commit,
                        skill.files.len(),
                        text("files"),
                        skill.bytes / 1024
                    )))
                    .child(note(format!("{} / {}", skill.directory, skill.name)))
                    .child(note(text("skill_notice")))
                    .child(
                        div()
                            .text_size(ui_text_size(14.))
                            .child(skill.markdown.chars().take(32_000).collect::<String>()),
                    )
                    .when(skill.markdown.chars().count() > 32_000, |body| {
                        body.child(note(text("preview_truncated")))
                    })
                    .child(note(
                        skill
                            .files
                            .iter()
                            .take(100)
                            .map(|f| f.path.as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ));
            }
            Prepared::Mcp(..) => {
                content = content.child(note(text("mcp_notice")));
                if self.options.is_empty() {
                    content = content.child(note(text("unsupported")));
                } else if let Some(preview) = &self.preview {
                    content = content
                        .child(note(text("preview_notice")))
                        .child(div().text_size(ui_text_size(12.)).child(preview.clone()));
                } else {
                    let mut options = h_flex().gap_2().flex_wrap();
                    for (index, option) in self.options.iter().enumerate() {
                        options = options.child(
                            resource_button(("catalog-mcp-option", index))
                                .small()
                                .outline()
                                .label(option.label.clone())
                                .selected(self.selected == index)
                                .disabled(self.saving)
                                .on_click(cx.listener(move |dialog, _, window, cx| {
                                    dialog.selected = index;
                                    dialog.fields =
                                        option_fields(dialog.options.get(index), window, cx);
                                    dialog.error = None;
                                    dialog.discard = false;
                                    cx.notify();
                                })),
                        );
                    }
                    content = content.child(options).child(
                        settings_input(&self.name)
                            .disabled(self.saving)
                            .aria_label(text("service_name")),
                    );
                    for (field, input) in
                        self.options[self.selected].fields.iter().zip(&self.fields)
                    {
                        content = content.child(
                            v_flex()
                                .gap_1()
                                .child(note(format!(
                                    "{}{}",
                                    field.label,
                                    if field.required { " *" } else { "" }
                                )))
                                .child(
                                    settings_input(input)
                                        .disabled(self.saving)
                                        .aria_label(field.label.clone()),
                                )
                                .when(!field.choices.is_empty(), |body| {
                                    body.child(note(format!(
                                        "{}: {}",
                                        text("choices"),
                                        field.choices.join(" / ")
                                    )))
                                })
                                .child(note(field.description.clone())),
                        );
                    }
                    if !self.options[self.selected].remote {
                        content = content.child(note(text("node_required")));
                    }
                }
            }
        }
        let can_install = matches!(self.prepared, Prepared::Skill(_)) || !self.options.is_empty();
        v_flex()
            .gap_3()
            .child(content)
            .when_some(self.error.clone(), |body, error| {
                body.child(div().text_color(rgb(palette().danger_text)).child(error))
            })
            .when(self.discard, |body| body.child(note(text("discard_hint"))))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .flex_wrap()
                    .when(self.preview.is_some(), |row| {
                        row.child(
                            resource_button("catalog-back")
                                .label(text("back"))
                                .disabled(self.saving)
                                .on_click(cx.listener(|dialog, _, _, cx| {
                                    dialog.preview = None;
                                    dialog.discard = false;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        resource_button("catalog-install-cancel")
                            .label(text(if self.discard { "discard" } else { "cancel" }))
                            .disabled(self.saving && matches!(self.prepared, Prepared::Mcp(..)))
                            .on_click(cx.listener(|dialog, _, window, cx| {
                                if dialog.saving {
                                    dialog.cancel.store(true, Ordering::Release);
                                } else if dialog.can_close(cx) {
                                    window.close_dialog(cx);
                                }
                            })),
                    )
                    .child(
                        resource_button("catalog-install-confirm")
                            .debug_selector(|| "catalog-install-confirm".into())
                            .primary()
                            .label(text(if self.saving {
                                "installing"
                            } else if matches!(self.prepared, Prepared::Skill(_)) {
                                "install"
                            } else if self.preview.is_some() {
                                "save_config"
                            } else {
                                "preview"
                            }))
                            .disabled(!writable || self.saving || !can_install)
                            .on_click(cx.listener(|dialog, _, window, cx| dialog.save(window, cx))),
                    ),
            )
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{
        ConduitApp, InputState, InstallDialog, Item, Payload, Prepared, Provider, mcp,
        option_fields,
    };
    use gpui_kit::{AppContext as _, TestAppContext};
    use std::{
        path::PathBuf,
        sync::{Arc, atomic::AtomicBool},
    };

    #[gpui_kit::test]
    fn mcp_preview_masks_credentials_and_requires_explicit_discard(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        cx.add_window_view(|window, cx| {
            let app = cx
                .new(|cx| ConduitApp::with_store(window, cx, Ok(Default::default()), false, false));
            let item = Item {
                id: "mcp:io.example/test".into(),
                name: "io.example/test".into(),
                description: String::new(),
                source_url: "https://example.com".into(),
                payload: Payload::Mcp(serde_json::json!({"remotes":[{
                    "type":"streamable-http", "url":"https://example.com/mcp",
                    "headers":[{"name":"Authorization","isRequired":true,"isSecret":true}]
                }]})),
            };
            let options = mcp::options(&item);
            let dialog = cx.new(|cx| InstallDialog {
                app: app.clone(),
                engine: Provider::Codex,
                workspace: None,
                destination: PathBuf::from("unused-test-config.toml"),
                name: cx.new(|cx| InputState::new(window, cx).default_value(mcp::alias(&item))),
                item,
                prepared: Prepared::Mcp(None, true),
                fields: option_fields(options.first(), window, cx),
                options,
                selected: 0,
                preview: None,
                saving: false,
                cancel: Arc::new(AtomicBool::new(false)),
                error: None,
                discard: false,
            });
            dialog.update(cx, |dialog, cx| {
                assert!(dialog.can_close(cx));
                dialog.preview(cx);
                assert!(dialog.error.is_some());
                assert!(dialog.preview.is_none());
                dialog.fields[0].update(cx, |input, cx| {
                    input.set_value("test-secret-123", window, cx)
                });
                dialog.preview(cx);
                let preview = dialog.preview.as_ref().unwrap();
                assert!(!preview.contains("test-secret-123"));
                assert!(preview.contains("••••••"));
                assert!(dialog.error.is_none());
                assert_eq!(
                    dialog.spec(cx).unwrap().values["Authorization"],
                    "test-secret-123"
                );
                assert!(!dialog.can_close(cx));
                assert!(dialog.discard);
                assert!(dialog.can_close(cx));
                dialog.saving = true;
                assert!(!dialog.can_close(cx));
            });
            gpui_kit::component::Root::new(app, window, cx)
        });
    }
}
