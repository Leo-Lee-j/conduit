use super::*;

fn claude() -> Protocol {
    Protocol::new(Provider::Claude, "C:/work".into(), "inspect".into(), None)
}

fn start(id: &str, kind: &str, tool: &str, run: &str) -> Value {
    json!({"type":"system","session_id":"root","subtype":"task_started",
        "task_id":id,"task_type":kind,"tool_use_id":tool,"run_id":run,"description":"inspect"})
}

fn tool_state(step: &Step, key: &str) -> Option<ToolStatus> {
    step.events.iter().rev().find_map(|event| match event {
        Event::Tool {
            key: actual,
            status,
            ..
        } if actual == key => Some(status.clone()),
        _ => None,
    })
}

fn agent_input(key: &str, prompt: &str) -> Value {
    json!({"type":"assistant","message":{"id":format!("message-{key}"),"content":[{
        "type":"tool_use","id":key,"name":"Agent",
        "input":{"prompt":prompt,"description":"Review code","subagent_type":"Explore"}
    }]}})
}

#[test]
fn claude_task_prompt_joins_inputs_in_either_order_and_survives_description_updates() {
    for input_first in [false, true] {
        let mut p = claude();
        let input = agent_input("tool", "检查代码\nPreserve this complete task.");
        let started = start("agent", "local_agent", "tool", "run-1");
        let (first, second) = if input_first {
            (input.clone(), started)
        } else {
            (started, input.clone())
        };
        p.accept(first);
        let step = p.accept(second);
        let record = &p.agents.records["agent"];
        assert_eq!(record.task, "检查代码\nPreserve this complete task.");
        assert_eq!(record.title, "inspect");
        assert_eq!(record.role, "Explore");
        assert!(
            step.events
                .iter()
                .any(|event| matches!(event, Event::Subagent(a) if a == record))
        );
        assert!(
            !p.accept(input)
                .events
                .iter()
                .any(|event| matches!(event, Event::Subagent(_)))
        );
        p.accept(json!({"type":"system","subtype":"task_updated","task_id":"agent","run_id":"run-1","patch":{"description":"New short title"}}));
        assert_eq!(p.agents.records["agent"].title, "New short title");
        assert_eq!(
            p.agents.records["agent"].task,
            "检查代码\nPreserve this complete task."
        );
    }
}

#[test]
fn claude_late_task_input_enriches_completion_without_reviving_it_or_crossing_runs() {
    let mut p = claude();
    p.accept(start("agent", "local_agent", "tool-1", "run-1"));
    p.accept(json!({"type":"system","subtype":"task_notification","task_id":"agent","run_id":"run-1","status":"completed","summary":"done"}));
    let finished_at = p.agents.records["agent"].finished_at;
    let step = p.accept(agent_input("tool-1", "Original task"));
    assert_eq!(tool_state(&step, "tool-1"), Some(ToolStatus::Completed));
    let record = &p.agents.records["agent"];
    assert_eq!(record.task, "Original task");
    assert_eq!(record.result, "done");
    assert_eq!(record.finished_at, finished_at);

    let mut restarted = start("agent", "local_agent", "tool-2", "run-2");
    restarted.as_object_mut().unwrap().remove("description");
    p.accept(restarted);
    assert!(p.agents.records["agent"].task.is_empty());
    p.accept(agent_input("tool-1", "Stale task"));
    assert!(p.agents.records["agent"].task.is_empty());
    p.accept(agent_input("tool-2", "Next task"));
    assert_eq!(p.agents.records["agent"].task, "Next task");
    assert_eq!(p.agents.records["agent"].status, Status::Running);
}

#[test]
fn claude_task_input_cache_is_bounded_without_erasing_correlated_prompts() {
    let mut p = claude();
    p.accept(start("agent", "local_agent", "tool", "run-1"));
    p.accept(agent_input("tool", &"中😀".repeat(2000)));
    let prompt = p.agents.records["agent"].task.clone();
    assert!(prompt.len() <= 8192);
    assert!(prompt.len() >= 8185);
    for index in 0..MAX_TASK_INPUTS + 1 {
        p.accept(agent_input(&format!("pending-{index}"), "Pending"));
    }
    assert_eq!(p.agents.claude_inputs.len(), MAX_TASK_INPUTS);
    assert!(!p.agents.claude_inputs.iter().any(|(key, _)| key == "tool"));
    p.accept(start("agent", "local_agent", "tool", "run-1"));
    assert_eq!(p.agents.records["agent"].task, prompt);
}

#[test]
fn claude_task_inputs_reject_unrelated_tools_and_child_transcripts() {
    for kind in ["Bash", "mcp__custom__Agent"] {
        let mut p = claude();
        let mut input = agent_input("tool", "Not an agent task");
        input["message"]["content"][0]["name"] = json!(kind);
        p.accept(input);
        p.accept(start("agent", "local_agent", "tool", "run-1"));
        assert_eq!(p.agents.records["agent"].task, "inspect");
        assert!(p.agents.claude_inputs.is_empty());
    }
    let mut p = claude();
    let mut input = agent_input("nested-tool", "Private child transcript");
    input["parent_tool_use_id"] = json!("parent-tool");
    assert!(p.accept(input).events.is_empty());
    assert!(p.agents.claude_inputs.is_empty());
    let mut p = claude().text_only();
    p.accept(agent_input("tool", "Do not retain in naming requests"));
    assert!(p.agents.claude_inputs.is_empty());
}

