use std::{
    collections::{HashSet, VecDeque},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::FILETIME,
    System::{
        ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
        },
        Threading::{GetCurrentProcess, GetProcessTimes},
    },
};

pub(crate) const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
const RETENTION: Duration = Duration::from_secs(60);
const HISTORY_CAPACITY: usize = 30;

#[derive(Clone, Copy, Default)]
pub(crate) struct Timings {
    pub count: u32,
    total: Duration,
    pub maximum: Duration,
}

impl Timings {
    fn record(&mut self, duration: Duration) {
        self.count += 1;
        self.total += duration;
        self.maximum = self.maximum.max(duration);
    }
    pub fn average(&self) -> Option<Duration> {
        self.total.checked_div(self.count)
    }
}

struct Sample {
    at: Instant,
    cpu: Option<f64>,
    private_bytes: Option<usize>,
}

/// Only numeric, process-local diagnostics. Never retains chat payloads.
pub(crate) struct PerformanceMonitor {
    started: Instant,
    last_sample: Instant,
    previous_cpu: Option<u64>,
    pub logical_cpus: usize,
    history: VecDeque<Sample>,
    pending_render: Timings,
    pending_messages: Timings,
    pending_events: usize,
    rendered_indices: HashSet<usize>,
    pub cpu_percent: Option<f64>,
    pub memory_bytes: Option<usize>,
    pub private_bytes: Option<usize>,
    pub peak_memory_bytes: Option<usize>,
    pub render: Timings,
    pub message_render: Timings,
    pub event_count: usize,
    pub sample_duration: Duration,
    pub message_count: usize,
    pub active_runs: usize,
    pub session_count: usize,
}

impl PerformanceMonitor {
    pub fn new() -> Self {
        let now = Instant::now();
        let mut monitor = Self {
            started: now,
            last_sample: now,
            previous_cpu: process_cpu_ticks(),
            logical_cpus: std::thread::available_parallelism().map_or(1, usize::from),
            history: VecDeque::with_capacity(HISTORY_CAPACITY),
            pending_render: Timings::default(),
            pending_messages: Timings::default(),
            pending_events: 0,
            rendered_indices: HashSet::new(),
            cpu_percent: None,
            memory_bytes: None,
            private_bytes: None,
            peak_memory_bytes: None,
            render: Timings::default(),
            message_render: Timings::default(),
            event_count: 0,
            sample_duration: Duration::ZERO,
            message_count: 0,
            active_runs: 0,
            session_count: 0,
        };
        monitor.sample_memory();
        monitor
    }

    pub fn sample_due(&self) -> bool {
        self.last_sample.elapsed() >= SAMPLE_INTERVAL
    }

    pub fn reset(&mut self) {
        let now = Instant::now();
        self.started = now;
        self.last_sample = now;
        self.previous_cpu = process_cpu_ticks();
        self.history.clear();
        self.pending_render = Timings::default();
        self.pending_messages = Timings::default();
        self.pending_events = 0;
        self.rendered_indices.clear();
        self.cpu_percent = None;
        self.memory_bytes = None;
        self.private_bytes = None;
        self.peak_memory_bytes = None;
        self.render = Timings::default();
        self.message_render = Timings::default();
        self.event_count = 0;
        self.sample_duration = Duration::ZERO;
        self.message_count = 0;
        self.active_runs = 0;
        self.session_count = 0;
        self.sample_memory();
    }

    pub fn sample(&mut self) {
        let now = Instant::now();
        let cpu = process_cpu_ticks();
        self.sample_duration = now - self.last_sample;
        self.cpu_percent = self.previous_cpu.zip(cpu).and_then(|(previous, current)| {
            current
                .checked_sub(previous)
                .map(|ticks| cpu_percent(ticks, self.sample_duration, self.logical_cpus))
        });
        self.previous_cpu = cpu;
        self.last_sample = now;
        self.sample_memory();
        self.render = std::mem::take(&mut self.pending_render);
        self.message_render = std::mem::take(&mut self.pending_messages);
        self.event_count = std::mem::take(&mut self.pending_events);
        self.history
            .retain(|sample| now.duration_since(sample.at) < RETENTION);
        if self.history.len() == HISTORY_CAPACITY {
            self.history.pop_front();
        }
        self.history.push_back(Sample {
            at: now,
            cpu: self.cpu_percent,
            private_bytes: self.private_bytes,
        });
    }

