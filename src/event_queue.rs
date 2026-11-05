//! Bounded, nonblocking delivery. Overflow stops the producer instead of
//! deadlocking cancellation on a UI thread waiting for process-tree cleanup.
use crate::protocol::Event;
#[cfg(test)]
use std::time::{Duration, Instant};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 2048;
const CONTROL_RESERVE: usize = 32;
struct Queue {
    events: VecDeque<Event>,
    bytes: usize,
    receiver: bool,
}
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    senders: AtomicUsize,
}
pub(crate) struct Sender(Arc<Shared>);
pub(crate) struct Receiver(Arc<Shared>);
pub(crate) fn channel() -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
        queue: Mutex::new(Queue {
            events: VecDeque::new(),
            bytes: 0,
            receiver: true,
        }),
        ready: Condvar::new(),
        senders: AtomicUsize::new(1),
    });
    (Sender(shared.clone()), Receiver(shared))
}
pub(crate) fn event_bytes(event: &Event) -> usize {
    128 + match event {
        Event::Delta { key, text }
        | Event::Text { key, text }
        | Event::ReasoningSummary { key, text, .. } => key.len() + text.len(),
        Event::Tool {
            key,
            title,
            detail,
            status,
        } => key.len() + title.len() + detail.len() + status.len(),
        Event::ToolInput { key, input } => key.len() + input.len(),
        Event::Approval(a) => a.id.len() + a.title.len() + a.detail.len(),
        Event::NativeId(s)
        | Event::ApprovalCancelled(s)
        | Event::Notice(s)
        | Event::CleanupBlocked(s)
        | Event::ErrorBanner(s) => s.len(),
        Event::ApprovalResolved { id, .. } => id.len(),
        Event::Finished { error, .. } => error.as_ref().map_or(0, String::len),
        Event::Plan { steps } => steps.iter().map(|(a, b)| a.len() + b.len() + 64).sum(),
        Event::OpenCodeConfig(value) => value.to_string().len(),
        Event::Usage(_) => 256,
    }
}
impl Clone for Sender {
    fn clone(&self) -> Self {
        self.0.senders.fetch_add(1, Ordering::Relaxed);
        Self(self.0.clone())
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        self.0.senders.fetch_sub(1, Ordering::Release);
        self.0.ready.notify_all();
    }
}
impl Sender {
    pub(crate) fn send(&self, event: Event) -> Result<(), Box<mpsc::SendError<Event>>> {
        let mut queue = self.0.queue.lock().unwrap();
        if !queue.receiver {
            return Err(Box::new(mpsc::SendError(event)));
        }
        let cost = event_bytes(&event);
        if queue.bytes + cost <= MAX_BYTES
            && let Event::Delta { key, text } = &event
            && let Some(Event::Delta {
                key: old_key,
                text: old_text,
            }) = queue.events.back_mut()
            && old_key == key
            && old_text.len() + text.len() <= 256 * 1024
        {
            old_text.push_str(text);
            queue.bytes += text.len();
            return Ok(());
        }
        let finished = matches!(event, Event::Finished { .. });
        let cleanup = matches!(event, Event::CleanupBlocked(_));
        let control = matches!(
            event,
            Event::Finished { .. }
                | Event::ErrorBanner(_)
                | Event::CleanupBlocked(_)
                | Event::ApprovalResolved { .. }
                | Event::ApprovalCancelled(_)
                | Event::Notice(_)
        );
        let limit = MAX_EVENTS
            + if control { CONTROL_RESERVE } else { 0 }
            + usize::from(cleanup || finished)
            + usize::from(finished);
        let bytes = MAX_BYTES
            + if control { 512 * 1024 } else { 0 }
            + if cleanup || finished { MAX_BYTES } else { 0 }
            + if finished { MAX_BYTES } else { 0 };
        if queue.events.len() >= limit || queue.bytes + cost > bytes {
            return Err(Box::new(mpsc::SendError(event)));
        }
        queue.bytes += cost;
        queue.events.push_back(event);
        self.0.ready.notify_one();
        Ok(())
    }
}
impl Receiver {
    pub(crate) fn try_recv(&self) -> Result<Event, mpsc::TryRecvError> {
        let mut queue = self.0.queue.lock().unwrap();
        if let Some(event) = queue.events.pop_front() {
            queue.bytes -= event_bytes(&event);
            return Ok(event);
        }
        if self.0.senders.load(Ordering::Acquire) == 0 {
            Err(mpsc::TryRecvError::Disconnected)
        } else {
            Err(mpsc::TryRecvError::Empty)
        }
    }
    #[cfg(test)]
    pub(crate) fn recv_timeout(&self, timeout: Duration) -> Result<Event, mpsc::RecvTimeoutError> {
        let until = Instant::now() + timeout;
        let mut queue = self.0.queue.lock().unwrap();
        loop {
            if let Some(event) = queue.events.pop_front() {
                queue.bytes -= event_bytes(&event);
                return Ok(event);
            }
            if self.0.senders.load(Ordering::Acquire) == 0 {
                return Err(mpsc::RecvTimeoutError::Disconnected);
            }
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(mpsc::RecvTimeoutError::Timeout);
            }
            queue = self.0.ready.wait_timeout(queue, remaining).unwrap().0;
        }
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        let mut queue = self.0.queue.lock().unwrap();
        queue.receiver = false;
        queue.events.clear();
        queue.bytes = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deltas_coalesce_without_crossing_snapshots_or_message_keys() {
        let (tx, rx) = channel();
        for text in ["a", "b"] {
            tx.send(Event::Delta {
                key: "one".into(),
                text: text.into(),
            })
            .unwrap();
        }
        tx.send(Event::Text {
            key: "one".into(),
            text: "complete".into(),
        })
        .unwrap();
        tx.send(Event::Delta {
            key: "two".into(),
            text: "c".into(),
        })
        .unwrap();
        assert!(matches!(rx.try_recv(), Ok(Event::Delta { text, .. }) if text=="ab"));
        assert!(matches!(rx.try_recv(), Ok(Event::Text { text, .. }) if text=="complete"));
        assert!(matches!(rx.try_recv(), Ok(Event::Delta { key, .. }) if key=="two"));
    }
    #[test]
    fn saturation_preserves_completion_without_waiting_on_consumer() {
        let (tx, rx) = channel();
        for i in 0..MAX_EVENTS {
            tx.send(Event::Delta {
                key: i.to_string(),
                text: "x".into(),
            })
            .unwrap();
        }
        assert!(
            tx.send(Event::Delta {
                key: "overflow".into(),
                text: "x".into()
            })
            .is_err()
        );
        tx.send(Event::Finished {
            error: Some("queue full".into()),
            cancelled: false,
        })
        .unwrap();
        for _ in 0..MAX_EVENTS {
            rx.try_recv().unwrap();
        }
        assert!(matches!(rx.try_recv(), Ok(Event::Finished { .. })));
        drop(rx);
        assert!(tx.send(Event::Notice("gone".into())).is_err());
    }

    #[test]
    fn control_saturation_reserves_a_final_completion_slot() {
        let (sender, receiver) = channel();
        for _ in 0..MAX_EVENTS + CONTROL_RESERVE {
            sender.send(Event::Notice("notice".into())).unwrap();
        }
        assert!(sender.send(Event::Notice("overflow".into())).is_err());
        sender
            .send(Event::CleanupBlocked("cleanup timed out".into()))
            .unwrap();
        sender
            .send(Event::Finished {
                error: Some("queue full".into()),
                cancelled: false,
            })
            .unwrap();
        for _ in 0..MAX_EVENTS + CONTROL_RESERVE {
            assert!(matches!(receiver.try_recv(), Ok(Event::Notice(_))));
        }
        assert!(matches!(receiver.try_recv(), Ok(Event::CleanupBlocked(_))));
        assert!(matches!(receiver.try_recv(), Ok(Event::Finished { .. })));
    }
}
