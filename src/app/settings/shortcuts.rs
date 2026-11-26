//! Binding editor and conflict feedback for the shortcut catalogue.
use super::extension_catalog::resource_button;
use super::*;
use crate::app::shortcuts::{self, SHORTCUTS, Scope, Shortcut};
use gpui_kit::component::list::{List, ListDelegate, ListItem, ListState as ShortcutListState};

struct ShortcutEditor {
    app: Entity<ConduitApp>,
    shortcut: &'static Shortcut,
    keys: Vec<String>,
    recorder: FocusHandle,
    recording: bool,
    first_shift: Option<std::time::Instant>,
    error: Option<String>,
    _subscription: Subscription,
}

impl ShortcutEditor {
    fn error(&self, cx: &App) -> Option<String> {
        self.error.clone().or_else(|| {
            shortcuts::validate(
                self.shortcut,
                &self.keys,
                &self.app.read(cx).store.ui.shortcuts,
            )
            .err()
        })
    }

    fn record(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let key = keystroke.key.as_str();
        if key == "shift" && self.shortcut.id == "search_files" {
            let now = std::time::Instant::now();
            if self
                .first_shift
                .take()
                .is_none_or(|first| now.duration_since(first) > Duration::from_millis(450))
            {
                self.first_shift = Some(now);
                cx.notify();
                return;
            }
            self.keys = vec![shortcuts::DOUBLE_SHIFT.into()];
        } else {
            if matches!(
                key,
                "ctrl"
                    | "control"
                    | "shift"
                    | "alt"
                    | "super"
                    | "cmd"
                    | "fn"
                    | "platform"
                    | "function"
            ) {
                return;
            }
            self.keys = vec![keystroke.unparse()];
        }
        self.first_shift = None;
        self.recording = false;
        self.error = None;
        cx.notify();
    }

