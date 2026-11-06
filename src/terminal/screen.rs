use super::runtime::InputQueue;
use alacritty_terminal::{
    Term,
    event::{Event, EventListener, WindowSize},
    grid::Dimensions,
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, TermMode, cell::Cell, color::Colors, test::TermSize},
    vte::ansi::{CursorShape, Processor, Rgb},
};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

pub(super) struct DisplayInfo {
    pub foreground: AtomicU32,
    pub background: AtomicU32,
    pub cell: AtomicU32,
}
impl Default for DisplayInfo {
    fn default() -> Self {
        Self {
            foreground: AtomicU32::new(0xf3f3f4),
            background: AtomicU32::new(0x0e0e10),
            cell: AtomicU32::new(20 << 16 | 8),
        }
    }
}

#[derive(Clone)]
pub(super) struct Listener {
    pub input: Arc<InputQueue>,
    pub geometry: Arc<AtomicU32>,
    pub display: Arc<DisplayInfo>,
}
impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let reply = match event {
            Event::PtyWrite(text) => Some(text),
            Event::ColorRequest(index, format) => {
                let background = self.display.background.load(Ordering::Acquire);
                let color = match index {
                    256 => rgb(self.display.foreground.load(Ordering::Acquire)),
                    257 => rgb(background),
                    _ => palette(index, background < 0x808080),
                };
                Some(format(color))
            }
            Event::TextAreaSizeRequest(format) => {
                let size = self.geometry.load(Ordering::Acquire);
                let cell = self.display.cell.load(Ordering::Acquire);
                Some(format(WindowSize {
                    num_cols: size as u16,
                    num_lines: (size >> 16) as u16,
                    cell_width: cell as u16,
                    cell_height: (cell >> 16) as u16,
                }))
            }
            _ => None, // Output cannot access clipboard or open URLs.
        };
        if let Some(reply) = reply {
            self.input.response(reply.into_bytes());
        }
    }
}

pub(super) struct Screen {
    pub term: Term<Listener>,
    pub parser: Processor,
    pub size: (u16, u16),
    guard: OutputGuard,
    extra_bytes: usize,
}
impl Screen {
    pub fn new(listener: Listener) -> Self {
        Self {
            term: Term::new(config(100), &TermSize::new(100, 30), listener),
            parser: Processor::new(),
            size: (100, 30),
            guard: OutputGuard::default(),
            extra_bytes: 0,
        }
    }
    pub fn advance(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.extra_bytes += self.guard.check(bytes)?;
        self.parser.advance(&mut self.term, bytes);
        // Plain ASCII has no dynamically allocated cell payload. Audit streams
        // with Unicode/OSC periodically, including history, without taxing logs.
        if self.extra_bytes >= 64 * 1024 {
            self.audit_extra()?;
            self.extra_bytes = 0;
        }
        Ok(())
    }
    fn audit_extra(&self) -> Result<(), String> {
        let grid = self.term.grid();
        let start = Point::new(grid.topmost_line() - 1, grid.last_column());
        let mut bytes = 0;
        for indexed in grid.iter_from(start) {
            let cell = indexed.cell;
            let marks = cell.zerowidth().map_or(0, |s| s.len());
            if marks > 4096 {
                return Err("终端单元格组合字符超出限制，已停止会话".into());
            }
            // Shared hyperlink allocations are charged per cell conservatively.
            bytes += marks * 4 + cell.hyperlink().map_or(0, |h| h.uri().len());
            if bytes > 8 * 1024 * 1024 {
                return Err("终端扩展文本超出内存预算，已停止会话".into());
            }
        }
        Ok(())
    }
    pub fn stop_sync(&mut self) {
        self.parser.stop_sync(&mut self.term);
    }
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.set_options(config(cols));
        self.term
            .resize(TermSize::new(cols as usize, rows as usize));
        self.size = (cols, rows);
    }
    pub fn select(&mut self, row: usize, col: usize, side: Side, kind: Option<SelectionType>) {
        let point = Point::new(
            Line(row as i32 - self.term.grid().display_offset() as i32),
            Column(col),
        );
        if let Some(kind) = kind {
            self.term.selection = Some(Selection::new(kind, point, side));
        } else if let Some(selection) = &mut self.term.selection {
            selection.update(point, side);
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        let content = self.term.renderable_content();
        let cells = content
            .display_iter
            .map(|indexed| {
                let selected = content
                    .selection
                    .as_ref()
                    .is_some_and(|s| s.contains(indexed.point));
                (
                    indexed.point.line.0 + content.display_offset as i32,
                    indexed.point.column.0,
                    indexed.cell.clone(),
                    selected,
                )
            })
            .collect();
        Snapshot {
            cells,
            colors: *content.colors,
            cursor: (
                content.cursor.point.line.0,
                content.cursor.point.column.0,
                content.cursor.shape,
            ),
            offset: content.display_offset,
            mode: content.mode,
            size: self.size,
        }
    }
}