#[test]
fn claude_structured_launch_and_legacy_task_inputs_preserve_original_prompt() {
    let mut p = claude();
    let mut input = agent_input("tool", "Legacy complete prompt");
    input["message"]["content"][0]["name"] = json!("Task");
    p.accept(input);
    p.accept(json!({"type":"user","tool_use_result":{"status":"async_launched","agentId":"agent"},"message":{"content":[{"type":"tool_result","tool_use_id":"tool","content":"launched"}]}}));
    assert_eq!(p.agents.records["agent"].task, "Legacy complete prompt");
    p.accept(start("agent", "local_agent", "tool", "run-1"));
    p.accept(json!({"type":"assistant","message":{"id":"partial","content":[{"type":"tool_use","id":"tool","name":"Task","input":{}}]}}));
    assert_eq!(p.agents.records["agent"].task, "Legacy complete prompt");
}

#[test]
fn claude_background_task_return_is_not_completion_and_notifications_update_the_tool() {
    for (native, expected) in [
        ("completed", ToolStatus::Completed),
        ("failed", ToolStatus::Failed),
        ("stopped", ToolStatus::Cancelled),
    ] {
        let mut p = claude();
        let started = p.accept(start("shell", "local_bash", "bash-tool", "run-1"));
        assert_eq!(tool_state(&started, "bash-tool"), Some(ToolStatus::Running));
        assert!(
            !started
                .events
                .iter()
                .any(|event| matches!(event, Event::Subagent(_)))
        );
        let returned = p.accept(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"bash-tool","content":"launched"}]}}));
        assert_eq!(
            tool_state(&returned, "bash-tool"),
            Some(ToolStatus::Running)
        );
        assert!(!p.accept(json!({"type":"result","subtype":"success"})).done);
        let progress = p.accept(json!({"type":"system","subtype":"task_progress","task_id":"shell","run_id":"run-1","summary":"still testing","usage":{"duration_ms":42}}));
        assert_eq!(
            tool_state(&progress, "bash-tool"),
            Some(ToolStatus::Running)
        );
        assert_eq!(p.agents.records["shell"].elapsed_ms, Some(42));
        let done = p.accept(json!({"type":"system","subtype":"task_notification","task_id":"shell","run_id":"run-1","status":native,"summary":"final result"}));
        assert!(done.done);
        assert_eq!(tool_state(&done, "bash-tool"), Some(expected.clone()));
        let duplicate = p.accept(start("shell", "local_bash", "bash-tool", "run-1"));
        assert!(duplicate.done);
        let replay = p.accept(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"bash-tool","content":"old launch result"}]}}));
        assert_eq!(tool_state(&replay, "bash-tool"), Some(expected));
        assert!(
            replay
                .events
                .iter()
                .any(|event| matches!(event, Event::Tool {detail, ..} if detail == "final result"))
        );
    }
}

#[test]
fn claude_task_runs_are_ordered_and_subagent_cards_share_the_native_tool_identity() {
    let mut p = claude();
    p.accept(start("agent", "local_agent", "tool-1", "run-1"));
    p.accept(json!({"type":"system","subtype":"task_notification","task_id":"agent","run_id":"run-1","status":"completed","summary":"old result"}));
    let restarted = p.accept(start("agent", "local_agent", "tool-2", "run-2"));
    assert_eq!(tool_state(&restarted, "tool-2"), Some(ToolStatus::Running));
    assert!(restarted.events.iter().any(|event| matches!(event, Event::Subagent(a) if a.task_id == "run-2" && a.status == Status::Running && a.result.is_empty())));
    for late in [
        start("agent", "local_agent", "tool-1", "run-1"),
        json!({"type":"system","subtype":"task_notification","task_id":"agent","run_id":"run-1","status":"failed","summary":"late failure"}),
        json!({"type":"system","subtype":"task_updated","task_id":"agent","run_id":"run-1","patch":{"status":"killed"}}),
        json!({"type":"system","subtype":"task_progress","task_id":"agent","run_id":"run-1","summary":"old progress"}),
    ] {
        assert!(p.accept(late).events.is_empty());
        assert_eq!(p.agents.records["agent"].status, Status::Running);
        assert!(p.agents.records["agent"].result.is_empty());
    }
    p.accept(json!({"type":"system","subtype":"task_updated","task_id":"agent","run_id":"run-2","patch":{"description":"new task","status":"running"}}));
    assert_eq!(p.agents.records["agent"].task, "new task");
    let done = p.accept(json!({"type":"system","subtype":"task_notification","task_id":"agent","run_id":"run-2","status":"completed","summary":"new result"}));
    assert_eq!(tool_state(&done, "tool-2"), Some(ToolStatus::Completed));
    let snapshot = p.accept(json!({"type":"assistant","message":{"id":"main","content":[{"type":"tool_use","id":"tool-2","name":"Agent","input":{}}]}}));
    assert_eq!(tool_state(&snapshot, "tool-2"), Some(ToolStatus::Completed));
}

