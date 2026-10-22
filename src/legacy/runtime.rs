use crate::pty::Pty;
use alacritty_terminal::{
    Term,
    event::{Event, EventListener, WindowSize},
    term::{Config, TermMode, test::TermSize},
    vte::{ansi::Processor, ansi::Rgb},
};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Duration,
};

#[derive(Clone, Debug, PartialEq)]
pub enum RunState {
    Starting,
    Running,
    Stopping,
    Exited(u32),
    Failed(String),
    Uncertain(String),
}
impl RunState {
    pub fn active(&self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Running | Self::Stopping | Self::Uncertain(_)
        )
    }
    pub fn label(&self) -> String {
        match self {
            Self::Starting => "启动中".into(),
            Self::Running => "运行中".into(),
            Self::Stopping => "正在停止进程树".into(),
            Self::Exited(code) => format!("已退出 · 退出码 {code}"),
            Self::Failed(message) => format!("启动/运行失败：{message}"),
            Self::Uncertain(message) => format!("状态待确认：{message}"),
        }
    }
}

#[derive(Clone)]
pub struct Listener {
    writer: SyncSender<Vec<u8>>,
    geometry: Arc<AtomicU32>,
}
impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let reply = match event {
            Event::PtyWrite(text) => Some(text),
            Event::ColorRequest(index, format) => Some(format(palette(index))),
            Event::TextAreaSizeRequest(format) => {
                let size = self.geometry.load(Ordering::Acquire);
                Some(format(WindowSize {
                    num_cols: size as u16,
                    num_lines: (size >> 16) as u16,
                    cell_width: 8,
                    cell_height: 20,
                }))
            }
            // OSC 52 clipboard access deliberately remains disabled.
            _ => None,
        };
        if let Some(text) = reply {
            let _ = self.writer.try_send(text.into_bytes());
        }
    }
}

pub struct Screen {
    pub term: Term<Listener>,
    pub state: RunState,
    pub pid: Option<u32>,
    pub error: Option<String>,
    pub size: (u16, u16),
    geometry: Arc<AtomicU32>,
}
pub struct Runtime {
    pub screen: Arc<Mutex<Screen>>,
    pub dirty: Arc<AtomicBool>,
    writer: SyncSender<Vec<u8>>,
    control: SyncSender<(u16, u16)>,
    stop: Arc<AtomicBool>,
}

