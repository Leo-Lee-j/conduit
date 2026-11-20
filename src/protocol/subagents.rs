//! Native child-agent lifecycle tracking, isolated from the main transcript and usage.
use super::*;
use crate::subagents::{Agent, Status};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Agents {
    pub records: BTreeMap<String, Agent>,
    decoders: HashMap<String, Box<Protocol>>,
    capability_reads: HashMap<String, String>,
    pub(super) claude_tasks: HashMap<String, super::claude_tasks::Task>,
    pub(super) claude_inputs: std::collections::VecDeque<(String, super::claude_tasks::TaskInput)>,
    published: BTreeMap<String, Agent>,
    pending_reads: HashMap<String, (String, String, Instant)>,
    prior_turns: HashMap<String, String>,
    operations: std::collections::HashSet<String>,
    read_sequence: u64,
    cursor: usize,
    last_poll: Option<Instant>,
    pub main_done: bool,
}
impl Agents {
    pub fn publish(&mut self, step: &mut Step) {
        for (id, record) in &mut self.records {
            record.bounded();
            if self.published.get(id) != Some(record) {
                record.updated_at = crate::subagents::now();
                self.published.insert(id.clone(), record.clone());
                if let Some(task) = self.claude_tasks.get(id) {
                    if task.agent && !task.ambient {
                        step.events.push(Event::Subagent(record.clone()));
                    }
                    if task.inline && !task.ambient && (!task.agent || task.has_tool()) {
                        step.events.push(task.tool_event(record));
                    }
                } else {
                    step.events.push(Event::Subagent(record.clone()));
                }
            }
        }
    }
    pub fn active(&self) -> bool {
        self.records.iter().any(|(id, a)| {
            a.status.active() && self.claude_tasks.get(id).is_none_or(|task| !task.ambient)
        })
    }
    pub(super) fn invalidate(&mut self, id: &str) {
        self.published.remove(id);
    }
    pub(super) fn register(&mut self, id: &str, parent: &str) -> Option<&mut Agent> {
        if id.is_empty() || (self.records.len() >= 256 && !self.records.contains_key(id)) {
            return None;
        }
        Some(
            self.records
                .entry(id.into())
                .or_insert_with(|| Agent::new(id, parent)),
        )
    }
}
pub(super) fn native_status(value: &str) -> Status {
    match value {
        "pendingInit" | "pending" => Status::Starting,
        "running" | "busy" | "retry" | "inProgress" => Status::Running,
        "completed" => Status::Completed,
        "failed" | "errored" | "error" => Status::Failed,
        "interrupted" | "shutdown" | "stopped" | "killed" => Status::Stopped,
        "idle" => Status::Idle,
        _ => Status::Unknown,
    }
}
fn thread_status(value: &Value) -> Status {
    if value["type"] == "active" {
        let flags = value["activeFlags"].as_array();
        if flags.is_some_and(|f| f.iter().any(|v| v == "waitingOnApproval")) {
            Status::WaitingApproval
        } else if flags.is_some_and(|f| f.iter().any(|v| v == "waitingOnUserInput")) {
            Status::WaitingInput
        } else {
            Status::Running
        }
    } else if value["type"] == "systemError" {
        Status::Failed
    } else {
        native_status(text(value, "type"))
    }
}
impl Protocol {
    pub(super) fn codex_child_events(&mut self, value: &Value) -> Vec<Event> {
        // Control requests continue through the one approval broker owned by the root.
        if value.get("id").is_some() {
            return Vec::new();
        }
        let params = &value["params"];
        let thread = text(params, "threadId");
        let turn = params["turnId"]
            .as_str()
            .or_else(|| params["turn"]["id"].as_str())
            .unwrap_or("");
        let Some(agent) = self.agents.records.get(thread) else {
            return Vec::new();
        };
        if turn.is_empty()
            || (!agent.task_id.starts_with("call:")
                && !agent.task_id.is_empty()
                && agent.task_id != turn)
        {
            return Vec::new();
        }
        let decoder = self
            .agents
            .decoders
            .entry(thread.into())
            .or_insert_with(|| {
                let mut decoder =
                    Protocol::new(Provider::Codex, self.cwd.clone(), String::new(), None);
                decoder.thread_id = Some(thread.into());
                Box::new(decoder)
            });
        decoder.turn_id = Some(turn.into());
        decoder
            .codex(value.clone())
            .events
            .into_iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::Delta { .. }
                        | Event::Text { .. }
                        | Event::ReasoningSummary { .. }
                        | Event::Tool { .. }
                        | Event::ToolInput { .. }
                )
            })
            .map(|event| Event::Child {
                thread_id: thread.into(),
                turn_id: turn.into(),
                event: Box::new(event),
            })
            .collect()
    }

    pub(crate) fn child_command(
        &self,
        command: &crate::subagents::ChildCommand,
    ) -> Result<Value, String> {
        let agent = self
            .agents
            .records
            .get(&command.thread)
            .filter(|agent| {
                self.provider == Provider::Codex
                    && !self.text_only
                    && agent.direct_input == Some(true)
                    && agent.task_id == command.turn
                    && !command.turn.is_empty()
                    && !command.turn.starts_with("call:")
                    && agent.status.active()
            })
            .ok_or_else(|| conduit::i18n::text("subagents.stale_target"))?;
        if let Some(text) = &command.text {
            if text.trim().is_empty()
                || text.len() > 256 * 1024
                || matches!(agent.status, Status::WaitingApproval | Status::WaitingInput)
            {
                return Err(conduit::i18n::text("subagents.stale_target"));
            }
            Ok(
                json!({"method":"turn/steer","params":{"threadId":command.thread,
                "expectedTurnId":command.turn,"input":[{"type":"text","text":text}]}}),
            )
        } else {
            Ok(
                json!({"method":"turn/interrupt","params":{"threadId":command.thread,"turnId":command.turn}}),
            )
        }
    }

    /// Return true only for an owned child's events. Never admit arbitrary other threads.
    pub(super) fn codex_agent_event(&mut self, v: &Value) -> bool {
        if self.text_only {
            return false;
        }
        if let Some(request) = v["id"].as_str()
            && let Some(id) = self.agents.capability_reads.remove(request)
        {
            if let Some(agent) = self.agents.records.get_mut(&id) {
                let thread = &v["result"]["thread"];
                let parent = thread["parentThreadId"].as_str().or_else(|| {
                    thread
                        .pointer("/source/subAgent/thread_spawn/parent_thread_id")
                        .and_then(Value::as_str)
                });
                agent.direct_input = Some(
                    thread["id"].as_str() == Some(id.as_str())
                        && parent == Some(agent.parent_id.as_str())
                        && thread["canAcceptDirectInput"] == true,
                );
            }
            return true;
        }
        if let Some(request) = v["id"].as_str()
            && let Some((id, generation, _)) = self.agents.pending_reads.remove(request)
        {
            if let Some(agent) = self.agents.records.get_mut(&id) {
                if agent.task_id != generation || agent.status.terminal() {
                    return true;
                }
                if v.get("error").is_some() {
                    if agent.status.active() {
                        agent.set_status(Status::Unknown);
                    }
                } else {
                    if let Some(turn) = v["result"]["data"]
                        .as_array()
                        .and_then(|turns| turns.first())
                    {
                        // Persistence can lag the live turn/started event. An
                        // older saved completion cannot finish the current task.
                        if !agent.task_id.is_empty()
                            && !agent.task_id.starts_with("call:")
                            && agent.task_id != text(turn, "id")
                        {
                            return true;
                        }
                        update_turn(
                            agent,
                            turn,
                            self.agents.prior_turns.get(&id).map(String::as_str),
                        );
                    }
                }
            }
            return true;
        }
        // Timed-out observer replies still belong to us, including late RPC errors.
        if v["id"].as_str().is_some_and(|id| {
            id.strip_prefix("conduit-agent-read-")
                .and_then(|sequence| sequence.parse::<u64>().ok())
                .is_some_and(|sequence| sequence > 0 && sequence <= self.agents.read_sequence)
        }) {
            return true;
        }
        let method = text(v, "method");
        let p = &v["params"];
        if method == "thread/started" {
            let thread = &p["thread"];
            let parent = thread["parentThreadId"].as_str().or_else(|| {
                thread
                    .pointer("/source/subAgent/thread_spawn/parent_thread_id")
                    .and_then(Value::as_str)
            });
            if let Some(parent) = parent
                && (self.thread_id.as_deref() == Some(parent)
                    || self.agents.records.contains_key(parent))
                && let Some(agent) = self.agents.register(text(thread, "id"), parent)
            {
                update_thread(agent, thread, None);
            }
            return true;
        }
        let id = text(p, "threadId");
        let child = self.agents.records.contains_key(id);
        let root = self.thread_id.as_deref() == Some(id);
        if root
            && p["turnId"]
                .as_str()
                .zip(self.turn_id.as_deref())
                .is_some_and(|(actual, expected)| actual != expected)
        {
            return false; // The main parser will reject this previous-turn notification too.
        }
        if (root || child)
            && matches!(method, "item/started" | "item/completed")
            && p["item"]["type"] == "collabAgentToolCall"
        {
            let item = &p["item"];
            let operation = text(item, "tool");
            for receiver in item["receiverThreadIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if receiver == self.thread_id.as_deref().unwrap_or("") {
                    continue;
                }
                let key = format!("{}:{receiver}", text(item, "id"));
                let new_task = matches!(
                    operation,
                    "spawnAgent" | "sendInput" | "followupTask" | "resumeAgent"
                ) && self.agents.operations.insert(key);
                if new_task
                    && let Some(agent) = self.agents.records.get(receiver)
                    && !agent.task_id.is_empty()
                {
                    self.agents
                        .prior_turns
                        .insert(receiver.into(), agent.task_id.clone());
                }
                if let Some(agent) = self.agents.register(receiver, id) {
                    if new_task {
                        agent.restart(&format!("call:{}", text(item, "id")));
                        if let Some(prompt) = item["prompt"].as_str() {
                            agent.task = prompt.into();
                        }
                    }
                    if item["status"] == "failed" && !agent.status.terminal() {
                        agent.set_status(Status::Unknown);
                    }
                    if let Some(state) = item["agentsStates"].get(receiver) {
                        let status = native_status(text(state, "status"));
                        // A repeated spawn result must not revive a child already completed.
                        if !agent.status.terminal() {
                            agent.set_status(status);
                        }
                        if let Some(message) = state["message"].as_str() {
                            agent.result = message.into();
                        }
                    }
                }
            }
            return child; // Keep the parent's ordinary tool card as well.
        }
        if !child {
            return false;
        }
        let agent = self.agents.records.get_mut(id).unwrap();
        match method {
            "thread/status/changed" => {
                let status = thread_status(&p["status"]);
                if !agent.status.terminal() {
                    agent.set_status(status);
                }
            }
            "turn/started" => agent.restart(text(&p["turn"], "id")),
            "turn/completed" => {
                let turn = &p["turn"];
                if agent.task_id.is_empty()
                    || agent.task_id == text(turn, "id")
                    || (agent.task_id.starts_with("call:")
                        && self.agents.prior_turns.get(id).map(String::as_str)
                            != Some(text(turn, "id")))
                {
                    agent.task_id = text(turn, "id").into();
                    agent.set_status(native_status(text(turn, "status")));
                    if !turn["error"].is_null() {
                        agent.result = pretty(&turn["error"]);
                    }
                }
            }
            "item/started" => {
                let item = &p["item"];
                if !agent.status.terminal() {
                    agent.status = Status::Running;
                }
                agent.activity = match text(item, "type") {
                    "commandExecution" => text(item, "command"),
                    "mcpToolCall" => text(item, "tool"),
                    other => other,
                }
                .into();
            }
            "item/completed" if p["item"]["type"] == "agentMessage" => {
                agent.result = text(&p["item"], "text").into()
            }
            // Preserve the normal approval broker for child requests, including exact wire IDs.
            _ if v.get("id").is_some() => return false,
            _ => {}
        }
        true
    }
    pub(crate) fn needs_agent_poll(&self) -> bool {
        self.provider == Provider::Codex && !self.agents.records.is_empty()
    }
    pub(crate) fn observe_opencode_agents(&mut self, records: &[Agent]) {
        for record in records {
            self.agents
                .records
                .insert(record.id.clone(), record.clone());
        }
    }
    pub(crate) fn opencode_agent_session(&self) -> Option<&str> {
        (self.provider == Provider::OpenCode)
            .then_some(self.acp.session_id.as_deref())
            .flatten()
    }
    pub(crate) fn poll_subagents(&mut self) -> Vec<Value> {
        if self.provider != Provider::Codex || self.text_only {
            return vec![];
        }
        let now = Instant::now();
        if self
            .agents
            .last_poll
            .is_some_and(|at| now.duration_since(at) < Duration::from_secs(1))
        {
            return vec![];
        }
        self.agents.last_poll = Some(now);
        let timed_out: Vec<_> = self
            .agents
            .pending_reads
            .iter()
            .filter(|(_, (_, _, at))| now.duration_since(*at) > Duration::from_secs(10))
            .map(|(key, (id, generation, _))| (key.clone(), id.clone(), generation.clone()))
            .collect();
        for (key, id, generation) in timed_out {
            self.agents.pending_reads.remove(&key);
            if let Some(agent) = self.agents.records.get_mut(&id)
                && agent.task_id == generation
                && agent.status.active()
            {
                agent.set_status(Status::Unknown);
            }
        }
        let ids: Vec<_> = self.agents.records.keys().cloned().collect();
        let mut writes = vec![];
        for _ in 0..ids.len().min(4) {
            let id = &ids[self.agents.cursor % ids.len()];
            self.agents.cursor = self.agents.cursor.wrapping_add(1);
            if self.agents.records[id].status.terminal()
                || self
                    .agents
                    .pending_reads
                    .values()
                    .any(|(pending, _, _)| pending == id)
            {
                continue;
            }
            self.agents.read_sequence += 1;
            if self.agents.records[id].direct_input.is_none()
                && !self.agents.records[id].task_id.is_empty()
                && !self.agents.records[id].task_id.starts_with("call:")
                && !self
                    .agents
                    .capability_reads
                    .values()
                    .any(|pending| pending == id)
            {
                let key = format!("conduit-agent-capability-{}", self.agents.read_sequence);
                self.agents.capability_reads.insert(key.clone(), id.clone());
                writes.push(json!({"id":key,"method":"thread/read","params":{"threadId":id,"includeTurns":false}}));
            }
            let key = format!("conduit-agent-read-{}", self.agents.read_sequence);
            self.agents.pending_reads.insert(
                key.clone(),
                (id.clone(), self.agents.records[id].task_id.clone(), now),
            );
            writes.insert(
                0,
                json!({"id":key,"method":"thread/turns/list","params":{
                    "threadId":id,"limit":1,"sortDirection":"desc","itemsView":"full"
                }}),
            );
        }
        writes
    }
    pub(super) fn claude_agent_event(&mut self, v: &Value) -> bool {
        if self.text_only {
            return false;
        }
        if v["type"] == "system" {
            let subtype = text(v, "subtype");
            if matches!(
                subtype,
                "task_started" | "task_updated" | "task_progress" | "task_notification"
            ) {
                self.claude_task_event(v);
                return true;
            }
        }
        // Child messages have independent IDs and must not overwrite the main stream cursor.
        !v["parent_tool_use_id"].is_null()
            && matches!(text(v, "type"), "assistant" | "stream_event" | "user")
    }
}
fn update_thread(agent: &mut Agent, thread: &Value, prior_turn: Option<&str>) {
    if let Some(direct) = thread["canAcceptDirectInput"].as_bool() {
        agent.direct_input = Some(direct);
    }
    if let Some(role) = thread["agentRole"].as_str() {
        agent.role = role.into();
    }
    if let Some(name) = thread["agentNickname"]
        .as_str()
        .or_else(|| thread["agentRole"].as_str())
    {
        agent.name = name.into();
    }
    if let Some(turn) = thread["turns"].as_array().and_then(|turns| turns.last()) {
        update_turn(agent, turn, prior_turn);
    }
    if !agent.status.terminal() {
        agent.set_status(thread_status(&thread["status"]));
    }
}