/// Bound unterminated OSC strings before the library's growable parser buffer.
/// Rejection is explicit and stops the process; no byte is silently discarded.
#[derive(Default)]
struct OutputGuard {
    escape: bool,
    osc: Option<usize>,
}
impl OutputGuard {
    fn check(&mut self, bytes: &[u8]) -> Result<usize, String> {
        let mut extra = 0;
        for &byte in bytes {
            if let Some(len) = &mut self.osc {
                if matches!(byte, 7 | 0x18 | 0x1a | 0x1b) {
                    self.osc = None;
                } else {
                    *len += 1;
                    extra += 1;
                    if *len > 4096 {
                        return Err("终端控制字符串超过 4 KiB，已停止会话".into());
                    }
                }
            } else if self.escape && byte == b']' {
                self.osc = Some(0);
            }
            self.escape = byte == 0x1b;
            if !byte.is_ascii() {
                extra += 1;
            }
        }
        Ok(extra)
    }
}
fn config(cols: u16) -> Config {
    Config {
        scrolling_history: 5000.min(1_000_000 / usize::from(cols.max(1))),
        osc52: alacritty_terminal::term::Osc52::Disabled,
        ..Config::default()
    }
}

pub(crate) struct Snapshot {
    pub cells: Vec<(i32, usize, Cell, bool)>,
    pub colors: Colors,
    pub cursor: (i32, usize, CursorShape),
    pub offset: usize,
    pub mode: TermMode,
    pub size: (u16, u16),
}
#[cfg(test)]
impl Snapshot {
    pub fn text(&self) -> String {
        use alacritty_terminal::term::cell::Flags;
        let mut rows = vec![String::new(); self.size.1 as usize];
        for (row, _, cell, _) in &self.cells {
            if *row >= 0
                && (*row as usize) < rows.len()
                && !cell.flags.contains(Flags::WIDE_CHAR_SPACER)
            {
                rows[*row as usize].push(cell.c);
                if let Some(extra) = cell.zerowidth() {
                    rows[*row as usize].extend(extra);
                }
            }
        }
        rows.iter()
            .map(|s| s.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub(crate) fn palette(index: usize, dark: bool) -> Rgb {
    const DARK: [u32; 16] = [
        0x161b22, 0xf85149, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xc9d1d9, 0x6e7681,
        0xff7b72, 0x7ee787, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xf0f6fc,
    ];
    const LIGHT: [u32; 16] = [
        0x24292f, 0xb42318, 0x116329, 0x745500, 0x0550ae, 0x6639ba, 0x0a6068, 0x6e7781, 0x57606a,
        0xcf222e, 0x1a7f37, 0x9a6700, 0x0969da, 0x8250df, 0x1b7c83, 0xffffff,
    ];
    let value = if index < 16 {
        if dark { DARK[index] } else { LIGHT[index] }
    } else if index < 232 {
        let n = index - 16;
        let c = |n| if n == 0 { 0 } else { 55 + 40 * n as u32 };
        c(n / 36) << 16 | c(n / 6 % 6) << 8 | c(n % 6)
    } else if index < 256 {
        let c = 8 + (index as u32 - 232) * 10;
        c << 16 | c << 8 | c
    } else if index == 257 {
        if dark { 0x0e0e10 } else { 0xffffff }
    } else if dark {
        0xf3f3f4
    } else {
        0x24292f
    };
    rgb(value)
}
fn rgb(value: u32) -> Rgb {
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}
