//! Native extension mutations are explicit, reversible and scoped to a source.
use super::*;
use crate::extensions::{self, Kind, edit};

impl ConduitApp {
    pub(super) fn open_extension_toggle(
        &mut self,
        entry: extensions::Entry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || entry.managed() || self.extensions_ui.changing {
            return;
        }
        let engine = self.extensions_ui.engine;
        let context = self.extensions_ui.context.clone();
        if entry.kind == Kind::Mcp && !engine.descriptor().capabilities.mcp_toggle {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let path = entry.path.clone();
            let result = cx.background_executor().spawn(async move {
                edit::read(&path)?.ok_or_else(|| anyhow::anyhow!(conduit::i18n::text("settings.extension_management.source_file_no_longer_exists_refresh_and")))
            }).await;
            let _ = this.update_in(cx, |app, window, cx| {
                if !app.writable || app.closing || app.extensions_ui.context != context { return; }
                let original = match result { Ok(text) => text, Err(error) => {
                    app.extensions_ui.feedback = Some(error.to_string()); cx.notify(); return;
                }};
                if entry.kind == Kind::Mcp {
                    match extensions::mcp::set_enabled_preview(engine, &original, &entry.name, entry.disabled) {
                        Ok(text) => super::config_editor::show_editor(cx.entity(), entry.path.clone(), Some(original), text, false, window, cx),
                        Err(error) => { app.extensions_ui.feedback = Some(error.to_string()); cx.notify(); }
                    }
                    return;
                }
                let view = cx.entity();
                let path = entry.path.clone();
                let title = if entry.disabled { conduit::i18n::text_static("settings.extension_management.restore_skill") } else { conduit::i18n::text_static("settings.extension_management.disable_skill") };
                window.open_dialog(cx, move |dialog, window, _| {
                    let view = view.clone(); let path = path.clone(); let original = original.clone();
                    dialog.title(title).margin_top(px(40.)).overlay_closable(false)
                        .w((window.viewport_size().width - px(60.)).clamp(px(0.), px(620.)))
                        .content(move |body, _, _| {
                            let confirm = view.clone(); let path = path.clone(); let original = original.clone();
                            body.child(v_flex().gap_3().child(conduit::i18n::format("settings.extensions.source", &[("arg0", format!("{}", path.display()))]))
                                .child(conduit::i18n::text_static("settings.extension_management.disable_or_restore_by_renaming_skill_md"))
                                .child(h_flex().gap_2().justify_end()
                                    .child(Button::new("skill-toggle-cancel").ghost().label(conduit::i18n::text_static("git_branches.cancel")).on_click(|_, window, cx| window.close_dialog(cx)))
                                    .child(Button::new("skill-toggle-confirm").primary().label(title).on_click(move |_, window, cx| {
                                        confirm.update(cx, |app, cx| app.toggle_skill_file(path.clone(), original.clone(), window, cx));
                                        window.close_dialog(cx);
                                    }))))
                        })
                });
            });
        }).detach();
    }

    fn toggle_skill_file(
        &mut self,
        path: PathBuf,
        original: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.extensions_ui.changing {
            return;
        }
        self.extensions_ui.changing = true;
        self.extensions_ui.feedback = Some(
            conduit::i18n::text_static("settings.extension_management.updating_skill_status")
                .into(),
        );
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { extensions::management::toggle_skill(&path, &original) })
                .await;
            let _ = this.update_in(cx, |app, window, cx| {
                app.extensions_ui.changing = false;
                let success = result.is_ok();
                let message = match result {
                    Ok(path) => conduit::i18n::format(
                        "settings.extension_management.skill_status_updated_applies_when_the_next",
                        &[("arg0", format!("{}", path.display()))],
                    ),
                    Err(error) => conduit::i18n::format(
                        "settings.extension_management.update_failed",
                        &[("error", format!("{}", error))],
                    ),
                };
                app.extensions_ui.feedback = Some(message.clone());
                app.notify_user(
                    Notice::new(
                        NoticeSource::Settings,
                        if success {
                            NoticeLevel::Success
                        } else {
                            NoticeLevel::Error
                        },
                        conduit::i18n::text_static(
                            "settings.extension_management.skill_status_update",
                        ),
                        message,
                    )
                    .target(NoticeTarget::Settings)
                    .presentation(NoticePresentation::History),
                    cx,
                );
                app.extensions_ui.invalidate();
                if !app.closing && app.settings_ui.open {
                    app.ensure_extensions(window, cx, true);
                }
                cx.notify();
            });
        })
        .detach();
    }
}