fn update_turn(agent: &mut Agent, turn: &Value, prior_turn: Option<&str>) {
    let id = text(turn, "id");
    if agent.task_id.starts_with("call:") && prior_turn == Some(id) {
        return;
    }
    let turn_status = native_status(text(turn, "status"));
    let waiting = matches!(agent.status, Status::WaitingApproval | Status::WaitingInput);
    if turn_status.active() && !waiting {
        agent.restart(id);
    }
    if turn_status.terminal() || (agent.task_id == id && !agent.status.terminal()) {
        agent.task_id = id.into();
        // A saved turn only knows inProgress, not the live thread's blocking flags.
        if !waiting || turn_status != Status::Running {
            agent.set_status(turn_status);
        }
    } else if waiting && turn_status == Status::Running && agent.task_id.starts_with("call:") {
        agent.task_id = id.into();
    }
    if !turn["error"].is_null() {
        agent.result = pretty(&turn["error"]);
    }
    for item in turn["items"].as_array().into_iter().flatten() {
        match text(item, "type") {
            "agentMessage" => agent.result = text(item, "text").into(),
            "commandExecution" => agent.activity = text(item, "command").into(),
            "mcpToolCall" => agent.activity = text(item, "tool").into(),
            "fileChange" => agent.activity = conduit::i18n::text("tools.edit_code"),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn codex() -> Protocol {
        let mut protocol = Protocol::new(Provider::Codex, "C:/work".into(), "inspect".into(), None);
        protocol.thread_id = Some("root".into());
        protocol.turn_id = Some("main-turn".into());
        protocol
    }
    fn spawn(id: &str) -> Value {
        json!({"method":"item/completed","params":{"threadId":"root","turnId":"main-turn","item":{
            "id":format!("spawn-{id}"),"type":"collabAgentToolCall","tool":"spawnAgent","status":"completed",
            "receiverThreadIds":[id],"senderThreadId":"root","prompt":"inspect code","agentsStates":{id:{"status":"running"}}
        }}})
    }
    #[test]
    fn child_observer_rpc_error_does_not_finish_main_turn() {
        let mut p = codex();
        p.accept(spawn("a"));
        let writes = p.poll_subagents();
        let step = p.accept(
            json!({"id":writes[0]["id"],"error":{"code":-1,"message":"observer unavailable"}}),
        );
        assert!(!step.done);
        assert!(step.error.is_none());
        assert!(step.events.iter().any(
            |event| matches!(event, Event::Subagent(agent) if agent.status == Status::Unknown)
        ));
        let reply = p.accept(json!({"method":"item/agentMessage/delta","params":{"threadId":"root","turnId":"main-turn","itemId":"reply","delta":"still working"}}));
        assert!(
            reply
                .events
                .iter()
                .any(|event| matches!(event, Event::Delta { text, .. } if text == "still working"))
        );
    }
    #[test]
    fn subagent_spawn_completion_is_not_task_completion_and_main_waits_for_all_children() {
        let mut p = codex();
        let mut previous = spawn("old-child");
        previous["params"]["turnId"] = json!("previous-turn");
        p.accept(previous);
        assert!(!p.agents.records.contains_key("old-child"));
        p.accept(spawn("a"));
        p.accept(spawn("b"));
        p.accept(json!({"method":"thread/started","params":{"thread":{
            "id":"a","parentThreadId":"root","agentNickname":"Ada","agentRole":"reviewer",
            "status":{"type":"active","activeFlags":[]}
        }}}));
        assert_eq!(p.agents.records["a"].name, "Ada");
        assert_eq!(p.agents.records["a"].role, "reviewer");
        assert!(p.agents.records["a"].title.is_empty());
        assert_eq!(p.agents.records["a"].status, Status::Running);
        let done = p.accept(json!({"method":"turn/completed","params":{"threadId":"root","turn":{"id":"main-turn","status":"completed"}}}));
        assert!(!done.done);
        let done = p.accept(json!({"method":"turn/completed","params":{"threadId":"a","turn":{"id":"a-turn","status":"completed"}}}));
        assert!(!done.done);
        assert_eq!(p.agents.records["a"].status, Status::Completed);
        // Duplicate spawn snapshots cannot resurrect completed work.
        p.accept(spawn("a"));
        assert_eq!(p.agents.records["a"].status, Status::Completed);
        p.accept(json!({"method":"item/completed","params":{"threadId":"root","turnId":"main-turn","item":{"id":"close-a","type":"collabAgentToolCall","tool":"closeAgent","status":"completed","receiverThreadIds":["a"],"agentsStates":{"a":{"status":"shutdown"}}}}}));
        assert_eq!(p.agents.records["a"].status, Status::Completed);
        p.accept(json!({"method":"thread/status/changed","params":{"threadId":"a","status":{"type":"active","activeFlags":[]}}}));
        assert_eq!(p.agents.records["a"].status, Status::Completed);
        let done = p.accept(json!({"method":"turn/completed","params":{"threadId":"b","turn":{"id":"b-turn","status":"failed","error":{"message":"failure"}}}}));
        assert!(done.done);
        assert_eq!(p.agents.records["b"].status, Status::Failed);
    }
    #[test]
    fn subagent_streams_are_isolated_and_approval_requests_keep_their_wire_identity() {
        let mut p = codex();
        p.accept(spawn("a"));
        for thread in ["a", "unrelated"] {
            let step = p.accept(json!({"method":"item/agentMessage/delta","params":{"threadId":thread,"turnId":"child-turn","itemId":"text","delta":"child output"}}));
            assert!(
                !step
                    .events
                    .iter()
                    .any(|event| matches!(event, Event::Delta { .. } | Event::Text { .. }))
            );
        }
        let step = p.accept(json!({"id":55,"method":"item/commandExecution/requestApproval","params":{"threadId":"a","turnId":"child-turn","itemId":"tool","command":"cargo test"}}));
        assert!(
            step.events
                .iter()
                .any(|event| matches!(event, Event::Approval(a) if a.wire_id == 55))
        );
        p.accept(json!({"method":"thread/status/changed","params":{"threadId":"a","status":{"type":"active","activeFlags":["waitingOnApproval"]}}}));
        assert_eq!(p.agents.records["a"].status, Status::WaitingApproval);
    }
    #[test]
    fn subagent_read_poll_completes_without_native_child_subscription() {
        let mut p = codex();
        p.accept(spawn("a"));
        let writes = p.poll_subagents();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0]["method"], "thread/turns/list");
        assert_eq!(
            writes[0]["params"],
            json!({"threadId":"a","limit":1,"sortDirection":"desc","itemsView":"full"})
        );
        let step = p.accept(json!({"id":writes[0]["id"],"result":{"data":[{"id":"child-turn","status":"completed","items":[{"type":"agentMessage","text":"verified"}]}]}}));
        assert!(step.events.iter().any(|event| matches!(event, Event::Subagent(a) if a.status == Status::Completed && a.result == "verified")));
    }
    #[test]
    fn subagent_old_read_cannot_complete_a_new_task() {
        let mut p = codex();
        p.accept(spawn("a"));
        let writes = p.poll_subagents();
        let mut next = spawn("a");
        next["params"]["item"]["id"] = json!("send-next");
        next["params"]["item"]["tool"] = json!("sendInput");
        p.accept(next);
        p.accept(
            json!({"id":writes[0]["id"],"result":{"data":[{"id":"old","status":"completed"}]}}),
        );
        assert_eq!(p.agents.records["a"].status, Status::Running);
    }
    #[test]
    fn stale_pages_cannot_complete_a_different_turn_or_revive_a_completed_child() {
        let mut p = codex();
        p.accept(spawn("a"));
        p.accept(
            json!({"method":"turn/started","params":{"threadId":"a","turn":{"id":"current"}}}),
        );
        let writes = p.poll_subagents();
        p.accept(
            json!({"id":writes[0]["id"],"result":{"data":[{"id":"old","status":"completed"}]}}),
        );
        assert_eq!(p.agents.records["a"].status, Status::Running);
        p.agents.last_poll = None;
        let writes = p.poll_subagents();
        p.accept(json!({"method":"turn/completed","params":{"threadId":"a","turn":{"id":"current","status":"completed"}}}));
        p.accept(json!({"id":writes[0]["id"],"result":{"data":[{"id":"current","status":"inProgress"}]}}));
        assert_eq!(p.agents.records["a"].status, Status::Completed);
    }
    fn expire_reads(p: &mut Protocol) {
        for (_, _, at) in p.agents.pending_reads.values_mut() {
            *at = Instant::now() - Duration::from_secs(11);
        }
        p.agents.last_poll = None;
    }
    #[test]
    fn subagent_poll_timeout_preserves_terminal_states_and_new_generations() {
        for status in ["completed", "failed", "interrupted"] {
            let mut p = codex();
            p.accept(spawn("a"));
            let writes = p.poll_subagents();
            p.accept(json!({"method":"turn/completed","params":{"threadId":"a","turn":{"id":"child-turn","status":status}}}));
            expire_reads(&mut p);
            assert!(p.poll_subagents().is_empty());
            assert_eq!(p.agents.records["a"].status, native_status(status));
            let late =
                p.accept(json!({"id":writes[0]["id"],"error":{"message":"late observer failure"}}));
            assert!(!late.done);
            assert!(late.error.is_none());
        }
        let mut p = codex();
        p.accept(spawn("a"));
        p.poll_subagents();
        p.accept(
            json!({"method":"turn/started","params":{"threadId":"a","turn":{"id":"new-turn"}}}),
        );
        p.accept(json!({"method":"turn/completed","params":{"threadId":"root","turn":{"id":"main-turn","status":"completed"}}}));
        expire_reads(&mut p);
        let writes = p.poll_subagents();
        assert_eq!(
            writes
                .iter()
                .filter(|request| request["method"] == "thread/turns/list")
                .count(),
            1
        );
        assert_eq!(
            writes
                .iter()
                .filter(|request| request["method"] == "thread/read")
                .count(),
            1
        );
        assert_eq!(p.agents.records["a"].status, Status::Running);
        assert!(!p.accept(json!({"method":"conduit/agentPoll"})).done);
        expire_reads(&mut p);
        p.poll_subagents();
        assert_eq!(
            p.agents.records["a"].status,
            Status::Unknown,
            "a current, unresponsive observer still becomes unknown"
        );
    }
    #[test]
    fn subagent_poll_preserves_live_waiting_flags_until_resume_or_completion() {
        for (flag, status) in [
            ("waitingOnApproval", Status::WaitingApproval),
            ("waitingOnUserInput", Status::WaitingInput),
        ] {
            let mut p = codex();
            p.accept(spawn("a"));
            let writes = p.poll_subagents();
            p.accept(json!({"method":"thread/status/changed","params":{"threadId":"a","status":{"type":"active","activeFlags":[flag]}}}));
            p.accept(json!({"id":writes[0]["id"],"result":{"data":[{"id":"child-turn","status":"inProgress"}]}}));
            assert_eq!(p.agents.records["a"].status, status);
            assert_eq!(p.agents.records["a"].task_id, "child-turn");
            p.agents.last_poll = None;
            let writes = p.poll_subagents();
            p.accept(json!({"id":writes[0]["id"],"result":{"data":[{"id":"child-turn","status":"inProgress"}]}}));
            assert_eq!(p.agents.records["a"].status, status);
            p.accept(json!({"method":"thread/status/changed","params":{"threadId":"a","status":{"type":"active","activeFlags":[]}}}));
            assert_eq!(p.agents.records["a"].status, Status::Running);
            p.accept(json!({"method":"thread/status/changed","params":{"threadId":"a","status":{"type":"active","activeFlags":[flag]}}}));
            p.agents.last_poll = None;
            let writes = p.poll_subagents();
            p.accept(json!({"id":writes[0]["id"],"result":{"data":[{"id":"child-turn","status":"completed"}]}}));
            assert_eq!(p.agents.records["a"].status, Status::Completed);
        }
    }
    #[test]
    fn subagent_claude_progress_end_and_out_of_order_start() {
        let mut p = Protocol::new(Provider::Claude, "C:/work".into(), "inspect".into(), None);
        p.accept(json!({"type":"system","session_id":"root","subtype":"task_started","task_id":"shell","task_type":"local_bash"}));
        assert!(!p.agents.claude_tasks["shell"].agent);
        p.accept(json!({"type":"system","session_id":"root","subtype":"task_notification","task_id":"shell","status":"completed"}));
        p.accept(json!({"type":"system","session_id":"root","subtype":"task_started","task_id":"a","task_type":"local_agent","description":"review","subagent_type":"Explore"}));
        assert_eq!(p.agents.records["a"].title, "review");
        assert_eq!(p.agents.records["a"].role, "Explore");
        p.accept(json!({"type":"system","session_id":"root","subtype":"task_progress","task_id":"a","last_tool_name":"Read","usage":{"duration_ms":42}}));
        assert_eq!(p.agents.records["a"].activity, "Read");
        assert_eq!(p.agents.records["a"].elapsed_ms, Some(42));
        assert!(
            !p.accept(json!({"type":"result","subtype":"success","session_id":"root"}))
                .done
        );
        let done = p.accept(json!({"type":"system","session_id":"root","subtype":"task_notification","task_id":"a","status":"completed","summary":"ok"}));
        assert!(done.done);
        p.accept(json!({"type":"system","session_id":"root","subtype":"task_started","task_id":"a","task_type":"local_agent"}));
        assert_eq!(p.agents.records["a"].status, Status::Completed);
    }
    #[test]
    fn subagent_claude_child_stream_does_not_replace_main_cursor() {
        let mut p = Protocol::new(Provider::Claude, "C:/work".into(), "inspect".into(), None);
        p.accept(
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"main"}}}),
        );
        let child = p.accept(json!({"type":"stream_event","parent_tool_use_id":"task","event":{"type":"message_start","message":{"id":"child"}}}));
        assert!(child.events.is_empty());
        assert_eq!(p.claude_message, "main");
        let main = p.accept(json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"main reply"}}}));
        assert!(
            main.events
                .iter()
                .any(|event| matches!(event, Event::Delta {key,..} if key.starts_with("main-")))
        );
    }
}

