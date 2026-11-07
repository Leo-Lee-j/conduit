//! Caret-local file picker. Discovery and ranking stay off the UI thread.

use super::*;
use crate::file_search::{self, FileIndex, MentionQuery};

#[derive(Default)]
pub(super) struct FileMentions {
    active: Option<(ComposerTarget, MentionQuery)>,
    dismissed: Option<(ComposerTarget, MentionQuery)>,
    index: Option<(PathBuf, Instant, Arc<FileIndex>)>,
    indexing: Option<PathBuf>,
    sequence: u64,
    results: Vec<String>,
    selected: usize,
    searching: bool,
}

impl ConduitApp {
    fn mention_query(&self, cx: &App) -> Option<(ComposerTarget, MentionQuery)> {
        if self.closing || !self.writable {
            return None;
        }
        let input = self.input.read(cx);
        Some((
            self.composer_target()?,
            file_search::query_at(&input.value(), input.selected_range())?,
        ))
    }

    pub(super) fn refresh_file_mentions(&mut self, cx: &mut Context<Self>) {
        let query = self.mention_query(cx);
        if query != self.file_mentions.dismissed {
            self.file_mentions.dismissed = None;
        }
        if query == self.file_mentions.active {
            return;
        }
        if query.is_none() {
            self.file_mentions.dismissed = None;
        }
        if query.is_some() && query == self.file_mentions.dismissed {
            return;
        }
        self.file_mentions.active = query;
        self.file_mentions.results.clear();
        self.file_mentions.selected = 0;
        self.file_mentions.sequence += 1;
        self.file_mentions.searching = false;
        cx.notify();
        let Some((target, _)) = self.file_mentions.active.clone() else {
            return;
        };
        let cached = self
            .file_mentions
            .index
            .as_ref()
            .is_some_and(|(path, time, _)| {
                path == &target.workspace && time.elapsed() < Duration::from_secs(30)
            });
        if cached {
            self.search_file_mentions(cx);
        } else if self.file_mentions.indexing.as_ref() != Some(&target.workspace) {
            self.file_mentions.indexing = Some(target.workspace.clone());
            cx.spawn(async move |this, cx| {
                let path = target.workspace.clone();
                let index = cx
                    .background_executor()
                    .spawn(async move { file_search::discover(&path) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    // A later workspace's load owns the cache now.
                    if this.file_mentions.indexing.as_ref() != Some(&target.workspace) {
                        return;
                    }
                    this.file_mentions.indexing = None;
                    this.file_mentions.index =
                        Some((target.workspace, Instant::now(), Arc::new(index)));
                    this.search_file_mentions(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn search_file_mentions(&mut self, cx: &mut Context<Self>) {
        let Some((target, query)) = self.file_mentions.active.clone() else {
            return;
        };
        let Some((path, _, index)) = &self.file_mentions.index else {
            return;
        };
        if path != &target.workspace {
            return;
        }
        let index = index.clone();
        let sequence = self.file_mentions.sequence;
        self.file_mentions.searching = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(60))
                .await;
            // Ignore superseded queries before doing any ranking work.
            let current = this
                .update(cx, |this, _| this.file_mentions.sequence == sequence)
                .unwrap_or(false);
            if !current {
                return;
            }
            let results = cx
                .background_executor()
                .spawn(async move { file_search::search(&index, &query.text) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.file_mentions.sequence != sequence {
                    return;
                }
                this.file_mentions.results = results;
                this.file_mentions.searching = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn dismiss_file_mentions(&mut self, cx: &mut Context<Self>) {
        self.file_mentions.dismissed = self.file_mentions.active.take();
        self.file_mentions.sequence += 1;
        self.file_mentions.results.clear();
        cx.notify();
    }

    pub(super) fn file_mentions_open(&self, window: &Window, cx: &App) -> bool {
        self.file_mentions.active.is_some()
            && self.file_mentions.active == self.mention_query(cx)
            && self.input.read(cx).focus_handle(cx).is_focused(window)
    }

    pub(super) fn composer_is_composing(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        })
    }

    pub(super) fn move_file_mention(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_is_composing(window, cx) {
            return;
        }
        let count = self.file_mentions.results.len();
        if count > 0 {
            self.file_mentions.selected =
                (self.file_mentions.selected as isize + delta).rem_euclid(count as isize) as usize;
            cx.notify();
        }
    }

    pub(super) fn accept_file_mention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_is_composing(window, cx) {
            return;
        }
        let Some((target, query)) = self.file_mentions.active.clone() else {
            return;
        };
        if Some((target.clone(), query.clone())) != self.mention_query(cx) {
            return;
        }
        let Some(relative) = self
            .file_mentions
            .results
            .get(self.file_mentions.selected)
            .cloned()
        else {
            return;
        };
        let path = target.workspace.join(relative);
        // Reuse the same validation and deduplication as drops, paste and the picker.
        let accepted = self.attach_paths(std::slice::from_ref(&path), window, cx);
        if accepted {
            self.dismiss_file_mentions(cx);
            self.input.update(cx, |input, cx| {
                input.set_selected_range(query.range, cx);
                input.replace("", window, cx);
            });
        }
        cx.notify();
    }

    pub(super) fn render_file_mentions(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let target = &self.file_mentions.active.as_ref().unwrap().0;
        let index = self
            .file_mentions
            .index
            .as_ref()
            .filter(|(path, _, _)| path == &target.workspace)
            .map(|(_, _, index)| index);
        let loading = self.file_mentions.searching
            || self.file_mentions.indexing.as_ref() == Some(&target.workspace);
        let status = if loading {
            "正在搜索工作区文件…".to_string()
        } else if let Some(error) = index.and_then(|index| index.error.clone()) {
            error
        } else if self.file_mentions.results.is_empty() {
            "未找到匹配文件，试试文件名或相对路径".to_string()
        } else {
            "↑↓ 选择 · Enter / Tab 引用 · Esc 关闭".to_string()
        };
        let mut menu = v_flex()
            .id("file-mention-menu")
            .role(Role::ListBox)
            .aria_label("工作区文件候选")
            .debug_selector(|| "file-mention-menu".into())
            .w((window.viewport_size().width - px(32.)).min(px(520.)))
            .min_w_0()
            .p_1()
            .gap_1()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_file_mentions(cx)))
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .child(
                        Icon::new(FullIconName::Files)
                            .small()
                            .text_color(rgb(palette().muted)),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().text))
                            .child("引用工作区文件"),
                    ),
            );
        for (index, path) in self.file_mentions.results.iter().enumerate() {
            let file = Path::new(path);
            let name = file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let parent = file
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|| "工作区根目录".into());
            let selected = index == self.file_mentions.selected;
            let tooltip = path.clone();
            menu = menu.child(
                h_flex()
                    .id(("file-mention", index))
                    .role(Role::ListBoxOption)
                    .aria_label(format!(
                        "引用文件 {path}{}",
                        if selected { "，当前选中" } else { "" }
                    ))
                    .debug_selector(move || format!("file-mention-{index}"))
                    .w_full()
                    .min_w_0()
                    .h(px(38.))
                    .px_2()
                    .gap_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(rgb(palette().surface_hover)))
                    .hover(|row| row.bg(rgb(palette().surface_hover)))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                            .build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.file_mentions.selected = index;
                        this.accept_file_mention(window, cx);
                    }))
                    .child(
                        Icon::new(file_ref::icon_for(file))
                            .small()
                            .text_color(rgb(palette().muted)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().text))
                                    .child(name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(parent),
                            ),
                    )
                    .when(selected, |row| {
                        row.child(
                            Icon::new(FullIconName::CornerDownLeft)
                                .small()
                                .text_color(rgb(palette().muted)),
                        )
                    }),
            );
        }
        menu = menu.child(
            div()
                .px_2()
                .py_1()
                .text_size(ui_text_size(11.))
                .text_color(rgb(palette().muted))
                .child(status),
        );
        if index.is_some_and(|index| index.incomplete) {
            menu = menu.child(
                div()
                    .px_2()
                    .pb_1()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child("扫描范围受限，仅展示已找到的文件；可用附件按钮选择其他文件"),
            );
        }
        // Deferred painting keeps this above the message list without resizing it.
        deferred(
            gpui_kit::base::Positioner::side(
                self.input
                    .read(cx)
                    .cursor_layout()
                    .map(|(mut bounds, _)| {
                        bounds.origin.y += self.input.read(cx).scroll_offset().y;
                        bounds
                    })
                    .unwrap_or_default(),
            )
            .placement(gpui_kit::base::Placement::Top)
            .align(gpui_kit::base::Align::Start)
            .offset(px(8.))
            .margin(px(8.))
            .occlude()
            .child(menu),
        )
        .with_priority(gpui_kit::base::POPUP_PRIORITY)
        .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::ConduitApp;
    use crate::{domain::Workspace, store::Store};
    use gpui_kit::{
        AppContext as _, Entity, EntityInputHandler as _, Focusable as _, Modifiers, MouseButton,
        TestAppContext, VisualTestContext, px, size,
    };
    use std::{
        path::PathBuf,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    fn fixture(cx: &mut TestAppContext) -> (Entity<ConduitApp>, &mut VisualTestContext, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "conduit-mentions-ui-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("docs")).unwrap();
        for file in ["first.rs", "second.rs", "docs/资料 说明.md"] {
            std::fs::write(root.join(file), "text").unwrap();
        }
        cx.update(gpui_kit::init);
        let output = std::rc::Rc::new(std::cell::RefCell::new(None));
        let slot = output.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let store = Store {
                    workspaces: vec![Workspace {
                        path: root.clone(),
                        sessions: vec![],
                    }],
                    ..Default::default()
                };
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.git.workspace = Some(root.clone());
                app
            });
            *slot.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = output.borrow_mut().take().unwrap();
        (app, cx, root)
    }

    fn settle(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.background_executor
            .advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }

    fn draft(app: &Entity<ConduitApp>, text: &str, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            app.update(cx, |this, cx| {
                this.input.update(cx, |input, cx| {
                    input.set_value(text.to_string(), window, cx);
                    input.set_selected_range(text.len()..text.len(), cx);
                });
                this.input.read(cx).focus_handle(cx).focus(window, cx);
            })
        });
        settle(cx);
    }

    #[gpui_kit::test]
    fn file_mentions_keyboard_selection_escape_and_empty_enter(cx: &mut TestAppContext) {
        let (app, cx, root) = fixture(cx);
        draft(&app, "检查 @", cx);
        assert!(cx.debug_bounds("file-mention-menu").is_some());
        cx.simulate_keystrokes("down enter");
        settle(cx);
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(
                app.current_files().unwrap().references[0].mention,
                "second.rs"
            );
            assert_eq!(app.input.read(cx).value().as_ref(), "检查 ");
            assert!(app.session_control.runs.is_empty());
            assert!(
                app.store.workspaces[0].sessions.is_empty(),
                "Enter must select without starting a turn"
            );
        });
        draft(&app, "@fi", cx);
        cx.simulate_keystrokes("escape");
        settle(cx);
        assert!(cx.debug_bounds("file-mention-menu").is_none());
        cx.update(|_, cx| app.update(cx, |this, cx| this.refresh_file_mentions(cx)));
        settle(cx);
        assert!(
            cx.debug_bounds("file-mention-menu").is_none(),
            "Esc must survive blink/repaint"
        );
        cx.simulate_keystrokes("r");
        settle(cx);
        assert!(cx.debug_bounds("file-mention-menu").is_some());
        cx.simulate_keystrokes("tab");
        settle(cx);
        cx.update(|_, cx| assert_eq!(app.read(cx).current_files().unwrap().references.len(), 2));
        draft(&app, "@no-such-file", cx);
        cx.simulate_keystrokes("enter");
        settle(cx);
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(app.input.read(cx).value().as_ref(), "@no-such-file");
            assert!(app.store.workspaces[0].sessions.is_empty());
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui_kit::test]
    fn file_mentions_mouse_preserves_suffix_dedupes_and_rejects_deleted_files(
        cx: &mut TestAppContext,
    ) {
        let (app, cx, root) = fixture(cx);
        draft(&app, "检查 @资料 后面的正文", cx);
        cx.update(|_, cx| {
            app.update(cx, |this, cx| {
                this.input.update(cx, |input, cx| {
                    let end = "检查 @资料".len();
                    input.set_selected_range(end..end, cx);
                })
            })
        });
        settle(cx);
        let row = cx
            .debug_bounds("file-mention-0")
            .expect("matching file")
            .center();
        cx.simulate_mouse_down(row, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(row, MouseButton::Left, Modifiers::default());
        settle(cx);
        cx.update(|window, cx| {
            let app = app.read(cx);
            assert_eq!(app.input.read(cx).value().as_ref(), "检查  后面的正文");
            assert_eq!(
                app.current_files().unwrap().references[0].mention,
                "docs/资料 说明.md"
            );
            assert!(app.input.read(cx).focus_handle(cx).is_focused(window));
        });
        draft(&app, "@资料", cx);
        cx.simulate_keystrokes("tab");
        settle(cx);
        cx.update(|_, cx| assert_eq!(app.read(cx).current_files().unwrap().references.len(), 1));
        draft(&app, "@first", cx);
        std::fs::remove_file(root.join("first.rs")).unwrap();
        cx.simulate_keystrokes("enter");
        settle(cx);
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(app.input.read(cx).value().as_ref(), "@first");
            assert!(
                app.current_files()
                    .unwrap()
                    .feedback
                    .as_ref()
                    .unwrap()
                    .contains("first.rs")
            );
            assert_eq!(app.current_files().unwrap().references.len(), 1);
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[gpui_kit::test]
    fn file_mentions_ignore_ime_and_discard_stale_workspace_results(cx: &mut TestAppContext) {
        let (app, cx, root) = fixture(cx);
        draft(&app, "@fi", cx);
        cx.update(|window, cx| {
            app.update(cx, |this, cx| {
                this.input.update(cx, |input, cx| {
                    input.replace_and_mark_text_in_range(None, "r", Some(1..1), window, cx)
                });
            })
        });
        settle(cx);
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| {
            app.update(cx, |this, cx| {
                assert!(this.composer_is_composing(window, cx));
                assert!(this.current_files().is_none());
                assert!(this.store.workspaces[0].sessions.is_empty());
                this.input
                    .update(cx, |input, cx| input.unmark_text(window, cx));
                // Queue a query and switch workspaces before its results arrive.
                this.store.workspaces.push(Workspace {
                    path: root.join("docs"),
                    sessions: vec![],
                });
                this.git.workspace = Some(root.join("docs"));
                this.switch(1, None, window, cx);
            })
        });
        settle(cx);
        assert!(cx.debug_bounds("file-mention-menu").is_none());
        draft(&app, "@", cx);
        cx.simulate_resize(size(px(850.), px(500.)));
        settle(cx);
        let menu = cx.debug_bounds("file-mention-menu").unwrap();
        assert!(menu.top() >= px(0.) && menu.bottom() <= px(500.));
        assert!(menu.left() >= px(0.) && menu.right() <= px(850.));
        cx.simulate_keystrokes("enter");
        settle(cx);
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(
                app.current_files().unwrap().references[0].mention,
                "资料 说明.md"
            );
            assert_eq!(app.composer_files.len(), 1);
        });
        draft(&app, "somebody@example.com", cx);
        assert!(cx.debug_bounds("file-mention-menu").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
