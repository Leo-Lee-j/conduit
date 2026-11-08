//! A scoped clock for UI tests that distinguish cache reuse from age-label ticks.
use std::{cell::Cell, marker::PhantomData, rc::Rc};

thread_local! {
    static NOW: Cell<Option<u64>> = const { Cell::new(None) };
}

pub(super) fn now() -> Option<u64> {
    NOW.get()
}

// Keep the override on the test's thread and restore it even if an assertion panics.
pub(in crate::app) struct Clock {
    previous: Option<u64>,
    _thread: PhantomData<Rc<()>>,
}

impl Clock {
    pub(in crate::app) fn freeze(now: u64) -> Self {
        Self {
            previous: NOW.replace(Some(now)),
            _thread: PhantomData,
        }
    }

    pub(in crate::app) fn set(&self, now: u64) {
        NOW.set(Some(now));
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        NOW.set(self.previous);
    }
}
