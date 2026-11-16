//! OpenCode-specific startup and authenticated question transport.
use super::*;
use crate::{
    domain::Provider,
    protocol::{ApprovalKind, Protocol},
};
use anyhow::Context;
mod agents;
mod questions;
use questions::QuestionBridge;

pub const COMMAND_SOURCE: crate::commands::Source = crate::commands::Source {
    id: "opencode",
    registration: 1,
};

pub fn opencode(request: AdapterRequest) -> Box<dyn CliAdapter> {
    let cwd = request.cwd.clone();
    Box::new(OpenCodeAdapter::new(
        builtin::protocol(Provider::OpenCode, request),
        cwd,
    ))
}

pub(super) struct OpenCodeAdapter {
    core: builtin::StdioAdapter,
    cwd: PathBuf,
    questions: Option<QuestionBridge>,
    agent_monitor: Option<agents::Monitor>,
    agent_records: Vec<crate::subagents::Agent>,
    main_done: bool,
    main_done_at: u64,
    agent_questions: BTreeMap<String, String>,
}

impl OpenCodeAdapter {
    fn start_agent_monitor(&mut self) {
        if self.agent_monitor.is_none()
            && let Some(session) = self.core.protocol.opencode_question_session()
            && let Some((port, password)) = self.core.protocol.opencode_question_connection()
        {
            self.agent_monitor = Some(agents::Monitor::start(port, password, &self.cwd, session));
        }
    }
    pub fn new(protocol: Protocol, cwd: PathBuf) -> Self {
        Self {
            core: builtin::StdioAdapter::new(protocol),
            cwd,
            questions: None,
            agent_monitor: None,
            agent_records: vec![],
            main_done: false,
            main_done_at: 0,
            agent_questions: BTreeMap::new(),
        }
    }
}

