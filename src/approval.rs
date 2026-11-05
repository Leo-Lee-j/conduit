//! UI queue: a click is pending until the worker accepts its exact ticket.
use crate::protocol::Approval;
use std::collections::{HashSet, VecDeque};

#[derive(Default)]
pub struct ApprovalQueue {
    items: VecDeque<Approval>,
    submitted: HashSet<String>,
}
impl ApprovalQueue {
    pub fn push(&mut self, approval: Approval) {
        if !self.items.iter().any(|a| a.id == approval.id) {
            self.items.push_back(approval);
        }
    }
    pub fn begin(&mut self, id: &str) -> bool {
        self.items.iter().any(|a| a.id == id) && self.submitted.insert(id.into())
    }
    pub fn remove(&mut self, id: &str) -> Option<Approval> {
        self.submitted.remove(id);
        let index = self.items.iter().position(|a| a.id == id)?;
        self.items.remove(index)
    }
    pub fn front(&self) -> Option<&Approval> {
        self.items.front()
    }
    pub fn get(&self, id: &str) -> Option<&Approval> {
        self.items.iter().find(|approval| approval.id == id)
    }
    pub fn is_submitted(&self, id: &str) -> bool {
        self.submitted.contains(id)
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.submitted.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(id: &str) -> Approval {
        Approval {
            id: id.into(),
            title: "test".into(),
            detail: String::new(),
            wire_id: serde_json::json!(id),
            input: serde_json::Value::Null,
            allow_once: true,
            kind: crate::protocol::ApprovalKind::Command,
        }
    }
    #[test]
    fn duplicate_clicks_cancellation_and_order_are_scoped_to_exact_ticket() {
        let mut q = ApprovalQueue::default();
        q.push(request("one"));
        q.push(request("two"));
        q.push(request("one"));
        assert_eq!(q.len(), 2);
        assert!(q.begin("one"));
        assert!(!q.begin("one"));
        assert!(q.is_submitted("one"));
        assert_eq!(q.front().unwrap().id, "one");
        q.remove("one");
        assert!(!q.begin("one"));
        assert_eq!(q.front().unwrap().id, "two");
        q.clear();
        assert!(!q.begin("two"));
        assert!(q.is_empty());
    }
}
