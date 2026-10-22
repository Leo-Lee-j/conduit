use crate::runtime::{Runtime, palette};
use alacritty_terminal::{
    term::{TermMode, cell::Flags},
    vte::ansi::{Color, CursorShape},
};
use gpui_kit::*;
use std::ops::Range;

const FONT_SIZE: f32 = 14.;
const LINE_HEIGHT: f32 = 20.;

pub struct TerminalView {
    pub runtime: Runtime,
    focus: FocusHandle,
    marked: String,
    bounds: Option<Bounds<Pixels>>,
    cursor: Point<Pixels>,
    cell_width: f32,
}
impl TerminalView {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        Self {
            runtime,
            focus: cx.focus_handle(),
            marked: String::new(),
            bounds: None,
            cursor: point(px(0.), px(0.)),
            cell_width: 8.4,
        }
    }
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }
    pub fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = text
                .replace('\0', "")
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            let bytes = if self.runtime.mode().contains(TermMode::BRACKETED_PASTE) {
                // A clipboard cannot terminate its own bracketed paste envelope.
                format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
            } else {
                text.replace('\n', "\r")
            };
            if bytes.len() > 1024 * 1024 {
                return;
            }
            self.runtime.send(bytes.into_bytes());
        }
    }
    pub fn copy_visible(&self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.runtime.text()));
    }
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.prefer_character_input {
            return;
        }
        let stroke = &event.keystroke;
        if stroke.modifiers.control && stroke.modifiers.shift && stroke.key == "c" {
            self.copy_visible(cx);
            cx.stop_propagation();
            return;
        }
        if stroke.modifiers.control && stroke.modifiers.shift && stroke.key == "v"
            || stroke.modifiers.shift && stroke.key == "insert"
        {
            self.paste(cx);
            cx.stop_propagation();
            return;
        }
        if !self.marked.is_empty() {
            return;
        }
        if let Some(bytes) = encode_key(
            &stroke.key,
            stroke.modifiers.control,
            stroke.modifiers.alt,
            stroke.modifiers.shift,
            self.runtime.mode().contains(TermMode::APP_CURSOR),
        ) {
            self.runtime.send(bytes);
            cx.stop_propagation();
        }
    }
}

