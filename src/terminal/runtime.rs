use super::{
    ShellProfile,
    pty::Pty,
    screen::{DisplayInfo, Listener, Screen, Snapshot},
};
use alacritty_terminal::{grid::Scroll, index::Side, selection::SelectionType};
use std::{
    collections::VecDeque,
    io::{Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::WAIT_FAILED,
    System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects},
};

const MAX_INPUT: usize = 1024 * 1024;
const QUEUE_BYTES: usize = 2 * MAX_INPUT;
const RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunState {
    Starting,
    Running,
    Stopping,
    Exited(u32),
    Failed(String),
    CleanupFailed(String),
}
impl RunState {
    pub fn active(&self) -> bool {
        !matches!(self, Self::Exited(_) | Self::Failed(_))
    }
    pub fn label(&self) -> String {
        match self {
            Self::Starting => "启动中".into(),
            Self::Running => "运行中".into(),
            Self::Stopping => "正在终止…".into(),
            Self::Exited(code) => format!("已退出 · {code}"),
            Self::Failed(e) => format!("启动/运行失败：{e}"),
            Self::CleanupFailed(e) => format!("清理未完成：{e}"),
        }
    }
}

struct Wake(OwnedHandle);
impl Wake {
    fn new() -> Self {
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        assert!(!handle.is_null(), "create terminal control event");
        Self(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
    fn signal(&self) {
        unsafe {
            SetEvent(self.0.as_raw_handle());
        }
    }
}
#[derive(Default)]
struct Pending {
    items: VecDeque<(Vec<u8>, bool)>,
    bytes: usize,
    responses: usize,
    closed: bool,
}
pub(super) struct InputQueue {
    pending: Mutex<Pending>,
    ready: Condvar,
    fault: Mutex<Option<String>>,
    wake: Arc<Wake>,
}
impl InputQueue {
    fn new(wake: Arc<Wake>) -> Self {
        Self {
            pending: Mutex::new(Pending::default()),
            ready: Condvar::new(),
            fault: Mutex::new(None),
            wake,
        }
    }
    fn push(&self, bytes: Vec<u8>, response: bool) -> Result<(), String> {
        let mut q = self.pending.lock().unwrap();
        if q.closed {
            return Err("终端已停止接收输入".into());
        }
        if bytes.len() > MAX_INPUT {
            return Err("单次输入不能超过 1 MiB".into());
        }
        if q.items.iter().filter(|(_, r)| *r == response).count() >= 128
            || if response {
                q.responses + bytes.len() > RESPONSE_BYTES
            } else {
                q.bytes + bytes.len() > QUEUE_BYTES
            }
        {
            return Err("终端暂时无法接收输入，请稍后重试".into());
        }
        if response {
            q.responses += bytes.len();
        } else {
            q.bytes += bytes.len();
        }
        q.items.push_back((bytes, response));
        self.ready.notify_one();
        Ok(())
    }
    pub fn response(&self, bytes: Vec<u8>) {
        if let Err(error) = self.push(bytes, true) {
            self.fail(error);
        }
    }
    fn fail(&self, error: String) {
        *self.fault.lock().unwrap() = Some(error);
        self.wake.signal();
    }
    fn close(&self) {
        let mut q = self.pending.lock().unwrap();
        q.closed = true;
        q.items.clear();
        q.bytes = 0;
        q.responses = 0;
        self.ready.notify_all();
    }
}

pub(crate) struct Runtime {
    display: Arc<DisplayInfo>,
    screen: Arc<Mutex<Screen>>,
    state: Arc<Mutex<RunState>>,
    pub dirty: Arc<AtomicBool>,
    input: Arc<InputQueue>,
    wake: Arc<Wake>,
    stop: Arc<AtomicBool>,
    geometry: Arc<AtomicU32>,
    desired_size: Arc<AtomicU32>,
}
impl Runtime {
    pub fn start(profile: ShellProfile, cwd: PathBuf) -> Self {
        Self::launch(move || profile.resolve().map(|(exe, args)| (exe, args, cwd)))
    }
    #[cfg(test)]
    pub fn command(exe: PathBuf, args: Vec<String>, cwd: PathBuf) -> Self {
        Self::launch(move || Ok((exe, args, cwd)))
    }
    fn launch(
        resolve: impl FnOnce() -> anyhow::Result<(PathBuf, Vec<String>, PathBuf)> + Send + 'static,
    ) -> Self {
        let wake = Arc::new(Wake::new());
        let input = Arc::new(InputQueue::new(wake.clone()));
        let geometry = Arc::new(AtomicU32::new(30 << 16 | 100));
        let display = Arc::new(DisplayInfo::default());
        let screen = Arc::new(Mutex::new(Screen::new(Listener {
            display: display.clone(),
            input: input.clone(),
            geometry: geometry.clone(),
        })));
        let state = Arc::new(Mutex::new(RunState::Starting));
        let dirty = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let desired_size = Arc::new(AtomicU32::new(30 << 16 | 100));
        let worker = Worker {
            screen: screen.clone(),
            state: state.clone(),
            dirty: dirty.clone(),
            input: input.clone(),
            wake: wake.clone(),
            stop: stop.clone(),
            geometry: geometry.clone(),
            desired_size: desired_size.clone(),
        };
        thread::spawn(move || worker.run(resolve));
        Self {
            display,
            screen,
            state,
            dirty,
            input,
            wake,
            stop,
            geometry,
            desired_size,
        }
    }
    pub fn state(&self) -> RunState {
        self.state.lock().unwrap().clone()
    }
    pub fn active(&self) -> bool {
        self.state().active()
    }
    pub fn stop(&self) {
        if self.active() {
            self.stop.store(true, Ordering::Release);
            self.wake.signal();
        }
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
        if self.state() != RunState::Running || self.stop.load(Ordering::Acquire) {
            return Err("终端尚未就绪或已停止".into());
        }
        self.input.push(bytes, false)?;
        if let Ok(mut screen) = self.screen.try_lock() {
            screen.term.scroll_display(Scroll::Bottom);
            screen.term.selection = None;
        }
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
    pub fn resize(&self, cols: u16, rows: u16) {
        let geometry = (u32::from(rows.clamp(1, 300)) << 16) | u32::from(cols.clamp(2, 500));
        if self.desired_size.swap(geometry, Ordering::AcqRel) != geometry {
            self.wake.signal();
        }
    }
    pub fn display(&self, foreground: u32, background: u32, width: u16, height: u16) {
        self.display.foreground.store(foreground, Ordering::Release);
        self.display.background.store(background, Ordering::Release);
        self.display.cell.store(
            u32::from(width) | u32::from(height) << 16,
            Ordering::Release,
        );
    }
    pub fn size(&self) -> (u16, u16) {
        let n = self.geometry.load(Ordering::Acquire);
        (n as u16, (n >> 16) as u16)
    }
    pub fn snapshot(&self) -> Option<Snapshot> {
        self.screen.try_lock().ok().map(|s| s.snapshot())
    }
    pub fn scroll(&self, lines: i32) {
        if let Ok(mut screen) = self.screen.try_lock() {
            screen.term.scroll_display(Scroll::Delta(lines));
            self.dirty.store(true, Ordering::Release);
        }
    }
    pub fn select(&self, row: usize, col: usize, right: bool, kind: Option<SelectionType>) {
        if let Ok(mut screen) = self.screen.try_lock() {
            screen.select(row, col, if right { Side::Right } else { Side::Left }, kind);
            self.dirty.store(true, Ordering::Release);
        }
    }
    pub fn selected_text(&self) -> Option<String> {
        self.screen
            .try_lock()
            .ok()
            .and_then(|s| s.term.selection_to_string())
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.wake.signal();
    }
}

struct Worker {
    screen: Arc<Mutex<Screen>>,
    state: Arc<Mutex<RunState>>,
    dirty: Arc<AtomicBool>,
    input: Arc<InputQueue>,
    wake: Arc<Wake>,
    stop: Arc<AtomicBool>,
    geometry: Arc<AtomicU32>,
    desired_size: Arc<AtomicU32>,
}
impl Worker {
    fn state(&self, value: RunState) {
        let mut state = self.state.lock().unwrap();
        if *state != value {
            *state = value;
            self.dirty.store(true, Ordering::Release);
        }
    }
    fn run(self, resolve: impl FnOnce() -> anyhow::Result<(PathBuf, Vec<String>, PathBuf)>) {
        let command = resolve().and_then(|(exe, args, cwd)| {
            anyhow::ensure!(
                cwd.is_absolute() && cwd.is_dir(),
                "启动目录不存在或不可访问：{}",
                cwd.display()
            );
            anyhow::ensure!(!self.stop.load(Ordering::Acquire), "启动已取消");
            Ok((exe, args, cwd))
        });
        let (exe, args, cwd) = match command {
            Ok(c) => c,
            Err(e) => {
                self.input.close();
                self.state(RunState::Failed(e.to_string()));
                return;
            }
        };
        let screen = self.screen.clone();
        let dirty = self.dirty.clone();
        let wake = self.wake.clone();
        let reader_fault = self.input.clone();
        let mut reader = None;
        let result = Pty::spawn(&exe, &args, &cwd, 100, 30, |mut pipe| {
            reader = Some(thread::spawn(move || {
                let mut bytes = [0; 16384];
                let mut parsing = true;
                loop {
                    match pipe.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(count) => {
                            // After a reported parser failure, keep draining until
                            // the owner has stopped the Job and closed ConPTY.
                            if !parsing {
                                continue;
                            }
                            let mut screen = screen.lock().unwrap();
                            let parsed =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    screen.advance(&bytes[..count])
                                }));
                            match parsed {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => {
                                    reader_fault.fail(error);
                                    parsing = false;
                                    continue;
                                }
                                Err(_) => {
                                    reader_fault.fail("终端解析器异常，正在清理进程".into());
                                    parsing = false;
                                    continue;
                                }
                            }
                            if screen.parser.sync_timeout().sync_timeout().is_some() {
                                wake.signal();
                            }
                            drop(screen);
                            dirty.store(true, Ordering::Release);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            if !matches!(error.raw_os_error(), Some(109 | 232 | 995)) {
                                reader_fault.fail(format!("终端读取失败：{error}"));
                            }
                            break;
                        }
                    }
                }
                if parsing {
                    screen.lock().unwrap().stop_sync();
                }
                dirty.store(true, Ordering::Release);
                wake.signal();
            }));
        });
        let (mut pty, mut pipe) = match result {
            Ok(value) => value,
            Err(error) => {
                self.input.close();
                if let Some(reader) = reader {
                    let _ = reader.join();
                }
                self.state(RunState::Failed(format!("{error:#}")));
                return;
            }
        };
        let input = self.input.clone();
        let writer = thread::spawn(move || {
            loop {
                let mut q = input.pending.lock().unwrap();
                while q.items.is_empty() && !q.closed {
                    q = input.ready.wait(q).unwrap();
                }
                if q.closed {
                    break;
                }
                let (bytes, response) = q.items.pop_front().unwrap();
                drop(q);
                let result = pipe.write_all(&bytes);
                let mut q = input.pending.lock().unwrap();
                if !q.closed {
                    if response {
                        q.responses -= bytes.len();
                    } else {
                        q.bytes -= bytes.len();
                    }
                }
                let closed = q.closed;
                drop(q);
                if let Err(error) = result {
                    if !closed {
                        input.fail(format!("终端写入失败：{error}"));
                    }
                    break;
                }
            }
        });
        if !self.stop.load(Ordering::Acquire) {
            self.state(RunState::Running);
        }
        let mut last_resize = Instant::now() - Duration::from_millis(34);
        let (code, failure) = loop {
            if self.stop.load(Ordering::Acquire) {
                break (1, None);
            }
            if let Some(error) = self.input.fault.lock().unwrap().take() {
                break (1, Some(error));
            }
            match pty.exit_code() {
                Ok(Some(code)) => break (code, None),
                Err(e) => break (1, Some(e.to_string())),
                _ => {}
            }
            if reader.as_ref().is_some_and(|r| r.is_finished()) {
                break (1, Some("终端输出管道提前关闭".into()));
            }
            let desired = self.desired_size.load(Ordering::Acquire);
            if desired != self.geometry.load(Ordering::Acquire)
                && last_resize.elapsed() >= Duration::from_millis(33)
            {
                let (cols, rows) = (desired as u16, (desired >> 16) as u16);
                if let Err(error) = pty.resize(cols, rows) {
                    break (1, Some(error.to_string()));
                }
                last_resize = Instant::now();
                self.screen.lock().unwrap().resize(cols, rows);
                self.geometry.store(desired, Ordering::Release);
                self.dirty.store(true, Ordering::Release);
            }
            let timeout = {
                let mut screen = self.screen.lock().unwrap();
                match screen.parser.sync_timeout().sync_timeout() {
                    Some(deadline) if deadline <= Instant::now() => {
                        screen.stop_sync();
                        self.dirty.store(true, Ordering::Release);
                        0
                    }
                    Some(deadline) => deadline
                        .saturating_duration_since(Instant::now())
                        .as_millis()
                        .clamp(1, u32::MAX as u128 - 1)
                        as u32,
                    None => INFINITE,
                }
            };
            let timeout = if self.desired_size.load(Ordering::Acquire)
                != self.geometry.load(Ordering::Acquire)
            {
                timeout.min(
                    Duration::from_millis(33)
                        .saturating_sub(last_resize.elapsed())
                        .as_millis()
                        .max(1) as u32,
                )
            } else {
                timeout
            };
            let handles = [self.wake.0.as_raw_handle(), pty.process_handle()];
            if unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, timeout) } == WAIT_FAILED {
                break (1, Some(std::io::Error::last_os_error().to_string()));
            }
        };
        self.state(RunState::Stopping);
        self.input.close();
        let started = Instant::now();
        let mut console = None;
        let mut stopped = false;
        loop {
            if !stopped {
                match pty.stop() {
                    Ok(()) => {
                        stopped = true;
                        console = pty.close_console();
                    }
                    Err(e) => self.state(RunState::CleanupFailed(e.to_string())),
                }
            }
            let empty = matches!(pty.active_processes(), Ok(0));
            if empty
                && writer.is_finished()
                && reader.as_ref().is_none_or(|t| t.is_finished())
                && console.as_ref().is_none_or(|t| t.is_finished())
            {
                break;
            }
            if started.elapsed() > Duration::from_secs(5) {
                self.state(RunState::CleanupFailed(
                    "正在等待管道和进程树释放；可重试终止".into(),
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
        let writer_ok = writer.join().is_ok();
        let reader_ok = reader.is_none_or(|r| r.join().is_ok());
        let console_ok = console.is_none_or(|c| c.join().is_ok());
        drop(pty);
        self.state(if let Some(error) = failure {
            RunState::Failed(error)
        } else if !writer_ok || !reader_ok || !console_ok {
            RunState::Failed("终端线程异常退出，资源已释放".into())
        } else {
            RunState::Exited(code)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_screen() -> (Screen, Arc<InputQueue>) {
        let input = Arc::new(InputQueue::new(Arc::new(Wake::new())));
        let screen = Screen::new(Listener {
            input: input.clone(),
            geometry: Arc::new(AtomicU32::new(30 << 16 | 100)),
            display: Arc::new(DisplayInfo::default()),
        });
        (screen, input)
    }
    #[test]
    fn parser_preserves_unicode_ansi_selection_and_alt_screen() {
        use alacritty_terminal::{
            term::{TermMode, cell::Flags},
            vte::ansi::Color,
        };
        let (mut screen, input) = test_screen();
        let text = "\x1b[31m红色\x1b[0m e\u{301}";
        for byte in text.as_bytes() {
            screen.advance(&[*byte]).unwrap();
        }
        let snapshot = screen.snapshot();
        assert!(snapshot.text().contains("红色 e\u{301}"));
        assert!(
            snapshot
                .cells
                .iter()
                .any(|(_, _, c, _)| c.flags.contains(Flags::WIDE_CHAR))
        );
        assert!(
            snapshot
                .cells
                .iter()
                .any(|(_, _, c, _)| c.c == '红' && matches!(c.fg, Color::Named(_)))
        );
        screen.select(0, 0, Side::Left, Some(SelectionType::Simple));
        screen.select(0, 3, Side::Right, None);
        assert!(screen.term.selection_to_string().unwrap().contains("红色"));
        screen.advance(b"\x1b[?1049hALT\x1b[?2004h").unwrap();
        assert!(
            screen
                .snapshot()
                .mode
                .contains(TermMode::ALT_SCREEN | TermMode::BRACKETED_PASTE)
        );
        screen.advance(b"\x1b[?1049l").unwrap();
        assert!(screen.snapshot().text().contains("红色"));
        screen.advance(b"\x1b[6n").unwrap();
        assert!(!input.pending.lock().unwrap().items.is_empty());
    }
    #[test]
    fn parser_rejects_unbounded_control_and_combining_payloads() {
        let (mut screen, _) = test_screen();
        screen.advance(b"\x1b]").unwrap();
        assert!(screen.advance(&vec![b'x'; 5000]).is_err());
        let (mut screen, _) = test_screen();
        screen.advance(b"a").unwrap();
        let payload = "\u{301}".repeat(4096);
        let mut rejected = false;
        for _ in 0..10 {
            if screen.advance(payload.as_bytes()).is_err() {
                rejected = true;
                break;
            }
        }
        assert!(rejected);
    }
    #[test]
    fn parser_history_and_snapshot_stay_bounded_under_large_logs() {
        use alacritty_terminal::grid::Dimensions;
        let (mut screen, _) = test_screen();
        screen.resize(160, 40);
        let chunk =
            b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\r\n".repeat(128);
        let started = Instant::now();
        for _ in 0..512 {
            screen.advance(&chunk).unwrap();
        }
        let parsed = started.elapsed();
        let started = Instant::now();
        for _ in 0..100 {
            let snapshot = screen.snapshot();
            assert_eq!(snapshot.cells.len(), 160 * 40);
        }
        eprintln!(
            "terminal probe: {} bytes parsed in {:?}; 100 snapshots in {:?}; cell={} bytes",
            chunk.len() * 512,
            parsed,
            started.elapsed(),
            std::mem::size_of::<alacritty_terminal::term::cell::Cell>()
        );
        assert!(screen.term.grid().history_size() <= 5000);
        assert!(screen.term.grid().total_lines() * 160 <= 1_000_000 + 160 * 40);
    }
    #[test]
    fn input_reserves_full_transaction_and_rejects_after_close() {
        let queue = InputQueue::new(Arc::new(Wake::new()));
        assert!(queue.push(vec![0; MAX_INPUT], false).is_ok());
        assert!(queue.push(vec![0; MAX_INPUT], false).is_ok());
        assert!(queue.push(vec![1], false).is_err());
        assert!(queue.push(vec![1; 128], true).is_ok());
        assert_eq!(queue.pending.lock().unwrap().bytes, QUEUE_BYTES);
        queue.close();
        assert!(queue.push(vec![1], false).is_err());
    }
}