#[test]
fn claude_structured_background_results_register_tasks_without_parsing_placeholder_text() {
    for (output, agent) in [
        (json!({"backgroundTaskId":"task"}), false),
        (json!({"status":"async_launched","agentId":"task"}), true),
    ] {
        let mut p = claude();
        let step = p.accept(json!({"type":"user","tool_use_result":output,"message":{"content":[{"type":"tool_result","tool_use_id":"tool","content":"launched"}]}}));
        assert_eq!(tool_state(&step, "tool"), Some(ToolStatus::Running));
        assert_eq!(p.agents.claude_tasks["task"].agent, agent);
        assert!(!p.accept(json!({"type":"result","subtype":"success"})).done);
        p.accept(start(
            "task",
            if agent { "local_agent" } else { "local_bash" },
            "tool",
            "run-1",
        ));
        let done = p.accept(json!({"type":"system","subtype":"task_notification","task_id":"task","run_id":"run-1","status":"completed","summary":"ok"}));
        assert!(done.done);
        assert_eq!(tool_state(&done, "tool"), Some(ToolStatus::Completed));
    }
    let mut p = claude();
    p.accept(json!({"type":"user","tool_use_result":{"backgroundTaskId":"task","backgroundEndsWithFinalResponse":true},"message":{"content":[{"type":"tool_result","tool_use_id":"tool","content":"launched"}]}}));
    let done = p.accept(json!({"type":"result","subtype":"success"}));
    assert!(
        done.done,
        "CLI explicitly says no completion notification will follow"
    );
    assert_eq!(tool_state(&done, "tool"), Some(ToolStatus::Cancelled));
}

#[test]
fn claude_ambient_tasks_do_not_block_and_nested_tasks_do_not_enter_the_main_transcript() {
    let mut p = claude();
    p.accept(start("parent", "local_agent", "parent-tool", "run-1"));
    let mut child = start("nested-shell", "local_bash", "nested-tool", "run-1");
    child["parent_task_id"] = json!("parent");
    let step = p.accept(child);
    assert!(
        !step
            .events
            .iter()
            .any(|event| matches!(event, Event::Tool { .. }))
    );
    let mut ambient = start("watcher", "local_bash", "watcher-tool", "run-1");
    ambient["ambient"] = json!(true);
    assert!(p.accept(ambient).events.is_empty());
    p.accept(json!({"type":"system","subtype":"task_notification","task_id":"parent","run_id":"run-1","status":"completed"}));
    assert!(!p.accept(json!({"type":"result","subtype":"success"})).done);
    let done = p.accept(json!({"type":"system","subtype":"task_notification","task_id":"nested-shell","run_id":"run-1","status":"completed"}));
    assert!(done.done);
    assert!(
        !done
            .events
            .iter()
            .any(|event| matches!(event, Event::Tool {key, ..} if key == "nested-tool"))
    );
}

#[test]
fn claude_task_failure_and_legacy_out_of_order_start_cannot_invent_success() {
    let mut p = claude();
    p.accept(start("shell", "local_bash", "tool", ""));
    let failed = p.accept(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool","content":"launch failed","is_error":true}]}}));
    assert_eq!(tool_state(&failed, "tool"), Some(ToolStatus::Failed));
    p.accept(start("shell", "local_bash", "tool", ""));
    assert_eq!(p.agents.records["shell"].status, Status::Failed);
    let mut p = claude();
    p.accept(json!({"type":"system","subtype":"task_notification","task_id":"a","status":"completed","summary":"done"}));
    let started = p.accept(start("a", "local_agent", "tool", ""));
    assert!(
        started
            .events
            .iter()
            .any(|event| matches!(event, Event::Subagent(a) if a.status == Status::Completed))
    );
    assert_eq!(tool_state(&started, "tool"), Some(ToolStatus::Completed));
    let mut p = claude().text_only();
    assert!(
        !p.accept(start("shell", "local_bash", "tool", "run-1"))
            .events
            .iter()
            .any(|event| matches!(event, Event::Tool { .. } | Event::Subagent(_)))
    );
    assert!(p.agents.records.is_empty());
}

#[test]
fn claude_foreground_task_results_finish_but_background_transitions_keep_running() {
    for background in [false, true] {
        let mut p = claude();
        let mut value = start("shell", "local_bash", "tool", "run-1");
        value["is_backgrounded"] = json!(false);
        p.accept(value);
        if background {
            p.accept(json!({"type":"system","subtype":"task_updated","task_id":"shell","run_id":"run-1","patch":{"is_backgrounded":true}}));
        }
        let returned = p.accept(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool","content":"result"}]}}));
        assert_eq!(
            tool_state(&returned, "tool"),
            Some(if background {
                ToolStatus::Running
            } else {
                ToolStatus::Completed
            })
        );
        assert_eq!(
            p.accept(json!({"type":"result","subtype":"success"})).done,
            !background
        );
    }
}