pub fn encode_key(
    key: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    if let Some(final_byte) = match key {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        _ => None,
    } {
        return Some(
            if modifier > 1 {
                format!("\x1b[1;{modifier}{final_byte}")
            } else {
                format!("\x1b{}{final_byte}", if app_cursor { 'O' } else { '[' })
            }
            .into_bytes(),
        );
    }
    if let Some(code) = match key {
        "insert" => Some(2),
        "delete" => Some(3),
        "pageup" => Some(5),
        "pagedown" => Some(6),
        "f5" => Some(15),
        "f6" => Some(17),
        "f7" => Some(18),
        "f8" => Some(19),
        "f9" => Some(20),
        "f10" => Some(21),
        "f11" => Some(23),
        "f12" => Some(24),
        _ => None,
    } {
        return Some(
            if modifier > 1 {
                format!("\x1b[{code};{modifier}~")
            } else {
                format!("\x1b[{code}~")
            }
            .into_bytes(),
        );
    }
    let mut bytes = match key {
        "enter" => vec![b'\r'],
        "escape" => vec![27],
        "backspace" => vec![if ctrl { 8 } else { 127 }],
        "tab" if shift => b"\x1b[Z".to_vec(),
        "tab" => vec![b'\t'],
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "space" if ctrl => vec![0],
        _ if ctrl && key.len() == 1 => {
            let c = key.as_bytes()[0].to_ascii_uppercase();
            if (b'@'..=b'_').contains(&c) {
                vec![c & 31]
            } else {
                return None;
            }
        }
        _ if alt && key.len() == 1 => key.as_bytes().to_vec(),
        _ => return None,
    };
    if alt {
        bytes.insert(0, 27);
    }
    Some(bytes)
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        *actual = Some(0..self.marked.encode_utf16().count());
        Some(self.marked.clone())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.marked.encode_utf16().count();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then(|| 0..self.marked.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
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
        self.runtime.send(text.as_bytes().to_vec());
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = text.into();
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
            size(px(self.cell_width), px(LINE_HEIGHT)),
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

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let prepaint_entity = entity.clone();
        div()
            .id("terminal")
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x0d1117))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus.focus(window, cx)),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
                this.runtime
                    .scroll((f32::from(delta.y) / LINE_HEIGHT).round() as i32);
                cx.notify();
            }))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let run = TextRun {
                            len: 1,
                            font: font("Consolas"),
                            color: rgb(0xc9d1d9).into(),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let line = window.text_system().shape_line(
                            "M".into(),
                            px(FONT_SIZE),
                            &[run],
                            None,
                        );
                        let cell_width = f32::from(line.width).max(1.);
                        prepaint_entity.update(cx, |view, _| {
                            view.bounds = Some(bounds);
                            view.cell_width = cell_width;
                            let cols = (f32::from(bounds.size.width) / cell_width)
                                .floor()
                                .clamp(2., 1000.) as u16;
                            let rows = (f32::from(bounds.size.height) / LINE_HEIGHT)
                                .floor()
                                .clamp(1., 500.) as u16;
                            if view.runtime.screen.lock().unwrap().size != (cols, rows) {
                                view.runtime.resize(cols, rows);
                            }
                        });
                    },
                    move |bounds, _, window, cx| {
                        let view = entity.read(cx);
                        let focus = view.focus.clone();
                        let cell_width = view.cell_width;
                        let marked = view.marked.clone();
                        let shared_screen = view.runtime.screen.clone();
                        let screen = shared_screen.lock().unwrap();
                        let content = screen.term.renderable_content();
                        let mut cursor_origin = bounds.origin;
                        for cell in content.display_iter {
                            let row = cell.point.line.0 + content.display_offset as i32;
                            if row < 0 {
                                continue;
                            }
                            let origin = point(
                                bounds.left() + px(cell.point.column.0 as f32 * cell_width),
                                bounds.top() + px(row as f32 * LINE_HEIGHT),
                            );
                            let color = |color: Color| {
                                let rgb = match color {
                                    Color::Spec(rgb) => rgb,
                                    Color::Indexed(index) => content.colors[index as usize]
                                        .unwrap_or_else(|| palette(index as usize)),
                                    Color::Named(name) => content.colors[name]
                                        .unwrap_or_else(|| palette(name as usize)),
                                };
                                gpui_kit::rgb(
                                    (rgb.r as u32) << 16 | (rgb.g as u32) << 8 | rgb.b as u32,
                                )
                            };
                            let (mut fg, mut bg) = (color(cell.fg), color(cell.bg));
                            if cell.flags.contains(Flags::INVERSE) {
                                std::mem::swap(&mut fg, &mut bg);
                            }
                            if bg != rgb(0x0d1117) {
                                window.paint_quad(fill(
                                    Bounds::new(origin, size(px(cell_width), px(LINE_HEIGHT))),
                                    bg,
                                ));
                            }
                            if cell
                                .flags
                                .intersects(Flags::WIDE_CHAR_SPACER | Flags::HIDDEN)
                                || cell.c == ' '
                            {
                                continue;
                            }
                            let mut text = cell.c.to_string();
                            if let Some(extra) = cell.zerowidth() {
                                text.extend(extra);
                            }
                            let mut text_font = font("Consolas");
                            if cell.flags.contains(Flags::BOLD) {
                                text_font.weight = FontWeight::BOLD;
                            }
                            if cell.flags.contains(Flags::ITALIC) {
                                text_font.style = FontStyle::Italic;
                            }
                            let run = TextRun {
                                len: text.len(),
                                font: text_font,
                                color: fg.into(),
                                background_color: None,
                                underline: cell.flags.contains(Flags::UNDERLINE).then_some(
                                    UnderlineStyle {
                                        thickness: px(1.),
                                        color: None,
                                        wavy: false,
                                    },
                                ),
                                strikethrough: None,
                            };
                            let line = window.text_system().shape_line(
                                text.into(),
                                px(FONT_SIZE),
                                &[run],
                                None,
                            );
                            let _ = line.paint(
                                origin,
                                px(LINE_HEIGHT),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        if content.display_offset == 0 {
                            cursor_origin = point(
                                bounds.left()
                                    + px(content.cursor.point.column.0 as f32 * cell_width),
                                bounds.top() + px(content.cursor.point.line.0 as f32 * LINE_HEIGHT),
                            );
                            if content.cursor.shape != CursorShape::Hidden
                                && focus.is_focused(window)
                            {
                                window.paint_quad(fill(
                                    Bounds::new(cursor_origin, size(px(2.), px(LINE_HEIGHT))),
                                    rgb(0x79c0ff),
                                ));
                            }
                        }
                        if !marked.is_empty() {
                            let run = TextRun {
                                len: marked.len(),
                                font: font("Consolas"),
                                color: rgb(0xffffff).into(),
                                background_color: Some(rgb(0x30363d).into()),
                                underline: Some(UnderlineStyle {
                                    thickness: px(1.),
                                    color: None,
                                    wavy: false,
                                }),
                                strikethrough: None,
                            };
                            let line = window.text_system().shape_line(
                                marked.into(),
                                px(FONT_SIZE),
                                &[run],
                                None,
                            );
                            let _ = line.paint(
                                cursor_origin,
                                px(LINE_HEIGHT),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        drop(screen);
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        entity.update(cx, |view, _| view.cursor = cursor_origin);
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::std::prelude::v1::test]
    fn control_and_application_cursor_sequences() {
        assert_eq!(encode_key("c", true, false, false, false), Some(vec![3]));
        assert_eq!(
            encode_key("up", false, false, false, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode_key("left", true, false, false, false),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(encode_key("a", false, false, false, false), None);
        assert_eq!(
            encode_key("enter", false, false, false, false),
            Some(vec![13])
        );
    }
}
