//! Branch comparison browser with per-file lazy diff loading.
use super::*;

impl ConduitApp {
    pub(super) fn open_git_comparison(
        &mut self,
        workspace: PathBuf,
        request: git::ComparisonRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_workspace_path().as_ref() != Some(&workspace) {
            self.message = Some("工作区已变化，请重新打开分支菜单".into());
            cx.notify();
            return;
        }
        let browser = cx.new(|cx| {
            let mut browser = ComparisonBrowser {
                workspace,
                request,
                result: None,
                error: None,
                loading: false,
                selected: None,
                diff: None,
                cancelled: Arc::new(AtomicBool::new(false)),
                task: Task::ready(()),
                commits_visible: true,
            };
            browser.reload(window, cx);
            browser
        });
        window.open_dialog(cx, move |dialog, window, _| {
            let viewport = window.viewport_size();
            let width = (f32::from(viewport.width) - 48.).clamp(320., 1360.);
            let height = (f32::from(viewport.height) - 72.).clamp(260., 880.);
            let margin_top = ((f32::from(viewport.height) - height) * 0.5).max(16.);
            let browser = browser.clone();
            dialog
                .entrance(gpui_kit::component::dialog::DialogEntrance::Fade)
                .w(px(width))
                .margin_top(px(margin_top))
                .p_0()
                .gap_0()
                .close_button(false)
                .content(move |content, _, _| {
                    content.child(div().w_full().h(px(height)).child(browser.clone()))
                })
        });
    }
}

struct ComparisonBrowser {
    workspace: PathBuf,
    request: git::ComparisonRequest,
    result: Option<git::Comparison>,
    error: Option<String>,
    loading: bool,
    selected: Option<usize>,
    diff: Option<Entity<GitDiffDialog>>,
    cancelled: Arc<AtomicBool>,
    task: Task<()>,
    commits_visible: bool,
}

