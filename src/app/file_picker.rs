//! Double-Shift navigation and an asynchronous, filename-only workspace picker.
use super::*;
use crate::{
    file_search::{self, FileIndex},
    workspace_files::FileKey,
};

const TAP_INTERVAL: Duration = Duration::from_millis(500);
const PICKER_LIMIT: usize = 50;

actions!(
    conduit_file_search,
    [
        PreviousFileResult,
        NextFileResult,
        OpenFileResult,
        DismissFileSearch
    ]
);

#[derive(Default)]
pub(super) struct DoubleShift {
    modifiers: Modifiers,
    pressed: Option<Instant>,
    released: Option<Instant>,
}

impl DoubleShift {
    pub(super) fn cancel(&mut self) {
        self.pressed = None;
        self.released = None;
    }

    pub(super) fn update(&mut self, modifiers: Modifiers, now: Instant) -> bool {
        if modifiers == self.modifiers {
            return false;
        }
        let previous = std::mem::replace(&mut self.modifiers, modifiers);
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        if modifiers == shift && previous == Modifiers::default() {
            self.pressed = Some(now);
        } else if modifiers == Modifiers::default() && previous == shift {
            let pressed = self.pressed.take();
            if pressed.is_some_and(|time| now.duration_since(time) <= TAP_INTERVAL) {
                if self
                    .released
                    .take()
                    .is_some_and(|time| now.duration_since(time) <= TAP_INTERVAL)
                {
                    return true;
                }
                self.released = Some(now);
            } else {
                self.cancel();
            }
        } else {
            // Ctrl/Alt/Win/Fn combinations are not standalone Shift taps.
            self.cancel();
        }
        false
    }
}

struct FilePicker {
    app: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    input: Entity<InputState>,
    index: Option<Arc<FileIndex>>,
    results: Vec<String>,
    selected: usize,
    sequence: u64,
    searching: bool,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl FilePicker {
    fn new(
        app: WeakEntity<ConduitApp>,
        workspace: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new("up", PreviousFileResult, Some("ConduitFileSearch > Input")),
            KeyBinding::new("up", PreviousFileResult, Some("ConduitFileSearch")),
            KeyBinding::new("down", NextFileResult, Some("ConduitFileSearch > Input")),
            KeyBinding::new("down", NextFileResult, Some("ConduitFileSearch")),
            KeyBinding::new("enter", OpenFileResult, Some("ConduitFileSearch > Input")),
            KeyBinding::new("enter", OpenFileResult, Some("ConduitFileSearch")),
            KeyBinding::new(
                "escape",
                DismissFileSearch,
                Some("ConduitFileSearch > Input"),
            ),
            KeyBinding::new("escape", DismissFileSearch, Some("ConduitFileSearch")),
        ]);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("搜索文件名或路径…"));
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh(cx);
            }
        });
        let root = workspace.clone();
        cx.spawn(async move |this, cx| {
            let index = cx
                .background_executor()
                .spawn(async move { file_search::discover(&root) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.index = Some(Arc::new(index));
                this.refresh(cx);
            });
        })
        .detach();
        Self {
            app,
            workspace,
            input,
            index: None,
            results: Vec::new(),
            selected: 0,
            sequence: 0,
            searching: true,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.sequence += 1;
        self.results.clear();
        self.selected = 0;
        self.searching = true;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
        let Some(index) = self.index.clone() else {
            return;
        };
        let query = self.input.read(cx).value().to_string();
        let sequence = self.sequence;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(60))
                .await;
            if !this
                .update(cx, |this, _| this.sequence == sequence)
                .unwrap_or(false)
            {
                return;
            }
            let results = cx
                .background_executor()
                .spawn(async move { file_search::search_with_limit(&index, &query, PICKER_LIMIT) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.sequence == sequence {
                    this.results = results;
                    this.searching = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn open(&mut self, position: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(relative) = self.results.get(position).cloned() else {
            return;
        };
        let key = FileKey {
            workspace: self.workspace.clone(),
            relative: relative.into(),
        };
        window.close_dialog(cx);
        let _ = self
            .app
            .update(cx, |app, cx| app.open_workspace_file(key, true, window, cx));
    }

    fn handle_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        match key {
            "up" | "down" => {
                if !self.results.is_empty() {
                    let len = self.results.len();
                    self.selected = if key == "up" {
                        (self.selected + len - 1) % len
                    } else {
                        (self.selected + 1) % len
                    };
                    self.scroll.scroll_to_item(self.selected);
                    cx.notify();
                }
            }
            "enter" => self.open(self.selected, window, cx),
            "escape" => window.close_dialog(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for FilePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list_height = (f32::from(window.viewport_size().height) - 270.).clamp(100., 360.);
        let mut body = v_flex()
            .id("file-search-picker")
            .w_full()
            .gap_2()
            .key_context("ConduitFileSearch")
            .on_action(cx.listener(|this, _: &PreviousFileResult, window, cx| {
                this.handle_key("up", window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextFileResult, window, cx| {
                this.handle_key("down", window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenFileResult, window, cx| {
                this.handle_key("enter", window, cx)
            }))
            .on_action(cx.listener(|this, _: &DismissFileSearch, window, cx| {
                this.handle_key("escape", window, cx)
            }))
            .child(
                div()
                    .debug_selector(|| "file-search-input".into())
                    .child(Input::new(&self.input).h(px(34.)).focus_bordered(false)),
            )
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().faint))
                    .truncate()
                    .child(display_path(&self.workspace)),
            );
        if self.searching {
            body = body.child(div().py_2().child(if self.index.is_none() {
                "正在扫描工作区文件…"
            } else {
                "正在搜索…"
            }));
        } else if self.results.is_empty() {
            let note = self
                .index
                .as_ref()
                .and_then(|index| index.error.clone())
                .unwrap_or_else(|| "没有匹配的文件".into());
            body = body.child(div().py_2().child(note));
        } else {
            let mut list = v_flex()
                .id("file-search-results")
                .w_full()
                .max_h(px(list_height))
                .overflow_y_scroll()
                .track_scroll(&self.scroll);
            for (position, path) in self.results.iter().enumerate() {
                let name = path.rsplit('/').next().unwrap_or(path).to_owned();
                list = list.child(
                    v_flex()
                        .id(("file-search-result", position))
                        .w_full()
                        .px_2()
                        .py_2()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .when(position == self.selected, |row| {
                            row.bg(rgb(palette().surface_hover))
                        })
                        .hover(|row| row.bg(rgb(palette().surface_hover)))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.open(position, window, cx)),
                        )
                        .child(div().text_size(ui_text_size(13.)).truncate().child(name))
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().muted))
                                .truncate()
                                .child(path.clone()),
                        ),
                );
            }
            body = body.child(list);
        }
        if self.index.as_ref().is_some_and(|index| index.incomplete) {
            body = body.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().faint))
                    .child("工作区较大或部分目录无法读取，仅显示已扫描的文件"),
            );
        }
        body.child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(rgb(palette().faint))
                .child("↑ / ↓ 选择 · Enter 打开 · Esc 关闭"),
        )
    }
}