    pub fn begin_render(&mut self, messages: usize, active_runs: usize, sessions: usize) {
        self.rendered_indices.clear();
        self.message_count = messages;
        self.active_runs = active_runs;
        self.session_count = sessions;
    }
    pub fn record_render(&mut self, elapsed: Duration) {
        self.pending_render.record(elapsed);
    }
    pub fn record_message_render(&mut self, index: usize, elapsed: Duration) {
        self.rendered_indices.insert(index);
        self.pending_messages.record(elapsed);
    }
    pub fn mounted_messages(&self) -> usize {
        self.rendered_indices.len()
    }
    pub fn record_events(&mut self, count: usize) {
        self.pending_events += count;
    }
    pub fn recent_cpu_peak(&self) -> Option<f64> {
        self.history
            .iter()
            .filter(|sample| sample.at.elapsed() < RETENTION)
            .filter_map(|sample| sample.cpu)
            .reduce(f64::max)
    }
    pub fn private_growth(&self) -> Option<i64> {
        let mut samples = self
            .history
            .iter()
            .filter(|sample| sample.at.elapsed() < RETENTION);
        let first = samples.next()?;
        let last = samples.next_back()?;
        Some(last.private_bytes? as i64 - first.private_bytes? as i64)
    }
    pub fn uptime(&self) -> String {
        let seconds = self.started.elapsed().as_secs();
        format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    }
    fn sample_memory(&mut self) {
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        // SAFETY: The EX structure starts with PROCESS_MEMORY_COUNTERS and cb
        // names its full size. The current-process pseudo handle needs no close.
        let success = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
                counters.cb,
            )
        } != 0;
        self.memory_bytes = success.then_some(counters.WorkingSetSize);
        self.private_bytes = success.then_some(counters.PrivateUsage);
        self.peak_memory_bytes = success.then_some(counters.PeakWorkingSetSize);
    }
}

fn process_cpu_ticks() -> Option<u64> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: All output pointers refer to initialized FILETIME values.
    let success = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    } != 0;
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    success.then(|| ticks(kernel) + ticks(user))
}
fn cpu_percent(ticks: u64, elapsed: Duration, logical_cpus: usize) -> f64 {
    (ticks as f64
        / 10_000_000.0
        / elapsed.as_secs_f64().max(f64::EPSILON)
        / logical_cpus.max(1) as f64
        * 100.0)
        .clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_usage_is_normalized_to_machine_capacity() {
        assert_eq!(cpu_percent(10_000_000, Duration::from_secs(1), 4), 25.0);
        assert_eq!(cpu_percent(0, Duration::from_secs(1), 4), 0.0);
        assert_eq!(cpu_percent(100_000_000, Duration::from_secs(1), 4), 100.0);
    }
    #[test]
    fn samples_real_private_memory_and_resets_interval_counters() {
        let mut monitor = PerformanceMonitor::new();
        assert!(monitor.memory_bytes.is_some_and(|bytes| bytes > 0));
        assert!(monitor.private_bytes.is_some_and(|bytes| bytes > 0));
        assert!(!monitor.sample_due());
        monitor.begin_render(1000, 2, 4);
        monitor.record_render(Duration::from_millis(2));
        monitor.record_render(Duration::from_millis(4));
        monitor.record_message_render(900, Duration::from_millis(1));
        monitor.record_message_render(900, Duration::from_millis(1));
        monitor.record_events(120);
        monitor.sample();
        assert_eq!(monitor.render.average(), Some(Duration::from_millis(3)));
        assert_eq!(monitor.render.maximum, Duration::from_millis(4));
        assert_eq!(monitor.mounted_messages(), 1);
        assert_eq!(monitor.event_count, 120);
        monitor.sample();
        assert_eq!(monitor.render.average(), None);
        assert_eq!(monitor.event_count, 0);
    }
    #[test]
    fn reset_clears_counters_and_starts_a_new_window() {
        let mut monitor = PerformanceMonitor::new();
        monitor.begin_render(10, 1, 2);
        monitor.record_render(Duration::from_millis(3));
        monitor.record_message_render(1, Duration::from_millis(1));
        monitor.record_events(5);
        monitor.sample();
        monitor.reset();
        assert_eq!(monitor.render.average(), None);
        assert_eq!(monitor.event_count, 0);
        assert_eq!(monitor.message_count, 0);
        assert!(monitor.history.is_empty());
        assert!(monitor.memory_bytes.is_some_and(|bytes| bytes > 0));
        assert!(!monitor.sample_due());
    }
    #[test]
    fn history_is_bounded_and_growth_keeps_its_sign_and_missing_values() {
        let mut monitor = PerformanceMonitor::new();
        assert_eq!(monitor.private_growth(), None);
        let now = Instant::now();
        monitor.history.push_back(Sample {
            at: now,
            cpu: None,
            private_bytes: Some(200),
        });
        monitor.history.push_back(Sample {
            at: now,
            cpu: Some(5.0),
            private_bytes: Some(100),
        });
        assert_eq!(monitor.private_growth(), Some(-100));
        assert_eq!(monitor.recent_cpu_peak(), Some(5.0));
        for _ in 0..100 {
            monitor.sample();
        }
        assert_eq!(monitor.history.len(), HISTORY_CAPACITY);
        monitor.history.clear();
        monitor.history.push_back(Sample {
            at: now - RETENTION,
            cpu: Some(99.0),
            private_bytes: Some(10),
        });
        monitor.sample();
        assert_eq!(monitor.history.len(), 1);
    }
}