impl Runtime {
    pub fn start(executable: PathBuf, args: Vec<String>, cwd: PathBuf) -> Self {
        let (writer, input_rx) = mpsc::sync_channel(128);
        let (control, resize_rx) = mpsc::sync_channel(1);
        let config = Config {
            scrolling_history: 5000,
            osc52: alacritty_terminal::term::Osc52::Disabled,
            ..Config::default()
        };
        let geometry = Arc::new(AtomicU32::new(30 << 16 | 100));
        let screen = Arc::new(Mutex::new(Screen {
            term: Term::new(
                config,
                &TermSize::new(100, 30),
                Listener {
                    writer: writer.clone(),
                    geometry: geometry.clone(),
                },
            ),
            state: RunState::Starting,
            pid: None,
            error: None,
            size: (100, 30),
            geometry,
        }));
        let dirty = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let this = Self {
            screen: screen.clone(),
            dirty: dirty.clone(),
            writer,
            control,
            stop: stop.clone(),
        };
        thread::spawn(move || {
            run(
                executable, args, cwd, screen, dirty, stop, input_rx, resize_rx,
            )
        });
        this
    }
    pub fn send(&self, input: Vec<u8>) {
        {
            let mut screen = self.screen.lock().unwrap();
            if !matches!(screen.state, RunState::Running) {
                return;
            }
            screen
                .term
                .scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        }
        self.dirty.store(true, Ordering::Release);
        if let Err(error) = self.writer.try_send(input) {
            self.screen.lock().unwrap().error = Some(format!("输入未发送：{error}"));
            self.dirty.store(true, Ordering::Release);
        }
    }
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self
            .control
            .try_send((cols.clamp(2, 1000), rows.clamp(1, 500)));
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        let mut screen = self.screen.lock().unwrap();
        if screen.state.active() {
            screen.state = RunState::Stopping;
        }
        self.dirty.store(true, Ordering::Release);
    }
    pub fn active(&self) -> bool {
        self.screen.lock().unwrap().state.active()
    }
    pub fn mode(&self) -> TermMode {
        *self.screen.lock().unwrap().term.mode()
    }
    pub fn scroll(&self, lines: i32) {
        self.screen
            .lock()
            .unwrap()
            .term
            .scroll_display(alacritty_terminal::grid::Scroll::Delta(lines));
        self.dirty.store(true, Ordering::Release);
    }
    pub fn text(&self) -> String {
        let screen = self.screen.lock().unwrap();
        let content = screen.term.renderable_content();
        let mut output = String::new();
        let mut previous = None;
        for cell in content.display_iter {
            if previous.is_some_and(|line| line != cell.point.line) {
                output.push('\n');
            }
            previous = Some(cell.point.line);
            if !cell
                .flags
                .contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER)
            {
                output.push(cell.c);
            }
        }
        output
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    executable: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    screen: Arc<Mutex<Screen>>,
    dirty: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    input_rx: Receiver<Vec<u8>>,
    resize_rx: Receiver<(u16, u16)>,
) {
    let reader_screen = screen.clone();
    let reader_dirty = dirty.clone();
    let result = Pty::spawn(&executable, &args, &cwd, 100, 30, move |mut reader| {
        thread::spawn(move || {
            let mut processor: Processor = Processor::new();
            let mut buffer = [0; 16384];
            loop {
                if processor
                    .sync_timeout()
                    .sync_timeout()
                    .is_some_and(|deadline| std::time::Instant::now() >= deadline)
                {
                    processor.stop_sync(&mut reader_screen.lock().unwrap().term);
                    reader_dirty.store(true, Ordering::Release);
                }
                let available = match crate::pty::available(&reader) {
                    Ok(0) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Ok(bytes) => (bytes as usize).min(buffer.len()),
                    Err(_) => break,
                };
                match reader.read(&mut buffer[..available]) {
                    Ok(0) => break,
                    Ok(count) => {
                        processor
                            .advance(&mut reader_screen.lock().unwrap().term, &buffer[..count]);
                        reader_dirty.store(true, Ordering::Release);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break, // Broken pipe is expected during ConPTY teardown.
                }
            }
            processor.stop_sync(&mut reader_screen.lock().unwrap().term);
            reader_dirty.store(true, Ordering::Release);
        });
    });
    let (pty, mut input) = match result {
        Ok(value) => value,
        Err(error) => {
            screen.lock().unwrap().state = RunState::Failed(format!("{error:#}"));
            dirty.store(true, Ordering::Release);
            return;
        }
    };
    {
        let mut screen = screen.lock().unwrap();
        screen.pid = Some(pty.pid);
        if !stop.load(Ordering::Acquire) {
            screen.state = RunState::Running;
        }
    }
    dirty.store(true, Ordering::Release);
    let writer_screen = screen.clone();
    let writer_dirty = dirty.clone();
    let writer_done = Arc::new(AtomicBool::new(false));
    let writer_done_flag = writer_done.clone();
    // Blocking writes cannot prevent the Job owner from terminating a non-reading child.
    thread::spawn(move || {
        while !writer_done_flag.load(Ordering::Acquire) {
            let bytes = match input_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(bytes) => bytes,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };
            if let Err(error) = input.write_all(&bytes) {
                writer_screen.lock().unwrap().error = Some(format!("PTY 输入失败：{error}"));
                writer_dirty.store(true, Ordering::Release);
                break;
            }
        }
    });
    let final_state = loop {
        if stop.load(Ordering::Acquire)
            && let Err(error) = pty.stop()
        {
            screen.lock().unwrap().state = RunState::Uncertain(format!("停止失败：{error}"));
            dirty.store(true, Ordering::Release);
        }
        if let Ok((cols, rows)) = resize_rx.try_recv() {
            match pty.resize(cols, rows) {
                Ok(()) => {
                    let mut screen = screen.lock().unwrap();
                    screen
                        .term
                        .resize(TermSize::new(cols as usize, rows as usize));
                    screen.size = (cols, rows);
                    screen
                        .geometry
                        .store((rows as u32) << 16 | cols as u32, Ordering::Release);
                }
                Err(error) => {
                    screen.lock().unwrap().error = Some(format!("调整终端大小失败：{error}"))
                }
            }
            dirty.store(true, Ordering::Release);
        }
        match pty.exit_code() {
            Ok(Some(code)) => {
                // A root CLI can exit before its tools. Keep the workspace occupied until all die.
                let _ = pty.stop();
                match pty.active_processes() {
                    Ok(0) => break RunState::Exited(code),
                    Ok(_) => {
                        screen.lock().unwrap().state = RunState::Stopping;
                    }
                    Err(error) => {
                        screen.lock().unwrap().state = RunState::Uncertain(error.to_string());
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                screen.lock().unwrap().state = RunState::Uncertain(error.to_string());
            }
        }
        thread::sleep(Duration::from_millis(20));
    };
    writer_done.store(true, Ordering::Release);
    drop(pty);
    screen.lock().unwrap().state = final_state;
    dirty.store(true, Ordering::Release);
}

pub fn palette(index: usize) -> Rgb {
    const ANSI: [u32; 16] = [
        0x161b22, 0xf85149, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xc9d1d9, 0x6e7681,
        0xff7b72, 0x7ee787, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xf0f6fc,
    ];
    let color = if index < 16 {
        ANSI[index]
    } else if index < 232 {
        let n = index - 16;
        let channel = |n: usize| if n == 0 { 0 } else { 55 + 40 * n as u32 };
        channel(n / 36) << 16 | channel(n / 6 % 6) << 8 | channel(n % 6)
    } else if index < 256 {
        let grey = 8 + (index as u32 - 232) * 10;
        grey << 16 | grey << 8 | grey
    } else if index == 257 {
        0x0d1117
    } else {
        0xc9d1d9
    };
    Rgb {
        r: (color >> 16) as u8,
        g: (color >> 8) as u8,
        b: color as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    fn wait_until(runtime: &Runtime, predicate: impl Fn(&Screen) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if predicate(&runtime.screen.lock().unwrap()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timeout: {:?}",
                runtime.screen.lock().unwrap().state
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
    #[test]
    fn native_pty_input_output_resize_and_exit() {
        let runtime = Runtime::start(
            PathBuf::from(std::env::var_os("COMSPEC").unwrap()),
            vec!["/d".into(), "/q".into()],
            std::env::temp_dir(),
        );
        wait_until(&runtime, |s| s.state == RunState::Running);
        runtime.resize(110, 35);
        wait_until(&runtime, |s| s.size == (110, 35));
        runtime.send(b"echo CONDUIT_PTY_OK\r".to_vec());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !runtime.text().contains("CONDUIT_PTY_OK") {
            assert!(Instant::now() < deadline, "no terminal output");
            thread::sleep(Duration::from_millis(20));
        }
        runtime.send(b"exit 7\r".to_vec());
        wait_until(&runtime, |s| s.state == RunState::Exited(7));
    }
    #[test]
    fn launch_failure_releases_runtime() {
        let runtime = Runtime::start(
            PathBuf::from(r"Z:\conduit-nonexistent.exe"),
            vec![],
            std::env::temp_dir(),
        );
        wait_until(&runtime, |s| matches!(s.state, RunState::Failed(_)));
        assert!(!runtime.active());
    }
    #[test]
    fn stop_during_startup_reaches_exit() {
        let runtime = Runtime::start(
            PathBuf::from(std::env::var_os("COMSPEC").unwrap()),
            vec!["/d".into(), "/q".into()],
            std::env::temp_dir(),
        );
        runtime.stop();
        wait_until(&runtime, |s| matches!(s.state, RunState::Exited(_)));
        assert!(!runtime.active());
    }
    #[test]
    #[ignore = "requires locally installed Claude Code and Codex; runs --version only"]
    fn installed_providers_report_versions_through_pty() {
        for provider in [
            crate::domain::Provider::Claude,
            crate::domain::Provider::Codex,
        ] {
            let executable = provider
                .executable()
                .expect("provider executable not installed");
            let runtime =
                Runtime::start(executable, vec!["--version".into()], std::env::temp_dir());
            wait_until(&runtime, |s| matches!(s.state, RunState::Exited(0)));
            let deadline = Instant::now() + Duration::from_secs(5);
            while runtime.text().trim().is_empty() {
                assert!(Instant::now() < deadline, "missing version output");
                thread::sleep(Duration::from_millis(20));
            }
            let output = runtime.text();
            assert!(
                output.contains(if provider == crate::domain::Provider::Claude {
                    "Claude Code"
                } else {
                    "codex-cli"
                }),
                "unexpected version output"
            );
            println!("{}: {}", provider.label(), output.trim());
        }
    }
}
