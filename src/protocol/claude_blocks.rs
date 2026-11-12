//! Claude emits one assistant snapshot per block, sharing the API message ID.
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Default)]
pub(super) struct Blocks {
    messages: HashMap<String, Message>,
}

#[derive(Default)]
struct Message {
    streams: BTreeMap<u64, usize>,
    completed: HashSet<usize>,
    snapshots: HashMap<(String, usize), usize>,
    next: usize,
}

impl Blocks {
    pub(super) fn stream(&mut self, id: &str, index: u64) -> usize {
        let message = self.messages.entry(id.into()).or_default();
        *message.streams.entry(index).or_insert_with(|| {
            let ordinal = message.next;
            message.next += 1;
            ordinal
        })
    }

    pub(super) fn streamed(&self, id: &str, index: u64) -> Option<usize> {
        self.messages.get(id)?.streams.get(&index).copied()
    }

    pub(super) fn snapshot(
        &mut self,
        id: &str,
        uuid: &str,
        position: usize,
        aggregate: bool,
    ) -> usize {
        let message = self.messages.entry(id.into()).or_default();
        let snapshot = (uuid.to_owned(), position);
        if !uuid.is_empty()
            && let Some(ordinal) = message.snapshots.get(&snapshot)
        {
            return *ordinal;
        }
        // With partial messages, the snapshot precedes content_block_stop.
        // Match the first stream block of this kind still awaiting its snapshot.
        // Older aggregate messages use their position among blocks of this kind.
        let ordinal = if aggregate {
            message.streams.values().nth(position).copied()
        } else {
            message
                .streams
                .values()
                .find(|ordinal| !message.completed.contains(ordinal))
                .copied()
        }
        .unwrap_or({
            if aggregate || uuid.is_empty() {
                position
            } else {
                message.next
            }
        });
        message.next = message.next.max(ordinal + 1);
        message.completed.insert(ordinal);
        if !uuid.is_empty() {
            message.snapshots.insert(snapshot, ordinal);
        }
        ordinal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_only_blocks_share_message_id_without_overwriting_or_replaying() {
        let mut blocks = Blocks::default();
        assert_eq!(blocks.snapshot("m", "first", 0, false), 0);
        assert_eq!(blocks.snapshot("other", "subagent", 0, false), 0);
        assert_eq!(blocks.snapshot("m", "second", 0, false), 1);
        assert_eq!(blocks.snapshot("m", "first", 0, false), 0);
        assert_eq!(blocks.snapshot("m", "second", 0, false), 1);
        assert_eq!(blocks.snapshot("m", "third", 0, false), 2);
    }

    #[test]
    fn streamed_snapshots_and_legacy_aggregates_reuse_their_block_identity() {
        let mut blocks = Blocks::default();
        assert_eq!(blocks.stream("m", 1), 0);
        assert_eq!(blocks.snapshot("m", "first", 0, false), 0);
        assert_eq!(blocks.stream("m", 4), 1);
        assert_eq!(blocks.snapshot("m", "second", 0, false), 1);
        assert_eq!(blocks.snapshot("m", "first", 0, false), 0);
        assert_eq!(blocks.snapshot("m", "", 0, true), 0);
        assert_eq!(blocks.snapshot("m", "", 1, true), 1);
        assert_eq!(blocks.streamed("m", 4), Some(1));
        assert_eq!(blocks.streamed("other", 4), None);
    }
}