#[cfg(test)]
mod child_control_tests {
    use super::*;
    #[test]
    fn commands_require_owned_capable_current_child_and_do_not_retarget() {
        let mut p = Protocol::new(Provider::Codex, "C:/work".into(), String::new(), None);
        p.thread_id = Some("parent".into());
        let agent = p.agents.register("child", "parent").unwrap();
        agent.task_id = "turn".into();
        agent.status = Status::Running;
        let mut command = crate::subagents::ChildCommand {
            thread: "child".into(),
            turn: "turn".into(),
            text: Some("follow up".into()),
        };
        assert!(p.child_command(&command).is_err());
        p.agents.records.get_mut("child").unwrap().direct_input = Some(true);
        let request = p.child_command(&command).unwrap();
        assert_eq!(request["method"], "turn/steer");
        assert_eq!(request["params"]["expectedTurnId"], "turn");
        command.turn = "stale".into();
        assert!(p.child_command(&command).is_err());
        command.turn = "turn".into();
        command.text = None;
        let request = p.child_command(&command).unwrap();
        assert_eq!(request["method"], "turn/interrupt");
        assert_eq!(request["params"]["threadId"], "child");
        command.thread = "parent".into();
        assert!(p.child_command(&command).is_err());
    }
    #[test]
    fn child_stream_keeps_native_identity_and_does_not_change_parent_cursor() {
        let mut p = Protocol::new(Provider::Codex, "C:/work".into(), String::new(), None);
        p.thread_id = Some("parent".into());
        p.turn_id = Some("parent-turn".into());
        p.agents
            .register("child", "parent")
            .unwrap()
            .restart("child-turn");
        let step = p.accept(json!({"method":"item/agentMessage/delta","params":{"threadId":"child","turnId":"child-turn","itemId":"item","delta":"hello"}}));
        assert!(step.events.iter().any(|event| matches!(event, Event::Child { thread_id, turn_id, event } if thread_id == "child" && turn_id == "child-turn" && matches!(event.as_ref(), Event::Delta {text,..} if text == "hello"))));
        assert_eq!(p.turn_id.as_deref(), Some("parent-turn"));
        assert!(!step.done);
        let stale = p.accept(json!({"method":"item/agentMessage/delta","params":{"threadId":"child","turnId":"old-turn","itemId":"item","delta":"late"}}));
        assert!(
            !stale
                .events
                .iter()
                .any(|event| matches!(event, Event::Child { .. } | Event::Delta { .. }))
        );
    }
}