impl ConduitApp {
    pub(super) fn open_file_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || window.has_active_dialog(cx) || self.composer_is_composing(window, cx) {
            return;
        }
        let Some(workspace) = self
            .selected_workspace
            .and_then(|index| self.store.workspaces.get(index))
            .map(|w| w.path.clone())
        else {
            self.message = Some("请先选择工作区，再搜索文件".into());
            cx.notify();
            return;
        };
        let app = cx.weak_entity();
        let picker = cx.new(|cx| FilePicker::new(app, workspace, window, cx));
        let initial_focus = std::cell::Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                picker
                    .read(cx)
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }
            let width = (f32::from(window.viewport_size().width) - 48.).clamp(240., 640.);
            dialog.title("搜索文件").w(px(width)).child(picker.clone())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{DoubleShift, Duration, Instant, Modifiers};
    fn shift() -> Modifiers {
        Modifiers {
            shift: true,
            ..Modifiers::default()
        }
    }
    fn tap(state: &mut DoubleShift, time: Instant) -> bool {
        assert!(!state.update(shift(), time));
        state.update(Modifiers::default(), time + Duration::from_millis(40))
    }

    #[test]
    fn double_shift_requires_two_complete_taps() {
        let now = Instant::now();
        let mut state = DoubleShift::default();
        assert!(!tap(&mut state, now));
        assert!(tap(&mut state, now + Duration::from_millis(150)));
        assert!(
            !tap(&mut state, now + Duration::from_millis(300)),
            "do not fire again on the third tap"
        );
    }

    #[test]
    fn double_shift_ignores_repeats_holds_and_slow_taps() {
        let now = Instant::now();
        let mut state = DoubleShift::default();
        assert!(!state.update(shift(), now));
        assert!(!state.update(shift(), now + Duration::from_millis(50)));
        assert!(!state.update(Modifiers::default(), now + Duration::from_millis(600)));
        assert!(!tap(&mut state, now + Duration::from_millis(700)));
        assert!(!tap(&mut state, now + Duration::from_millis(1400)));
        assert!(tap(&mut state, now + Duration::from_millis(1550)));
    }

    #[test]
    fn double_shift_cancels_on_typing_and_other_modifiers() {
        let now = Instant::now();
        let mut state = DoubleShift::default();
        assert!(!tap(&mut state, now));
        state.cancel();
        assert!(!tap(&mut state, now + Duration::from_millis(100)));
        assert!(!state.update(shift(), now + Duration::from_millis(200)));
        state.cancel(); // Shift+letter or Shift+Enter.
        assert!(!state.update(Modifiers::default(), now + Duration::from_millis(240)));
        assert!(!tap(&mut state, now + Duration::from_millis(300)));
        assert!(!state.update(
            Modifiers {
                control: true,
                ..shift()
            },
            now + Duration::from_millis(350)
        ));
        assert!(!state.update(shift(), now + Duration::from_millis(360)));
        assert!(!state.update(Modifiers::default(), now + Duration::from_millis(400)));
        assert!(!tap(&mut state, now + Duration::from_millis(450)));
    }
}