    fn save(&mut self, keys: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.app.read(cx).writable || self.app.read(cx).closing {
            return;
        }
        if let Err(error) =
            shortcuts::validate(self.shortcut, &keys, &self.app.read(cx).store.ui.shortcuts)
        {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let shortcut = self.shortcut;
        self.app.update(cx, |app, cx| {
            if keys
                .iter()
                .map(String::as_str)
                .eq(shortcut.defaults.iter().copied())
            {
                app.store.ui.shortcuts.remove(shortcut.id);
            } else {
                app.store.ui.shortcuts.insert(shortcut.id.into(), keys);
            }
            shortcuts::install(&app.store.ui.shortcuts, cx);
            app.double_shift.cancel();
            app.persist();
            cx.notify();
        });
        window.close_dialog(cx);
    }
}
impl Render for ShortcutEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = self.error(cx);
        let disabled = !self.app.read(cx).writable || self.app.read(cx).closing;
        v_flex()
            .w_full()
            .gap_3()
            .child(
                div()
                    .id("shortcut-recorder")
                    .debug_selector(|| "shortcut-recorder".into())
                    .track_focus(&self.recorder)
                    .w_full()
                    .min_h(px(44.))
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(if self.recording {
                        palette().focus
                    } else {
                        palette().border
                    }))
                    .bg(rgb(palette().surface))
                    .cursor_pointer()
                    .text_size(ui_text_size(14.))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(div().text_size(ui_text_size(14.)).child(
                                if self.keys.is_empty() {
                                    conduit::i18n::text_static("settings.shortcuts.unbound")
                                        .to_owned()
                                } else {
                                    self.keys
                                        .iter()
                                        .map(|key| shortcuts::display_key(key))
                                        .collect::<Vec<_>>()
                                        .join(" / ")
                                },
                            ))
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().muted))
                                    .child(if self.recording {
                                        if self.first_shift.is_some() {
                                            conduit::i18n::text_static(
                                                "settings.shortcuts.press_shift_again",
                                            )
                                        } else {
                                            conduit::i18n::text_static(
                                                "settings.shortcuts.press_the_new_shortcut",
                                            )
                                        }
                                    } else {
                                        conduit::i18n::text_static(
                                            "settings.shortcuts.click_here_to_record_again",
                                        )
                                    }),
                            ),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|editor, _, window, cx| {
                            if !editor.app.read(cx).writable || editor.app.read(cx).closing {
                                return;
                            }
                            editor.recording = true;
                            editor.first_shift = None;
                            editor.recorder.focus(window, cx);
                            cx.notify();
                        }),
                    ),
            )
            .when_some(error.clone(), |body, error| {
                body.child(
                    div()
                        .debug_selector(|| "shortcut-conflict".into())
                        .text_size(ui_text_size(14.))
                        .text_color(rgb(palette().danger_text))
                        .child(error),
                )
            })
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        resource_button("shortcut-reset")
                            .debug_selector(|| "shortcut-reset".into())
                            .small()
                            .ghost()
                            .label(conduit::i18n::text_static(
                                "settings.shortcuts.restore_default_for_this_shortcut",
                            ))
                            .disabled(disabled)
                            .on_click(cx.listener(|editor, _, window, cx| {
                                editor.save(
                                    editor
                                        .shortcut
                                        .defaults
                                        .iter()
                                        .map(|key| key.to_string())
                                        .collect(),
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        resource_button("shortcut-unbind")
                            .debug_selector(|| "shortcut-unbind".into())
                            .small()
                            .ghost()
                            .label(conduit::i18n::text_static(
                                "settings.shortcuts.remove_binding",
                            ))
                            .disabled(disabled)
                            .on_click(cx.listener(|editor, _, _, cx| {
                                editor.keys.clear();
                                editor.recording = false;
                                editor.first_shift = None;
                                editor.error = None;
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        resource_button("shortcut-cancel")
                            .small()
                            .outline()
                            .label(conduit::i18n::text_static("git_branches.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        resource_button("shortcut-save")
                            .debug_selector(|| "shortcut-save".into())
                            .small()
                            .primary()
                            .label(conduit::i18n::text_static(
                                "settings.shortcuts.save_binding",
                            ))
                            .disabled(disabled || self.recording || error.is_some())
                            .on_click(cx.listener(|editor, _, window, cx| {
                                editor.save(editor.keys.clone(), window, cx);
                            })),
                    ),
            )
    }
}

impl ConduitApp {
    fn open_shortcut_editor(
        &mut self,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let shortcut = shortcuts::get(id);
        let keys = shortcut.keys(&self.store.ui.shortcuts);
        let app = cx.entity();
        let editor = cx.new(|cx: &mut Context<ShortcutEditor>| {
            let weak = cx.weak_entity();
            // GPUI resolves bindings before element key listeners. Intercepting here
            // lets the recorder capture keys already bound to application actions.
            let subscription = cx.intercept_keystrokes(move |event, window, cx| {
                let _ = weak.update(cx, |editor, cx| {
                    if !editor.recording
                        || !editor.recorder.is_focused(window)
                        || !window.has_active_dialog(cx)
                        || !editor.app.read(cx).writable
                        || editor.app.read(cx).closing
                    {
                        return;
                    }
                    cx.stop_propagation();
                    window.prevent_default();
                    editor.record(&event.keystroke, cx);
                });
            });
            ShortcutEditor {
                app,
                shortcut,
                keys,
                recorder: cx.focus_handle(),
                recording: true,
                first_shift: None,
                error: None,
                _subscription: subscription,
            }
        });
        let focus = editor.read(cx).recorder.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let editor = editor.clone();
            dialog
                .title(conduit::i18n::format(
                    "settings.shortcuts.edit_shortcut",
                    &[("arg0", conduit::i18n::source(shortcut.label).to_string())],
                ))
                .w(px((f32::from(window.viewport_size().width) - 48.).min(520.)))
                .overlay_closable(false)
                .keyboard(false)
                .content(move |body, _, _| body.child(editor.clone()))
        });
        focus.focus(window, cx);
    }

    pub(super) fn settings_shortcuts_page(&self, view: Entity<Self>) -> SettingsContent {
        SettingsContent::new(ShortcutList { view })
    }
}

#[derive(IntoElement)]
struct ShortcutList {
    view: Entity<ConduitApp>,
}

impl RenderOnce for ShortcutList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let list = window.use_keyed_state("settings-shortcut-list", cx, |window, cx| {
            ShortcutListState::new(
                ShortcutRows {
                    view: self.view.downgrade(),
                    shortcuts: [true, false].map(|global| {
                        SHORTCUTS
                            .iter()
                            .filter(|shortcut| (shortcut.scope == Scope::Global) == global)
                            .collect()
                    }),
                },
                window,
                cx,
            )
            .searchable(false)
            .selectable(false)
        });
        List::new(&list)
            .size_full()
            .px(px(settings_content_inset()))
            .py_2()
            .text_size(ui_text_size(14.))
    }
}

struct ShortcutRows {
    view: WeakEntity<ConduitApp>,
    shortcuts: [Vec<&'static Shortcut>; 2],
}

impl ListDelegate for ShortcutRows {
    type Item = ListItem;

    fn sections_count(&self, _: &App) -> usize {
        self.shortcuts.len()
    }

    fn items_count(&self, section: usize, _: &App) -> usize {
        self.shortcuts[section].len()
    }

    fn render_section_header(
        &mut self,
        section: usize,
        window: &mut Window,
        cx: &mut Context<ShortcutListState<Self>>,
    ) -> Option<impl IntoElement> {
        let view = self.view.upgrade()?;
        let app = view.read(cx);
        let disabled = !app.writable || app.closing;
        let narrow = window.viewport_size().width < ui_text_size(900.).to_pixels(window.rem_size());
        Some(
            v_flex()
                .w_full()
                .min_w_0()
                // List measures headers at min-content width. Bound its two
                // rows so Chinese labels cannot inflate the virtual row gap.
                .h(ui_text_size(68.))
                .flex_shrink_0()
                .whitespace_nowrap()
                .child(
                    h_flex()
                        .debug_selector(move || format!("shortcuts-section-{section}"))
                        .w_full()
                        .min_w_0()
                        .min_h(ui_text_size(40.))
                        .px_3()
                        .py_1()
                        .gap_2()
                        .justify_between()
                        .border_b_1()
                        .border_color(settings_group_border())
                        .text_size(ui_text_size(14.))
                        .font_semibold()
                        .child(conduit::i18n::text_static(if section == 0 {
                            "settings.shortcuts.global_shortcuts"
                        } else {
                            "settings.shortcuts.context_shortcuts"
                        }))
                        .when(section == 0, |header| {
                            header.child(
                                resource_button("shortcuts-reset-all")
                                    .debug_selector(|| "shortcuts-reset-all".into())
                                    .small()
                                    .outline()
                                    .flex_shrink_0()
                                    .label(conduit::i18n::text_static(
                                        "settings.shortcuts.restore_all_defaults",
                                    ))
                                    .disabled(disabled)
                                    .on_click(move |_, _, cx| {
                                        view.update(cx, |app, cx| {
                                            if !app.writable || app.closing {
                                                return;
                                            }
                                            app.store.ui.shortcuts.clear();
                                            shortcuts::install(&app.store.ui.shortcuts, cx);
                                            app.double_shift.cancel();
                                            app.persist();
                                            cx.notify();
                                        });
                                    }),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .px_3()
                        .py_1()
                        .gap_2()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(div().flex_1().min_w_0().child(conduit::i18n::text_static(
                            "settings.shortcuts.command_column",
                        )))
                        .child(
                            div()
                                .w(ui_text_size(220.))
                                .max_w(relative(if narrow { 0.50 } else { 0.40 }))
                                .child(conduit::i18n::text_static(
                                    "settings.shortcuts.binding_column",
                                )),
                        )
                        .when(!narrow, |row| {
                            row.child(div().w(ui_text_size(160.)).flex_shrink_0().child(
                                conduit::i18n::text_static("settings.shortcuts.scope_column"),
                            ))
                        }),
                ),
        )
    }

    fn render_item(
        &mut self,
        index: IndexPath,
        window: &mut Window,
        cx: &mut Context<ShortcutListState<Self>>,
    ) -> Option<ListItem> {
        let view = self.view.upgrade()?;
        let app = view.read(cx);
        let shortcut = *self.shortcuts.get(index.section)?.get(index.row)?;
        let value = shortcuts::display(shortcut.id, &app.store.ui.shortcuts);
        let disabled = !app.writable || app.closing;
        let customized = app.store.ui.shortcuts.contains_key(shortcut.id);
        // Keep binding controls content-sized; extra width belongs to command
        // names. At narrow widths scope moves below its command, not offscreen.
        let narrow = window.viewport_size().width < ui_text_size(900.).to_pixels(window.rem_size());
        let scope = format!(
            "{}{}",
            shortcut.scope.label(),
            conduit::i18n::text_static(if customized {
                "settings.shortcuts.customized"
            } else {
                "settings.shortcuts.default"
            }),
        );
        let scope_hint = scope.clone();
        let title = conduit::i18n::source(shortcut.label).to_string();
        let title_hint = title.clone();
        Some(
            ListItem::new(shortcut.id).py_0().child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .min_h(ui_text_size(44.))
                    .py_1()
                    .border_b_1()
                    .border_color(settings_group_border())
                    .gap_2()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "shortcut-command-{}",
                                        shortcut.id
                                    )))
                                    .text_size(ui_text_size(14.))
                                    .truncate()
                                    .tooltip(move |window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(
                                            title_hint.clone(),
                                        )
                                        .build(window, cx)
                                    })
                                    .child(title),
                            )
                            .when(narrow, |column| {
                                column.child(
                                    div()
                                        .text_size(ui_text_size(12.))
                                        .text_color(rgb(palette().muted))
                                        .child(scope.clone()),
                                )
                            }),
                    )
                    .child(
                        resource_button(SharedString::from(format!(
                            "shortcut-edit-{}",
                            shortcut.id
                        )))
                        .debug_selector(move || format!("shortcut-edit-{}", shortcut.id))
                        .small()
                        .outline()
                        .h(ui_text_size(32.))
                        .text_size(ui_text_size(14.))
                        .w(ui_text_size(220.))
                        .max_w(relative(if narrow { 0.50 } else { 0.40 }))
                        .icon(FullIconName::Pencil)
                        .tooltip(value.clone())
                        .child(div().min_w_0().truncate().child(value))
                        .accessibility_label(conduit::i18n::format(
                            "settings.shortcuts.edit_shortcut_bde991",
                            &[("arg0", conduit::i18n::source(shortcut.label).to_string())],
                        ))
                        .disabled(disabled)
                        .on_click(move |_, window, cx| {
                            view.update(cx, |app, cx| {
                                app.open_shortcut_editor(shortcut.id, window, cx)
                            });
                        }),
                    )
                    .when(!narrow, |row| {
                        row.child(
                            div()
                                .id(SharedString::from(format!(
                                    "shortcut-scope-{}",
                                    shortcut.id
                                )))
                                .w(ui_text_size(160.))
                                .flex_shrink_0()
                                .truncate()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().muted))
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(scope_hint.clone())
                                        .build(window, cx)
                                })
                                .child(scope),
                        )
                    }),
            ),
        )
    }

    fn set_selected_index(
        &mut self,
        _: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ShortcutListState<Self>>,
    ) {
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{ConduitApp, Store};
    use gpui_kit::{AppContext as _, Entity, px};
    use gpui_kit::{Modifiers, MouseButton, TestAppContext, VisualTestContext, size};

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }
    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let center = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"))
            .center();
        cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
        draw(cx);
    }
    fn fixture(
        cx: &mut TestAppContext,
        store: Store,
    ) -> (Entity<ConduitApp>, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.startup = None;
                app.connection_settings.syncing = true;
                app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.simulate_resize(size(px(850.), px(700.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_settings(window, cx);
                app.select_settings_page(2, window, cx);
            })
        });
        draw(cx);
        (app, cx)
    }

    #[gpui_kit::test]
    fn settings_shortcuts_editor_blocks_conflicts_records_saves_and_restores(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx) = fixture(cx, Store::default());
        assert!(cx.debug_bounds("shortcuts-section-0").is_some());
        assert!(cx.debug_bounds("shortcuts-reset-all").is_some());
        click(cx, "shortcut-edit-open_settings");
        cx.simulate_keystrokes("ctrl-p");
        draw(cx);
        assert!(cx.debug_bounds("shortcut-conflict").is_some());
        click(cx, "shortcut-save");
        assert!(app.read_with(cx, |app, _| app.store.ui.shortcuts.is_empty()));
        assert!(cx.debug_bounds("shortcut-recorder").is_some());
        click(cx, "shortcut-recorder");
        cx.simulate_keystrokes("ctrl-p");
        draw(cx);
        assert!(cx.debug_bounds("shortcut-conflict").is_some());
        assert!(cx.debug_bounds("shortcut-recorder").is_some());
        click(cx, "shortcut-recorder");
        cx.simulate_keystrokes("ctrl-alt-s");
        draw(cx);
        assert!(cx.debug_bounds("shortcut-conflict").is_none());
        click(cx, "shortcut-save");
        assert_eq!(
            app.read_with(cx, |app, _| app
                .store
                .ui
                .shortcuts
                .get("open_settings")
                .cloned()),
            Some(vec!["ctrl-alt-s".into()])
        );
        assert!(cx.debug_bounds("shortcut-recorder").is_none());
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(!app.read_with(cx, |app, _| app.settings_ui.open));
        cx.simulate_keystrokes("ctrl-,");
        draw(cx);
        assert!(!app.read_with(cx, |app, _| app.settings_ui.open));
        cx.simulate_keystrokes("ctrl-alt-s");
        draw(cx);
        assert!(app.read_with(cx, |app, _| app.settings_ui.open));
        click(cx, "shortcut-edit-open_settings");
        click(cx, "shortcut-reset");
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.store
                    .ui
                    .shortcuts
                    .insert("open_settings".into(), vec!["ctrl-alt-s".into()]);
                cx.notify();
            })
        });
        draw(cx);
        click(cx, "shortcuts-reset-all");
        assert!(app.read_with(cx, |app, _| {
            !app.store.ui.shortcuts.contains_key("open_settings")
        }));
        cx.simulate_keystrokes("escape ctrl-,");
        draw(cx);
        assert!(app.read_with(cx, |app, _| app.settings_ui.open));
    }
}