impl ComparisonBrowser {
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.cancelled = Arc::new(AtomicBool::new(false));
        self.loading = true;
        self.error = None;
        self.result = None;
        self.diff = None;
        self.selected = None;
        let cancelled = self.cancelled.clone();
        let workspace = self.workspace.clone();
        let request = self.request.clone();
        self.task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { git::load_comparison(&workspace, request, &cancelled) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match result {
                    Ok(result) => {
                        this.result = Some(result);
                        this.select_file(0, window, cx);
                    }
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn select_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(result) = &self.result else {
            return;
        };
        let Some(file) = result.files.get(index).cloned() else {
            return;
        };
        self.selected = Some(index);
        self.diff = Some(cx.new(|cx| {
            GitDiffDialog::new_source(
                self.workspace.clone(),
                file,
                Some(result.request.clone()),
                window,
                cx,
            )
        }));
        cx.notify();
    }
}

impl Drop for ComparisonBrowser {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl Render for ComparisonBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let other = if self.request.kind == git::ComparisonKind::WorkingTree {
            "工作区"
        } else {
            self.request
                .expected
                .current_ref
                .as_deref()
                .unwrap_or("HEAD")
                .trim_start_matches("refs/heads/")
        };
        let header = h_flex()
            .id("comparison-header")
            .debug_selector(|| "comparison-header".into())
            .p_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(rgb(palette().border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .font_semibold()
                    .child(format!("{} → {other}", self.request.selected.name)),
            )
            .child(
                Button::new("comparison-refresh")
                    .icon(IconName::RefreshCw)
                    .tooltip("重新读取比较")
                    .accessibility_label("重新读取比较")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
            )
            .child(
                Button::new("comparison-close")
                    .icon(FullIconName::X)
                    .tooltip("关闭比较")
                    .accessibility_label("关闭比较")
                    .small()
                    .ghost()
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            );
        let mut body = v_flex()
            .id("git-comparison-browser")
            .debug_selector(|| "git-comparison-browser".into())
            .size_full()
            .min_h_0()
            .rounded(px(12.))
            .overflow_hidden()
            .bg(rgb(palette().surface))
            .text_color(rgb(palette().text))
            .child(header);
        if self.loading {
            return body
                .child(div().p_4().child("正在读取分支比较…"))
                .into_any_element();
        }
        if let Some(error) = &self.error {
            return body
                .child(
                    div()
                        .p_4()
                        .text_size(px(13.))
                        .text_color(rgb(palette().danger_text))
                        .child(error.clone()),
                )
                .into_any_element();
        }
        let Some(result) = &self.result else {
            return body.into_any_element();
        };
        let narrow = f32::from(window.viewport_size().width) < 760.;
        let mut left = v_flex().h_full().min_h_0().gap_1().child(
            div()
                .p_2()
                .text_size(px(12.))
                .font_semibold()
                .child(format!("文件差异 · {}", result.files.len())),
        );
        let mut files = v_flex()
            .id("comparison-files")
            .debug_selector(|| "comparison-files".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_1();
        for (index, file) in result.files.iter().enumerate() {
            files = files.child(
                Button::new(("comparison-file", index))
                    .ghost()
                    .small()
                    .w_full()
                    .accessibility_label(format!("查看差异 {}", file.path))
                    .when(self.selected == Some(index), |button| {
                        button.bg(rgb(palette().surface_hover))
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(rgb(git_change_color(file.kind())))
                                    .child(file.kind().label()),
                            )
                            .child(div().flex_1().min_w_0().truncate().child(file.path.clone())),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.select_file(index, window, cx)),
                    ),
            );
        }
        if result.files.is_empty() {
            files = files.child(div().p_3().text_size(px(12.)).child("两个版本没有文件差异"));
        }
        left = left.child(files);
        if self.request.kind == git::ComparisonKind::Branches && narrow {
            let commits = if result.commits.is_empty() {
                "没有独有提交".into()
            } else {
                result.commits.clone()
            };
            left = left.child(
                Popover::new("comparison-commits-popup")
                    .trigger(
                        Button::new("comparison-commits-open")
                            .label("查看独有提交")
                            .small()
                            .ghost(),
                    )
                    .content(move |_, _, _| {
                        div()
                            .id("comparison-commits-popup-body")
                            .w(px(360.))
                            .max_h(px(240.))
                            .overflow_y_scroll()
                            .p_2()
                            .text_size(px(12.))
                            .child(commits.clone())
                    }),
            );
        } else if self.request.kind == git::ComparisonKind::Branches {
            left = left.child(
                Button::new("comparison-commits-toggle")
                    .label(if self.commits_visible {
                        "收起独有提交"
                    } else {
                        "查看独有提交"
                    })
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.commits_visible = !this.commits_visible;
                        cx.notify();
                    })),
            );
            if self.commits_visible {
                left = left.child(
                    div()
                        .id("comparison-commits")
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .p_2()
                        .text_size(px(11.))
                        .child(if result.commits.is_empty() {
                            "没有独有提交".into()
                        } else {
                            result.commits.clone()
                        }),
                );
            }
        }
        let content = if narrow {
            v_flex()
                .flex_1()
                .min_h_0()
                .child(div().h(px(150.)).w_full().child(left))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .children(self.diff.clone()),
                )
                .into_any_element()
        } else {
            h_flex()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .w(px(290.))
                        .h_full()
                        .border_r_1()
                        .border_color(rgb(palette().border))
                        .child(left),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .children(self.diff.clone()),
                )
                .into_any_element()
        };
        body = body.child(content);
        body.child(
            v_flex()
                .id("comparison-notes")
                .debug_selector(|| "comparison-notes".into())
                .p_2()
                .gap_1()
                .text_size(px(11.))
                .text_color(rgb(palette().muted))
                .children(result.notes.iter().map(|note| div().child(note.clone()))),
        )
        .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::ComparisonBrowser;
    use crate::git;
    use gpui_kit::{AppContext, Task, TestAppContext, px, size};
    use std::sync::{Arc, atomic::AtomicBool};

    #[gpui_kit::test]
    fn comparison_browser_keeps_files_diff_and_notes_inside_small_viewport(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let browser = cx.new(|_| {
                let request = git::ComparisonRequest {
                    selected: git::BranchInfo {
                        name: "feature/较长分支名".into(),
                        full_ref: "refs/heads/feature/较长分支名".into(),
                        oid: "0123456789012345678901234567890123456789".into(),
                        remote: false,
                        upstream: None,
                        upstream_remote: None,
                        upstream_ref: None,
                        occupied: None,
                    },
                    expected: git::BranchContext {
                        current_ref: Some("refs/heads/main".into()),
                        head: Some("1234567890123456789012345678901234567890".into()),
                    },
                    kind: git::ComparisonKind::Branches,
                };
                let result = git::Comparison {
                    request: request.clone(),
                    files: (0..80)
                        .map(|i| git::ChangedFile {
                            index: 'M',
                            worktree: ' ',
                            path: format!("src/long_directory/file_{i}.rs"),
                            original: None,
                        })
                        .collect(),
                    commits: (0..200)
                        .map(|i| format!("< abc1234 change {i}\n"))
                        .collect(),
                    notes: vec![
                        "< 仅所选分支拥有；> 仅当前分支拥有。文件差异比较两端最终内容。".into(),
                    ],
                };
                ComparisonBrowser {
                    workspace: std::env::temp_dir(),
                    request,
                    result: Some(result),
                    error: None,
                    loading: false,
                    selected: None,
                    diff: None,
                    cancelled: Arc::new(AtomicBool::new(false)),
                    task: Task::ready(()),
                    commits_visible: true,
                }
            });
            browser.update(cx, |browser, cx| {
                let file = browser.result.as_ref().unwrap().files[0].clone();
                let comparison = browser.request.clone();
                let view = cx.new(|cx| {
                    let mut diff = super::GitDiffDialog::new_source(std::env::temp_dir(), file, Some(comparison), window, cx);
                    diff.cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
                    diff._load_task = Task::ready(());
                    diff.finish_load(Ok(git::FileDiff { patch: "diff --git a/src/test.rs b/src/test.rs\n--- a/src/test.rs\n+++ b/src/test.rs\n@@ -1 +1 @@\n-old\n+new\n".into(), notes: vec![] }), window, cx);
                    diff
                });
                browser.diff = Some(view);
            });
            gpui_kit::component::Root::new(browser, window, cx)
        });
        for (width, height) in [(1200., 800.), (850., 500.), (700., 500.)] {
            cx.simulate_resize(size(px(width), px(height)));
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            let browser = cx.debug_bounds("git-comparison-browser").unwrap();
            let header = cx.debug_bounds("comparison-header").unwrap();
            let files = cx.debug_bounds("comparison-files").unwrap();
            let notes = cx.debug_bounds("comparison-notes").unwrap();
            let diff = cx.debug_bounds("git-diff-dialog").expect("embedded diff");
            assert!(
                files.size.height > px(20.),
                "files retain a usable viewport at {width}x{height}"
            );
            assert!(header.bottom() <= files.top());
            assert!(
                files.bottom() <= notes.top(),
                "notes cannot overlap the file list"
            );
            assert!(notes.bottom() <= browser.bottom());
            assert!(notes.right() <= browser.right());
            assert!(diff.right() <= browser.right());
            assert!(diff.bottom() <= notes.top());
            if width >= 760. {
                assert!(files.right() <= diff.left());
            } else {
                assert!(files.bottom() <= diff.top());
            }
        }
    }
}
