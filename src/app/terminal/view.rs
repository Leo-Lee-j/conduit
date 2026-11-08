use super::super::*;
use crate::terminal::{Runtime, Snapshot, encode_key};
use alacritty_terminal::{
    selection::SelectionType,
    term::{TermMode, cell::Flags},
    vte::ansi::{Color, CursorShape},
};
use std::ops::Range;

pub(super) struct TerminalView {
    pub runtime: Arc<Runtime>,
    pub input_enabled: bool,
    pub focus: FocusHandle,
    snapshot: Option<Arc<Snapshot>>,
    marked: String,
    marked_selection: Range<usize>,
    blur_subscription: Option<Subscription>,
    bounds: Option<Bounds<Pixels>>,
    cursor: Point<Pixels>,
    cell_width: f32,
    line_height: f32,
    font_size: f32,
    family: SharedString,
    metrics: Option<(SharedString, f32)>,
    colors: (u32, u32, u32),
    selecting: bool,
    error: Option<String>,
    pending_paste: Option<String>,
}
impl TerminalView {
    pub fn new(runtime: Arc<Runtime>, cx: &mut Context<Self>) -> Self {
        Self {
            runtime,
            input_enabled: true,
            focus: cx.focus_handle(),
            snapshot: None,
            marked: String::new(),
            marked_selection: 0..0,
            blur_subscription: None,
            bounds: None,
            cursor: point(px(0.), px(0.)),
            cell_width: 8.4,
            line_height: 20.,
            font_size: 14.,
            family: "Consolas".into(),
            metrics: None,
            colors: (palette().text, palette().canvas, palette().surface_strong),
            selecting: false,
            error: None,
            pending_paste: None,
        }
    }
    pub fn configure(&mut self, family: SharedString, size: f32, cx: &mut Context<Self>) {
        let colors = (palette().text, palette().canvas, palette().surface_strong);
        if colors != self.colors {
            self.colors = colors;
            cx.notify();
        }
        if self.family != family || self.font_size != size {
            self.family = family;
            self.font_size = size;
            self.line_height = (size * 1.45).ceil();
            self.metrics = None;
            cx.notify();
        }
    }
    pub fn blur(&mut self) {
        self.marked.clear();
        self.marked_selection = 0..0;
        self.selecting = false;
        self.pending_paste = None;
    }
    fn mode(&self) -> TermMode {
        self.snapshot.as_ref().map_or(TermMode::empty(), |s| s.mode)
    }
    fn send(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.error = if self.input_enabled {
            self.runtime.send(bytes).err()
        } else {
            Some(
                conduit::i18n::text_static(
                    "terminal.view.the_current_terminal_cannot_accept_input",
                )
                .into(),
            )
        };
        cx.notify();
    }
    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            if text.len() > 1024 * 1024 - 12 {
                self.error = Some(
                    conduit::i18n::text_static("terminal.view.a_single_paste_cannot_exceed_1_mib")
                        .into(),
                );
            } else {
                let text = text
                    .replace('\0', "")
                    .replace("\r\n", "\n")
                    .replace('\r', "\n");
                if text.contains('\n') {
                    self.pending_paste = Some(text);
                } else {
                    self.send_paste(text, cx);
                }
            }
            cx.notify();
        }
    }
    fn send_paste(&mut self, text: String, cx: &mut Context<Self>) {
        let text = text.replace('\x1b', "");
        let bytes = if self.mode().contains(TermMode::BRACKETED_PASTE) {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text.replace('\n', "\r")
        };
        self.send(bytes.into_bytes(), cx);
    }
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        if k.modifiers.control && k.modifiers.shift && k.key == "c" {
            if let Some(text) = self.runtime.selected_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            cx.stop_propagation();
            return;
        }
        if k.modifiers.control && k.modifiers.shift && k.key == "v"
            || k.modifiers.shift && k.key == "insert"
        {
            self.paste(cx);
            cx.stop_propagation();
            return;
        }
        if !self.marked.is_empty() || event.prefer_character_input {
            return;
        }
        if let Some(bytes) = encode_key(
            &k.key,
            k.modifiers.control,
            k.modifiers.alt,
            k.modifiers.shift,
            self.mode().contains(TermMode::APP_CURSOR),
        ) {
            self.send(bytes, cx);
            cx.stop_propagation();
        }
    }
    fn position(&self, p: Point<Pixels>) -> Option<(usize, usize, bool)> {
        let bounds = self.bounds?;
        let (cols, rows) = self.runtime.size();
        let x = (f32::from(p.x - bounds.left()) / self.cell_width).max(0.);
        let y = (f32::from(p.y - bounds.top()) / self.line_height).max(0.);
        Some((
            (y as usize).min(rows.saturating_sub(1) as usize),
            (x as usize).min(cols.saturating_sub(1) as usize),
            x.fract() > 0.5,
        ))
    }
    fn mouse_report(
        &mut self,
        p: Point<Pixels>,
        button: u8,
        release: bool,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) -> bool {
        if modifiers.shift
            || !self.mode().intersects(
                TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION,
            )
        {
            return false;
        }
        let Some((row, col, _)) = self.position(p) else {
            return false;
        };
        let button =
            button + if modifiers.alt { 8 } else { 0 } + if modifiers.control { 16 } else { 0 };
        if self.mode().contains(TermMode::SGR_MOUSE) {
            self.send(
                format!(
                    "\x1b[<{button};{};{}{}",
                    col + 1,
                    row + 1,
                    if release { 'm' } else { 'M' }
                )
                .into_bytes(),
                cx,
            );
        } else if col < 223 && row < 223 {
            self.send(
                vec![
                    27,
                    b'[',
                    b'M',
                    32 + if release { 3 } else { button },
                    33 + col as u8,
                    33 + row as u8,
                ],
                cx,
            );
        }
        true
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let total = self.marked.encode_utf16().count();
        let start = range.start.min(total);
        let end = range.end.min(total).max(start);
        let (start_byte, actual_start) = utf16_boundary(&self.marked, start, false);
        let (end_byte, actual_end) = utf16_boundary(&self.marked, end, true);
        *actual = Some(actual_start..actual_end);
        Some(self.marked[start_byte..end_byte].to_string())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.marked_selection.clone(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then(|| 0..self.marked.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        self.marked_selection = 0..0;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked.clear();
        self.marked_selection = 0..0;
        self.send(text.as_bytes().to_vec(), cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = text.into();
        let len = text.encode_utf16().count();
        self.marked_selection = selected
            .map(|r| r.start.min(len)..r.end.min(len))
            .unwrap_or(len..len);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(Bounds::new(
            self.cursor,
            size(px(self.cell_width), px(self.line_height)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

// Expand partial surrogate-pair ranges to complete Unicode scalar values.
fn utf16_boundary(text: &str, target: usize, round_up: bool) -> (usize, usize) {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        if units == target {
            return (byte, units);
        }
        let next = units + character.len_utf16();
        if target < next {
            return if round_up {
                (byte + character.len_utf8(), next)
            } else {
                (byte, units)
            };
        }
        units = next;
    }
    (text.len(), units)
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.blur_subscription.is_none() {
            self.blur_subscription = Some(cx.on_blur(&self.focus, window, |this, _, cx| {
                this.marked.clear();
                this.marked_selection = 0..0;
                this.selecting = false;
                cx.notify();
            }));
        }
        if let Some(snapshot) = self.runtime.snapshot() {
            self.snapshot = Some(Arc::new(snapshot));
        } else {
            self.runtime.dirty.store(true, Ordering::Release);
        }
        let entity = cx.entity();
        let prepaint = entity.clone();
        let paint = entity.clone();
        let grid =
            div()
                .id("terminal-grid")
                .debug_selector(|| "terminal-grid".into())
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .key_context("ConduitTerminal")
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::key_down))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        this.focus.focus(window, cx);
                        if !this.mouse_report(event.position, 0, false, event.modifiers, cx) {
                            this.selecting = true;
                            if let Some((row, col, right)) = this.position(event.position) {
                                let kind = match event.click_count {
                                    2 => SelectionType::Semantic,
                                    3 => SelectionType::Lines,
                                    _ => SelectionType::Simple,
                                };
                                this.runtime.select(row, col, right, Some(kind));
                                cx.notify();
                            }
                        }
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        if this.selecting {
                            if let Some((r, c, side)) = this.position(event.position) {
                                this.runtime.select(r, c, side, None);
                                cx.notify();
                            }
                        } else if this
                            .mode()
                            .intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
                        {
                            this.mouse_report(event.position, 32, false, event.modifiers, cx);
                        }
                    } else {
                        this.selecting = false;
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseUpEvent, _, cx| {
                        if !this.selecting {
                            this.mouse_report(event.position, 0, true, event.modifiers, cx);
                        }
                        this.selecting = false;
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.selecting = false),
                )
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                    let lines = (f32::from(event.delta.pixel_delta(px(this.line_height)).y)
                        / this.line_height)
                        .round() as i32;
                    if lines != 0 {
                        if this.mode().intersects(
                            TermMode::MOUSE_REPORT_CLICK
                                | TermMode::MOUSE_DRAG
                                | TermMode::MOUSE_MOTION,
                        ) && !event.modifiers.shift
                        {
                            for _ in 0..lines.unsigned_abs().min(10) {
                                this.mouse_report(
                                    event.position,
                                    if lines > 0 { 64 } else { 65 },
                                    false,
                                    event.modifiers,
                                    cx,
                                );
                            }
                        } else {
                            this.runtime.scroll(lines);
                            cx.notify();
                        }
                    }
                    cx.stop_propagation();
                }))
                .child(
                    canvas(
                        move |bounds, window, cx| {
                            prepaint.update(cx, |view, _| {
                                let key = (view.family.clone(), view.font_size);
                                if view.metrics.as_ref() != Some(&key) {
                                    let run = TextRun {
                                        len: 1,
                                        font: font(view.family.clone()),
                                        color: rgb(palette().text).into(),
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    };
                                    view.cell_width = f32::from(
                                        window
                                            .text_system()
                                            .shape_line(
                                                "M".into(),
                                                px(view.font_size),
                                                &[run],
                                                None,
                                            )
                                            .width,
                                    )
                                    .max(1.);
                                    view.metrics = Some(key);
                                }
                                view.bounds = Some(bounds);
                                view.runtime.display(
                                    palette().text,
                                    palette().canvas,
                                    view.cell_width.ceil() as u16,
                                    view.line_height.ceil() as u16,
                                );
                                view.runtime.resize(
                                    (f32::from(bounds.size.width) / view.cell_width)
                                        .floor()
                                        .clamp(2., 500.) as u16,
                                    (f32::from(bounds.size.height) / view.line_height)
                                        .floor()
                                        .clamp(1., 300.) as u16,
                                );
                            });
                        },
                        move |bounds, _, window, cx| {
                            let (
                                focus,
                                snapshot,
                                cell_width,
                                line_height,
                                font_size,
                                family,
                                marked,
                            ) = {
                                let view = paint.read(cx);
                                (
                                    view.focus.clone(),
                                    view.snapshot.clone(),
                                    view.cell_width,
                                    view.line_height,
                                    view.font_size,
                                    view.family.clone(),
                                    view.marked.clone(),
                                )
                            };
                            let mut cursor = bounds.origin;
                            if let Some(screen) = &snapshot {
                                let dark = palette().canvas < 0x808080;
                                let color = |value: Color| {
                                    let c = match value {
                                        Color::Named(
                                            alacritty_terminal::vte::ansi::NamedColor::Foreground,
                                        ) if screen.colors[256].is_none() => {
                                            return rgb(palette().text);
                                        }
                                        Color::Named(
                                            alacritty_terminal::vte::ansi::NamedColor::Background,
                                        ) if screen.colors[257].is_none() => {
                                            return rgb(palette().canvas);
                                        }
                                        Color::Spec(c) => c,
                                        Color::Indexed(i) => screen.colors[i as usize]
                                            .unwrap_or_else(|| {
                                                crate::terminal::palette(i as usize, dark)
                                            }),
                                        Color::Named(i) => screen.colors[i].unwrap_or_else(|| {
                                            crate::terminal::palette(i as usize, dark)
                                        }),
                                    };
                                    rgb((c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32)
                                };
                                let mut index = 0;
                                while index < screen.cells.len() {
                                    let (row, col, cell, selected) = &screen.cells[index];
                                    if *row < 0 || *row >= screen.size.1 as i32 {
                                        index += 1;
                                        continue;
                                    }
                                    let origin = point(
                                        bounds.left() + px(*col as f32 * cell_width),
                                        bounds.top() + px(*row as f32 * line_height),
                                    );
                                    let (mut fg, mut bg) = (color(cell.fg), color(cell.bg));
                                    if cell.flags.contains(Flags::INVERSE) {
                                        std::mem::swap(&mut fg, &mut bg);
                                    }
                                    if *selected {
                                        bg = rgb(palette().surface_strong);
                                        fg = rgb(palette().text);
                                    }
                                    if bg != rgb(palette().canvas) {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                origin,
                                                size(px(cell_width), px(line_height)),
                                            ),
                                            bg,
                                        ));
                                    }
                                    if cell
                                        .flags
                                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::HIDDEN)
                                    {
                                        index += 1;
                                        continue;
                                    }
                                    let mut text = cell.c.to_string();
                                    if let Some(extra) = cell.zerowidth() {
                                        text.extend(extra);
                                    }
                                    let mut consumed = 1;
                                    // Shape contiguous plain ASCII in runs. Unicode keeps explicit cell coordinates.
                                    if cell.c.is_ascii() && cell.zerowidth().is_none() {
                                        while let Some((next_row, next_col, next, next_selected)) =
                                            screen.cells.get(index + consumed)
                                        {
                                            if next_row != row
                                                || *next_col != *col + consumed
                                                || next.fg != cell.fg
                                                || next.bg != cell.bg
                                                || next.flags != cell.flags
                                                || next_selected != selected
                                                || !next.c.is_ascii()
                                                || next.zerowidth().is_some()
                                            {
                                                break;
                                            }
                                            text.push(next.c);
                                            consumed += 1;
                                        }
                                        if bg != rgb(palette().canvas) {
                                            window.paint_quad(fill(
                                                Bounds::new(
                                                    origin,
                                                    size(
                                                        px(cell_width * consumed as f32),
                                                        px(line_height),
                                                    ),
                                                ),
                                                bg,
                                            ));
                                        }
                                    }
                                    if !text.trim().is_empty() {
                                        let mut f = font(family.clone());
                                        if cell.flags.contains(Flags::BOLD) {
                                            f.weight = FontWeight::BOLD;
                                        }
                                        if cell.flags.contains(Flags::ITALIC) {
                                            f.style = FontStyle::Italic;
                                        }
                                        let run = TextRun {
                                            len: text.len(),
                                            font: f,
                                            color: fg.into(),
                                            background_color: None,
                                            underline: cell
                                                .flags
                                                .intersects(Flags::ALL_UNDERLINES)
                                                .then_some(UnderlineStyle {
                                                    thickness: px(1.),
                                                    color: None,
                                                    wavy: false,
                                                }),
                                            strikethrough: None,
                                        };
                                        let line = window.text_system().shape_line(
                                            text.into(),
                                            px(font_size),
                                            &[run],
                                            None,
                                        );
                                        let _ = line.paint(
                                            origin,
                                            px(line_height),
                                            TextAlign::Left,
                                            None,
                                            window,
                                            cx,
                                        );
                                    }
                                    index += consumed;
                                }
                                if screen.offset == 0 {
                                    cursor = point(
                                        bounds.left() + px(screen.cursor.1 as f32 * cell_width),
                                        bounds.top() + px(screen.cursor.0 as f32 * line_height),
                                    );
                                    if screen.cursor.2 != CursorShape::Hidden
                                        && focus.is_focused(window)
                                    {
                                        window.paint_quad(fill(
                                            Bounds::new(cursor, size(px(2.), px(line_height))),
                                            rgb(palette().text),
                                        ));
                                    }
                                }
                                if !marked.is_empty() {
                                    let run = TextRun {
                                        len: marked.len(),
                                        font: font(family.clone()),
                                        color: rgb(palette().text).into(),
                                        background_color: Some(
                                            rgb(palette().surface_strong).into(),
                                        ),
                                        underline: Some(UnderlineStyle {
                                            thickness: px(1.),
                                            color: None,
                                            wavy: false,
                                        }),
                                        strikethrough: None,
                                    };
                                    let line = window.text_system().shape_line(
                                        marked.clone().into(),
                                        px(font_size),
                                        &[run],
                                        None,
                                    );
                                    let _ = line.paint(
                                        cursor,
                                        px(line_height),
                                        TextAlign::Left,
                                        None,
                                        window,
                                        cx,
                                    );
                                }
                            }
                            window.handle_input(
                                &focus,
                                ElementInputHandler::new(bounds, paint.clone()),
                                cx,
                            );
                            paint.update(cx, |view, _| view.cursor = cursor);
                        },
                    )
                    .size_full(),
                );
        v_flex()
            .size_full()
            .min_h_0()
            .bg(rgb(palette().canvas))
            .when_some(self.error.clone(), |pane, error| {
                pane.child(
                    div()
                        .px_2()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().danger_text))
                        .child(error),
                )
            })
            .when_some(
                self.pending_paste.as_ref().map(|s| s.lines().count()),
                |pane, lines| {
                    pane.child(
                        h_flex()
                            .px_2()
                            .gap_2()
                            .items_center()
                            .child(conduit::i18n::format(
                                "terminal.view.paste_lines_this_may_execute_multiple_commands",
                                &[("lines", format!("{}", lines))],
                            ))
                            .child(
                                Button::new("terminal-confirm-paste")
                                    .small()
                                    .label(conduit::i18n::text_static("terminal.view.paste"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(text) = this.pending_paste.take() {
                                            this.send_paste(text, cx);
                                        }
                                        this.focus.focus(window, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("terminal-cancel-paste")
                                    .small()
                                    .ghost()
                                    .label(conduit::i18n::text_static("git_branches.cancel"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pending_paste = None;
                                        this.focus.focus(window, cx);
                                        cx.notify();
                                    })),
                            ),
                    )
                },
            )
            .when_some(
                self.pending_paste
                    .as_ref()
                    .map(|s| s.chars().take(240).collect::<String>()),
                |pane, preview| {
                    pane.child(
                        div()
                            .max_h(px(64.))
                            .overflow_hidden()
                            .px_2()
                            .text_size(ui_text_size(11.))
                            .font_family(self.family.clone())
                            .child(preview),
                    )
                },
            )
            .child(grid)
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use gpui_kit::{ClipboardItem, EntityInputHandler, TestAppContext};
    use std::time::{Duration, Instant};

    #[gpui_kit::test]
    fn ime_preedit_paste_confirmation_and_readonly_input(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, root) = crate::app::queue::tests::fixture(cx, "terminal-ime");
        let (runtime, view) = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.new_terminal(crate::terminal::ShellProfile::Cmd, window, cx);
                let session = &app.terminals.sessions[0];
                (session.runtime.clone(), session.view.clone())
            })
        });
        let start = Instant::now();
        while runtime.state() != crate::terminal::RunState::Running {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.replace_and_mark_text_in_range(None, "中文🙂", Some(2..4), window, cx);
                assert_eq!(view.marked_text_range(window, cx), Some(0..4));
                assert_eq!(
                    view.selected_text_range(false, window, cx).unwrap().range,
                    2..4
                );
                let mut actual = None;
                for range in [2..4, 3..4, 2..3] {
                    assert_eq!(
                        view.text_for_range(range, &mut actual, window, cx),
                        Some("🙂".into())
                    );
                    assert_eq!(actual, Some(2..4));
                }
                assert_eq!(
                    view.text_for_range(1..2, &mut actual, window, cx),
                    Some("文".into())
                );
                assert_eq!(actual, Some(1..2));
                assert_eq!(
                    view.text_for_range(99..usize::MAX, &mut actual, window, cx),
                    Some(String::new())
                );
                assert_eq!(actual, Some(4..4));
                assert!(!runtime.snapshot().unwrap().text().contains("中文"));
                view.blur();
                assert!(view.marked.is_empty());
                view.replace_text_in_range(None, "echo ime-ok>输入验证.txt\r", window, cx);
                cx.write_to_clipboard(ClipboardItem::new_string("echo first\necho second".into()));
                view.paste(cx);
                assert!(view.pending_paste.is_some());
                view.blur();
                assert!(view.pending_paste.is_none());
                view.input_enabled = false;
                view.replace_text_in_range(None, "echo forbidden>readonly.txt\r", window, cx);
                assert!(view.error.is_some());
            })
        });
        let start = Instant::now();
        while !root.join("输入验证.txt").is_file() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "{}",
                runtime.snapshot().unwrap().text()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!root.join("readonly.txt").exists());
        runtime.stop();
        let start = Instant::now();
        while runtime.active() {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