impl CliAdapter for OpenCodeAdapter {
    fn launch(&mut self) -> anyhow::Result<LaunchSpec> {
        let launch = self.core.launch()?;
        self.questions = self
            .core
            .protocol
            .opencode_question_connection()
            .map(|(port, password)| QuestionBridge::new(port, password, &self.cwd));
        Ok(launch)
    }
    fn accept(&mut self, value: Value) -> Step {
        if value["method"] == "conduit/opencodeAgents" {
            let mut step = Step::new();
            if let Ok(mut records) =
                serde_json::from_value::<Vec<crate::subagents::Agent>>(value["params"].clone())
            {
                for agent in &mut records {
                    if self.agent_questions.values().any(|id| *id == agent.id) {
                        agent.status = crate::subagents::Status::WaitingInput;
                    }
                }
                for agent in &records {
                    if !self.agent_records.contains(agent) {
                        step.events
                            .push(crate::protocol::Event::Subagent(agent.clone()));
                    }
                }
                self.core.protocol.observe_opencode_agents(&records);
                self.agent_records = records;
            }
            step.done = self.main_done
                && value["observed_at"]
                    .as_u64()
                    .is_some_and(|at| at > self.main_done_at)
                && !self.agent_records.iter().any(|agent| agent.status.active());
            return step;
        }
        if value["method"] == "session/update"
            && value["params"]["update"]["sessionUpdate"] == "available_commands_update"
        {
            let metadata = self
                .questions
                .as_ref()
                .ok_or_else(|| conduit::i18n::text("commands.no_metadata"))
                .and_then(|bridge| {
                    bridge
                        .commands()
                        .map_err(|_| conduit::i18n::text("commands.no_metadata"))
                });
            self.core.protocol.set_command_metadata(metadata);
        }
        let mut question_agent = None;
        if value["method"] == "conduit/question" {
            if let Some(agent) = self
                .agent_records
                .iter_mut()
                .find(|agent| value["params"]["sessionID"] == agent.id)
            {
                if let Some(id) = value["id"].as_str() {
                    self.agent_questions.insert(id.into(), agent.id.clone());
                }
                agent.status = crate::subagents::Status::WaitingInput;
                question_agent = Some(agent.clone());
            }
        } else if value["method"] == "conduit/questionCancelled"
            && let Some(id) = value["params"]["id"].as_str()
        {
            self.agent_questions.remove(id);
        }
        let mut step = self.core.accept(value);
        self.start_agent_monitor();
        if let Some(agent) = question_agent {
            step.events.push(crate::protocol::Event::Subagent(agent));
        }
        if step.done && step.error.is_none() && self.agent_monitor.is_some() {
            self.main_done = true;
            self.main_done_at = crate::subagents::now_ms();
            // Wait for a fresh observer snapshot after the main prompt completes.
            if let Some(monitor) = &self.agent_monitor {
                while monitor.receiver.try_recv().is_ok() {}
            }
            step.done = false;
        }
        step
    }
    fn startup_pending(&self) -> bool {
        self.core.startup_pending()
    }
    fn cancel_message(&self) -> Option<Value> {
        self.core.cancel_message()
    }
    fn poll(&mut self) -> anyhow::Result<Vec<Value>> {
        self.core.protocol.check_command_timeout()?;
        self.start_agent_monitor();
        let mut messages = vec![];
        if let Some(monitor) = &self.agent_monitor {
            while let Ok((observed_at, records)) = monitor.receiver.try_recv() {
                messages
                    .push(serde_json::json!({"method":"conduit/opencodeAgents","observed_at":observed_at,"params":records}));
            }
        }
        let questions = match (
            self.core.protocol.opencode_agent_session(),
            self.questions.as_mut(),
        ) {
            (Some(session), Some(bridge)) => bridge.poll_sessions(session, &self.agent_records),
            _ => Ok(vec![]),
        }?;
        messages.extend(questions);
        Ok(messages)
    }
    fn route_response(
        &mut self,
        approval: &Approval,
        response: Value,
    ) -> anyhow::Result<Option<Value>> {
        if approval.kind == ApprovalKind::OpenCodeUserInput {
            self.questions
                .as_ref()
                .context(conduit::i18n::text_static(
                    "cli_adapter.opencode.opencode_question_connection_unavailable",
                ))?
                .respond(approval, response)?;
            Ok(None)
        } else {
            Ok(Some(response))
        }
    }
    fn automatic_approval(&self, approval: &Approval) -> Option<bool> {
        self.core.automatic_approval(approval)
    }
    fn approval_response(&self, approval: &Approval, allow: bool) -> Value {
        self.core.approval_response(approval, allow)
    }
    fn user_input_response(
        &self,
        approval: &Approval,
        answers: &BTreeMap<String, Vec<String>>,
    ) -> Option<Value> {
        self.core.user_input_response(approval, answers)
    }
    fn enable_full_access(&mut self) {
        self.core.enable_full_access();
    }
}

#[cfg(test)]
mod agent_tests {
    use super::*;
    use crate::subagents::{Agent, Status};
    #[test]
    fn subagent_old_observer_snapshot_cannot_finish_background_work() {
        let protocol = Protocol::new(
            Provider::OpenCode,
            "C:/fixture".into(),
            "hello".into(),
            None,
        );
        let mut adapter = OpenCodeAdapter::new(protocol, PathBuf::from("C:/fixture"));
        adapter.main_done = true;
        adapter.main_done_at = 10;
        assert!(!adapter.accept(serde_json::json!({"method":"conduit/opencodeAgents","observed_at":9,"params":[]})).done);
        let mut agent = Agent::new("child", "parent");
        agent.status = Status::Running;
        assert!(!adapter.accept(serde_json::json!({"method":"conduit/opencodeAgents","observed_at":11,"params":[agent]})).done);
        agent.set_status(Status::Completed);
        assert!(adapter.accept(serde_json::json!({"method":"conduit/opencodeAgents","observed_at":12,"params":[agent]})).done);
    }
}
