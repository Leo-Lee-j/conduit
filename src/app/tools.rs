//! Tool grouping, summaries, output formatting, and file edit previews.

use super::*;

mod diff_cache;
pub(super) use diff_cache::DiffCache;

/// Tool activity collapses into one quiet "执行工具" group. File edits and
/// unresolved or rejected approvals stay outside so their UI keeps its place.
#[cfg(test)]
pub(super) fn contiguous_tool_group(
    session: &SessionRecord,
    index: usize,
) -> Option<(usize, usize)> {
    if !is_groupable_activity(session.messages.get(index)?) {
        return None;
    }
    let mut start = index;
    while start > 0
        && (is_groupable_activity(&session.messages[start - 1])
            || is_empty_chat_placeholder(&session.messages[start - 1]))
    {
        start -= 1;
    }
    while start < index && is_empty_chat_placeholder(&session.messages[start]) {
        start += 1;
    }
    let mut end = index + 1;
    while end < session.messages.len()
        && (is_groupable_activity(&session.messages[end])
            || is_empty_chat_placeholder(&session.messages[end]))
    {
        end += 1;
    }
    while end > index + 1 && is_empty_chat_placeholder(&session.messages[end - 1]) {
        end -= 1;
    }
    Some((start, end))
}

/// Providers can start empty text blocks before issuing tools. Keep their keys
/// for future stream updates, but give them no transcript chrome or spacing.
pub(super) fn is_empty_chat_placeholder(item: &ChatItem) -> bool {
    matches!(item.kind, ItemKind::Assistant | ItemKind::Reasoning) && item.text.trim().is_empty()
}

pub(super) fn is_groupable_activity(item: &ChatItem) -> bool {
    // Thinking has its own disclosure control and must remain discoverable
    // after the surrounding tool groups collapse when a reply arrives.
    is_compact_tool(item)
}

/// Approval history must not permanently change a normal tool's layout.
/// Keep pending and unsuccessful decisions visible outside collapsed groups.
pub(super) fn is_compact_tool(item: &ChatItem) -> bool {
    item.kind == ItemKind::Tool
        && !is_tool_edit(item)
        && item.approval.as_ref().is_none_or(|approval| {
            approval.status == crate::message_status::ApprovalStatus::Allowed
        })
}

/// A later turn must not hide the tool history of a turn that never replied.
#[cfg(test)]
pub(super) fn tool_group_has_reply(session: &SessionRecord, end: usize) -> bool {
    session.messages[end..]
        .iter()
        .take_while(|item| item.kind != ItemKind::User)
        .any(|item| item.kind == ItemKind::Assistant && !item.text.trim().is_empty())
}

pub(super) fn is_tool_edit(item: &ChatItem) -> bool {
    #[derive(serde::Deserialize)]
    struct Kind<'a> {
        #[serde(rename = "acpKind", borrow)]
        kind: Option<std::borrow::Cow<'a, str>>,
    }
    matches!(
        item.text.trim(),
        "修改文件" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "apply_patch"
    ) || serde_json::from_str::<Kind<'_>>(&item.detail)
        .is_ok_and(|value| value.kind.as_deref() == Some("edit"))
}

pub(super) fn is_tool_command(item: &ChatItem) -> bool {
    if matches!(
        item.text.trim(),
        "Bash" | "BashOutput" | "exec_command" | "shell" | "shell_command" | "命令输出"
    ) {
        return true;
    }
    serde_json::from_str::<serde_json::Value>(&item.input)
        .ok()
        .is_some_and(|value| {
            value.get("command").is_some()
                && value.get("server").is_none()
                && value.get("tool").is_none()
        })
}

pub(super) fn tool_running(item: &ChatItem) -> bool {
    let status = item.status.trim();
    matches!(status, "运行中" | "输出中")
        || status.eq_ignore_ascii_case("pending")
        || status.eq_ignore_ascii_case("in_progress")
}

pub(super) fn tool_failed(item: &ChatItem) -> bool {
    #[derive(serde::Deserialize)]
    struct Exit {
        #[serde(rename = "exitCode")]
        code: Option<i64>,
        exit_code: Option<i64>,
    }
    if item.approval.as_ref().is_some_and(|approval| {
        matches!(
            approval.status,
            crate::message_status::ApprovalStatus::Denied
                | crate::message_status::ApprovalStatus::Interrupted
        )
    }) {
        return true;
    }
    let status = item.status.trim();
    status.contains("失败")
        || status.contains("错误")
        || matches!(status, "已中断" | "已拒绝")
        || status.eq_ignore_ascii_case("failed")
        || status.eq_ignore_ascii_case("error")
        || status.eq_ignore_ascii_case("aborted")
        || serde_json::from_str::<Exit>(&item.detail)
            .ok()
            .and_then(|value| value.code.or(value.exit_code))
            .is_some_and(|code| code != 0)
}

/// Human category shown before the one-line summary, mirroring Codex's
/// activity rows (terminal command, workspace search, reasoning, ...).
pub(super) fn tool_category(item: &ChatItem) -> String {
    if item.kind == ItemKind::Reasoning {
        return conduit::i18n::text_static("tools.thought").into();
    }
    let title = item.text.trim();
    if is_tool_edit(item) {
        return conduit::i18n::text_static("tools.edit_code").into();
    }
    if is_tool_command(item) {
        return conduit::i18n::text_static("tools.terminal_command").into();
    }
    let lowered = title.to_lowercase();
    match lowered.as_str() {
        "websearch" | "web_search" | "web search" | "search" => {
            conduit::i18n::text_static("tools.web_search").into()
        }
        "webfetch" | "web_fetch" => conduit::i18n::text_static("tools.read_webpage").into(),
        "read" | "read_file" | "readfile" | "view" => {
            conduit::i18n::text_static("tools.read_file").into()
        }
        "grep" | "glob" | "ls" | "list" => {
            conduit::i18n::text_static("tools.search_workspace").into()
        }
        "task" | "agent" => conduit::i18n::text_static("tools.subtask").into(),
        "todowrite" | "todo_write" => conduit::i18n::text_static("tools.plan_task").into(),
        "" => conduit::i18n::text_static("events.tool_call").into(),
        _ if lowered.contains("search") || lowered.contains("grep") => {
            conduit::i18n::text_static("tools.search_workspace").into()
        }
        _ if lowered.contains("read") => conduit::i18n::text_static("tools.read_file").into(),
        _ => title.to_string(),
    }
}

pub(super) fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string()
}

pub(super) fn tool_summary(item: &ChatItem) -> String {
    if item.kind == ItemKind::Reasoning {
        return first_line(&item.text);
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&item.input) {
        for key in [
            "command",
            "query",
            "url",
            "pattern",
            "path",
            "file_path",
            "prompt",
            "description",
            "cmd",
        ] {
            let Some(argument) = value.get(key) else {
                continue;
            };
            let text = match argument {
                serde_json::Value::String(text) => first_line(text),
                serde_json::Value::Array(parts) => parts
                    .iter()
                    .rev()
                    .find_map(serde_json::Value::as_str)
                    .map(first_line)
                    .unwrap_or_default(),
                _ => String::new(),
            };
            if !text.is_empty() {
                return text;
            }
        }
        if let Some(arguments) = value.get("arguments") {
            for key in ["query", "code", "path", "pattern"] {
                if let Some(text) = arguments.get(key).and_then(serde_json::Value::as_str) {
                    let text = first_line(text);
                    if !text.is_empty() {
                        return text;
                    }
                }
            }
        }
    }
    let title = first_line(&item.text);
    if !title.is_empty() && !is_tool_edit(item) {
        return match title.as_str() {
            "命令输出" => conduit::i18n::text("protocol.command_output"),
            _ => title,
        };
    }
    first_line(&item.detail)
}

/// Tool output is literal text, never Markdown or HTML. Strip terminal control
/// sequences and expand tabs so logs, JSON and code remain readable together.
pub(super) fn format_tool_text(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    for ch in chars.by_ref() {
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(ch) = chars.next() {
                        if ch == '\u{7}' {
                            break;
                        }
                        if ch == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                plain.push('\n');
            }
            '\t' => plain.push_str("    "),
            '\n' => plain.push(ch),
            ch if !ch.is_control() => plain.push(ch),
            _ => {}
        }
    }
    if let Ok(value @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) =
        serde_json::from_str(&plain)
    {
        return serde_json::to_string_pretty(&value).unwrap_or(plain);
    }
    plain
}

/// Chat tools that rewrite files carry either a unified patch (Codex) or an
/// old/new pair (Claude). Parse both into a small, bounded line model and
/// synthesize one unified patch for the diff viewer instead of raw JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditLineKind {
    Add,
    Remove,
    Context,
    Hunk,
}

#[derive(Clone)]
pub(super) struct EditLine {
    pub(super) kind: EditLineKind,
    pub(super) text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EditFileStatus {
    Modified,
    Added,
    Deleted,
}

#[derive(Clone)]
pub(super) struct EditFileDiff {
    pub(super) path: String,
    pub(super) note: String,
    /// A unified patch built from the parsed lines, ready for `DiffFile::parse`.
    pub(super) patch: String,
    pub(super) added: usize,
    pub(super) removed: usize,
    pub(super) truncated: bool,
}

pub(super) const MAX_EDIT_DIFF_LINES: usize = 400;
const MAX_EDIT_DIFF_BYTES: usize = 64 * 1024;

/// Count the whole change, but allocate only the displayed prefix.
#[derive(Default)]
pub(super) struct PreviewLines {
    lines: Vec<EditLine>,
    added: usize,
    removed: usize,
    seen: usize,
    bytes: usize,
    truncated: bool,
}

impl PreviewLines {
    fn push(&mut self, kind: EditLineKind, text: &str) {
        self.seen += 1;
        self.added += usize::from(kind == EditLineKind::Add);
        self.removed += usize::from(kind == EditLineKind::Remove);
        if self.lines.len() == MAX_EDIT_DIFF_LINES || self.bytes == MAX_EDIT_DIFF_BYTES {
            self.truncated = true;
            return;
        }
        let mut end = text.len().min(MAX_EDIT_DIFF_BYTES - self.bytes);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.truncated |= end < text.len();
        self.bytes += end;
        self.lines.push(edit_line(kind, &text[..end]));
        if end < text.len() {
            self.bytes = MAX_EDIT_DIFF_BYTES;
        }
    }

    fn text(&mut self, kind: EditLineKind, text: &str) {
        for line in text.lines() {
            self.push(kind, line);
        }
    }

    fn is_empty(&self) -> bool {
        self.seen == 0
    }

    fn finish(self, path: String, note: String, status: EditFileStatus) -> EditFileDiff {
        let mut diff = EditFileDiff::new(path, note, status, self.lines);
        diff.added = self.added;
        diff.removed = self.removed;
        diff.truncated |= self.truncated;
        diff
    }
}

impl EditFileDiff {
    pub(super) fn new(
        path: String,
        note: String,
        status: EditFileStatus,
        mut lines: Vec<EditLine>,
    ) -> Self {
        let truncated = lines.len() > MAX_EDIT_DIFF_LINES;
        lines.truncate(MAX_EDIT_DIFF_LINES);
        let added = lines
            .iter()
            .filter(|line| line.kind == EditLineKind::Add)
            .count();
        let removed = lines
            .iter()
            .filter(|line| line.kind == EditLineKind::Remove)
            .count();
        let patch = if lines.is_empty() {
            String::new()
        } else {
            synthesize_patch(&path, status, &lines)
        };
        Self {
            path,
            note,
            patch,
            added,
            removed,
            truncated,
        }
    }
}

/// Rewrites parsed lines as one unified patch. Real hunk headers keep their
/// original line numbers; generated markers are renumbered from the counters.
fn synthesize_patch(path: &str, status: EditFileStatus, lines: &[EditLine]) -> String {
    let mut patch = String::new();
    if status == EditFileStatus::Added {
        patch.push_str("--- /dev/null\n");
    } else {
        patch.push_str(&format!("--- a/{path}\n"));
    }
    if status == EditFileStatus::Deleted {
        patch.push_str("+++ /dev/null\n");
    } else {
        patch.push_str(&format!("+++ b/{path}\n"));
    }
    let mut old_pos = usize::from(status != EditFileStatus::Added);
    let mut new_pos = usize::from(status != EditFileStatus::Deleted);
    let mut header = None;
    let mut body = Vec::new();
    for line in lines {
        if line.kind == EditLineKind::Hunk {
            flush_hunk(&mut patch, &mut body, &mut old_pos, &mut new_pos, header);
            header = hunk_starts(&line.text);
        } else {
            body.push(line);
        }
    }
    flush_hunk(&mut patch, &mut body, &mut old_pos, &mut new_pos, header);
    patch
}

fn flush_hunk(
    patch: &mut String,
    body: &mut Vec<&EditLine>,
    old_pos: &mut usize,
    new_pos: &mut usize,
    header: Option<(usize, usize)>,
) {
    if body.is_empty() {
        return;
    }
    let old_count = body
        .iter()
        .filter(|line| line.kind != EditLineKind::Add)
        .count();
    let new_count = body
        .iter()
        .filter(|line| line.kind != EditLineKind::Remove)
        .count();
    if old_count == 0 && new_count == 0 {
        body.clear();
        return;
    }
    let (old_start, new_start) = header.unwrap_or((*old_pos, *new_pos));
    patch.push_str(&format!(
        "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
    ));
    for line in body.drain(..) {
        patch.push(match line.kind {
            EditLineKind::Add => '+',
            EditLineKind::Remove => '-',
            _ => ' ',
        });
        patch.push_str(&line.text);
        patch.push('\n');
    }
    *old_pos = old_start + old_count;
    *new_pos = new_start + new_count;
}

fn hunk_starts(text: &str) -> Option<(usize, usize)> {
    let rest = text.strip_prefix("@@ -")?;
    let (old, rest) = digits(rest)?;
    let rest = match rest.strip_prefix(',') {
        Some(rest) => digits(rest)?.1,
        None => rest,
    };
    let rest = rest.strip_prefix(" +")?;
    let (new, _) = digits(rest)?;
    Some((old, new))
}

fn digits(text: &str) -> Option<(usize, &str)> {
    let end = text.find(|character: char| !character.is_ascii_digit())?;
    if end == 0 {
        return None;
    }
    Some((text[..end].parse().ok()?, &text[end..]))
}

pub(super) fn json_path(value: &serde_json::Value) -> Option<String> {
    ["file_path", "filePath", "path", "notebook_path"]
        .iter()
        .find_map(|key| value.get(*key).and_then(|value| value.as_str()))
        .map(str::to_string)
}

pub(super) fn edit_line(kind: EditLineKind, text: &str) -> EditLine {
    EditLine {
        kind,
        text: text.to_string(),
    }
}

fn parse_unified_patch(patch: &str, preview: bool) -> PreviewLines {
    let mut lines = PreviewLines::default();
    if !preview {
        lines.bytes = MAX_EDIT_DIFF_BYTES;
    }
    for raw in patch.lines() {
        if raw.starts_with("diff --git")
            || raw.starts_with("index ")
            || raw.starts_with("new file mode")
            || raw.starts_with("deleted file mode")
            || raw.starts_with("similarity index")
            || raw.starts_with("rename ")
            || raw.starts_with("--- ")
            || raw.starts_with("+++ ")
        {
            continue;
        }
        let (kind, text) = if raw.starts_with("@@") {
            (EditLineKind::Hunk, raw)
        } else if let Some(text) = raw.strip_prefix('+') {
            (EditLineKind::Add, text)
        } else if let Some(text) = raw.strip_prefix('-') {
            (EditLineKind::Remove, text)
        } else if raw.starts_with('\\') {
            continue;
        } else {
            (EditLineKind::Context, raw.strip_prefix(' ').unwrap_or(raw))
        };
        lines.push(kind, text);
    }
    lines
}

pub(super) fn codex_change_note(kind: &serde_json::Value) -> String {
    let name = kind.as_str().map(str::to_string).or_else(|| {
        kind.get("type")
            .and_then(|value| value.as_str())
            .map(str::to_string)
    });
    match name.as_deref() {
        Some("add") | Some("added") | Some("create") => {
            conduit::i18n::text_static("git_state.added").into()
        }
        Some("delete") | Some("deleted") | Some("remove") => {
            conduit::i18n::text_static("git_state.deleted").into()
        }
        Some("update") | Some("modified") | Some("edit") => {
            conduit::i18n::text_static("tools.update").into()
        }
        _ => String::new(),
    }
}

pub(super) fn codex_edit_diffs(input: &str, detail: &str) -> Option<Vec<EditFileDiff>> {
    // `item/started` can carry an empty changes array before the patch is
    // applied; prefer whichever payload actually has changes.
    let changes = |text: &str| {
        serde_json::from_str::<Vec<serde_json::Value>>(text)
            .ok()
            .filter(|changes| !changes.is_empty())
    };
    let changes = changes(input).or_else(|| changes(detail))?;
    let mut files = Vec::new();
    for (index, change) in changes.iter().enumerate() {
        let path = json_path(change)
            .unwrap_or_else(|| conduit::i18n::text_static("tools.unknown_file").into());
        let note = codex_change_note(&change["kind"]);
        let mut lines = parse_unified_patch(
            change
                .get("diff")
                .and_then(|value| value.as_str())
                .unwrap_or(""),
            index < 8,
        );
        if lines.is_empty()
            && let Some(content) = change.get("content").and_then(|value| value.as_str())
        {
            lines.text(EditLineKind::Add, content);
        }
        if lines.is_empty() {
            continue;
        }
        let kind = change["kind"]
            .as_str()
            .or_else(|| change["kind"]["type"].as_str())
            .unwrap_or("");
        let status = match kind {
            "add" | "added" | "create" => EditFileStatus::Added,
            "delete" | "deleted" | "remove" => EditFileStatus::Deleted,
            _ => EditFileStatus::Modified,
        };
        files.push(lines.finish(path, note, status));
    }
    (!files.is_empty()).then_some(files)
}

pub(super) fn claude_edit_diffs(title: &str, input: &str) -> Option<Vec<EditFileDiff>> {
    let value: serde_json::Value = serde_json::from_str(input).ok()?;
    let path = json_path(&value)
        .unwrap_or_else(|| conduit::i18n::text_static("tools.unknown_file").into());
    let mut lines = PreviewLines::default();
    let mut note = String::new();
    match title {
        "Write" => {
            let content = value.get("content").and_then(|value| value.as_str())?;
            lines.text(EditLineKind::Add, content);
            note = conduit::i18n::text_static("tools.write").into();
        }
        "MultiEdit" => {
            let edits = value.get("edits").and_then(|value| value.as_array())?;
            for (index, edit) in edits.iter().enumerate() {
                lines.push(
                    EditLineKind::Hunk,
                    &conduit::i18n::format("tools.change", &[("arg0", format!("{}", index + 1))]),
                );
                for line in edit
                    .get("old_string")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .lines()
                {
                    lines.push(EditLineKind::Remove, line);
                }
                for line in edit
                    .get("new_string")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .lines()
                {
                    lines.push(EditLineKind::Add, line);
                }
            }
            note = conduit::i18n::format("tools.changes", &[("arg0", format!("{}", edits.len()))]);
        }
        _ => {
            let old = value.get("old_string").and_then(|value| value.as_str())?;
            let new = value
                .get("new_string")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if value
                .get("replace_all")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                note = conduit::i18n::text_static("tools.replace_all").into();
            }
            lines.push(
                EditLineKind::Hunk,
                conduit::i18n::text_static("tools.change_fc68b6"),
            );
            lines.text(EditLineKind::Remove, old);
            lines.text(EditLineKind::Add, new);
        }
    }
    if lines.is_empty() {
        return None;
    }
    let status = if title == "Write" {
        EditFileStatus::Added
    } else {
        EditFileStatus::Modified
    };
    Some(vec![lines.finish(path, note, status)])
}

pub(super) fn edit_tool_diffs(item: &ChatItem) -> Option<Vec<EditFileDiff>> {
    if let Some(diffs) = opencode_edit_diffs(&item.detail) {
        return Some(diffs);
    }
    match item.text.trim() {
        "修改文件" => codex_edit_diffs(&item.input, &item.detail),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            claude_edit_diffs(item.text.trim(), &item.input)
        }
        _ => None,
    }
}

fn opencode_edit_diffs(detail: &str) -> Option<Vec<EditFileDiff>> {
    let value: serde_json::Value = serde_json::from_str(detail).ok()?;
    if value["acpKind"] != "edit" {
        return None;
    }
    let files = value["content"]
        .as_array()?
        .iter()
        .filter(|part| part["type"] == "diff")
        .enumerate()
        .filter_map(|(index, part)| {
            let old = part["oldText"].as_str()?;
            let new = part["newText"].as_str()?;
            let status = if old.is_empty() {
                EditFileStatus::Added
            } else if new.is_empty() {
                EditFileStatus::Deleted
            } else {
                EditFileStatus::Modified
            };
            let mut lines = PreviewLines::default();
            if index >= 8 {
                lines.bytes = MAX_EDIT_DIFF_BYTES;
            }
            lines.push(EditLineKind::Hunk, "@@ OpenCode @@");
            lines.text(EditLineKind::Remove, old);
            lines.text(EditLineKind::Add, new);
            Some(
                lines.finish(
                    part["path"]
                        .as_str()
                        .unwrap_or(conduit::i18n::text_static("tools.unknown_file"))
                        .into(),
                    "OpenCode".into(),
                    status,
                ),
            )
        })
        .collect::<Vec<_>>();
    (!files.is_empty()).then_some(files)
}

/// Upper bound for one expanded edit card; the diff viewer scrolls inside it.
const MAX_EDIT_DIFF_HEIGHT: f32 = 320.;

pub(super) fn edit_diff_content(
    detail_key: &(u64, String),
    item: &ChatItem,
    scroll: &ScrollHandle,
    typography: &ChatTypography,
    diffs: &mut DiffCache,
    analysis: &mut tool_analysis::Cache,
    cx: &mut Context<ConduitApp>,
) -> (AnyElement, String) {
    let tool_id = format!("tool-{}-{}", detail_key.0, detail_key.1);
    let copy_text = String::new(); // Full input/output is formatted lazily by the copy action.
    let Some(value) = analysis.get(item, cx) else {
        return (
            div()
                .p_3()
                .child(conduit::i18n::text_static("tools.reading_changes"))
                .into_any_element(),
            copy_text,
        );
    };
    let files = &value.files;
    if files.is_empty() {
        return tool_content(&tool_id, item, scroll, typography);
    }
    let mut patch = String::new();
    let mut headers = HashMap::new();
    for file in files.iter().take(8) {
        patch.push_str(&file.patch);
        headers.insert(
            SharedString::from(file.path.clone()),
            (SharedString::from(file.note.clone()), file.truncated),
        );
    }
    let Some(state) = diffs.state(detail_key, &patch, cx) else {
        let fallback = div()
            .w_full()
            .min_w_0()
            .max_h(px(MAX_EDIT_DIFF_HEIGHT))
            .overflow_hidden()
            .p_3()
            .font_family(typography.code_font.clone())
            .text_size(px(typography.code_size))
            .child(patch)
            .into_any_element();
        return (fallback, copy_text);
    };
    let rows = patch.lines().count() + files.len();
    let height = edit_diff_height(rows, cx);
    let diff = Diff::new(&state)
        .line_number(true)
        .syntax_highlight(false)
        .change_indicator(DiffChangeIndicator::Bars)
        .hunk_separator(DiffHunkSeparator::LineInfo)
        .hover_highlight(DiffHoverHighlight::Line)
        .soft_wrap(true)
        .header_title_suffix(move |file, _, _| {
            let Some((note, truncated)) = headers.get(file.path()) else {
                return div().into_any_element();
            };
            h_flex()
                .gap(px(4.))
                .items_center()
                .when(!note.is_empty(), |row| {
                    row.child(
                        div()
                            .px(px(4.))
                            .rounded(px(4.))
                            .bg(rgb(palette().surface_strong))
                            .text_size(px(9.))
                            .text_color(rgb(palette().faint))
                            .child(note.clone()),
                    )
                })
                .when(*truncated, |row| {
                    row.child(
                        div()
                            .text_size(px(9.))
                            .text_color(rgb(palette().warning_text))
                            .child(conduit::i18n::text_static("tools.truncated")),
                    )
                })
                .into_any_element()
        });
    let viewport = div()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .p_3()
        .child(
            v_flex()
                .w_full()
                .min_w_0()
                .h(px(height))
                .overflow_hidden()
                .rounded(px(8.))
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(diff),
        )
        .into_any_element();
    (viewport, copy_text)
}

fn edit_diff_height(rows: usize, cx: &App) -> f32 {
    let row_height = f32::from(gpui_kit::component::Theme::global(cx).mono_font_size) * 1.6;
    (rows.max(1) as f32 * row_height).clamp(64., MAX_EDIT_DIFF_HEIGHT)
}

/// Use a block viewport around natural-height flex content. Constraining the
/// flex column itself lets its text children shrink and erases scroll overflow.
pub(super) fn tool_content(
    tool_id: &str,
    item: &ChatItem,
    scroll: &ScrollHandle,
    typography: &ChatTypography,
) -> (AnyElement, String) {
    let input = format_tool_text(&item.input);
    let output = format_tool_text(&item.detail);
    let copy_text = conduit::i18n::format(
        "tools.input_output_result",
        &[("input", input.to_string()), ("output", output.to_string())],
    );
    let mut content = v_flex()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .p_3()
        .gap_2()
        .font_family(typography.code_font.clone())
        .text_size(px(typography.code_size))
        .line_height(relative(1.5))
        .text_color(rgb(palette().muted));
    for (section, label, text, empty) in [
        (
            "input",
            conduit::i18n::text_static("tools.input"),
            input,
            conduit::i18n::text_static("tools.no_input_provided"),
        ),
        (
            "output",
            conduit::i18n::text_static("tools.output_result"),
            output,
            if matches!(
                item.status,
                crate::message_status::ToolStatus::Running
                    | crate::message_status::ToolStatus::Output
            ) {
                conduit::i18n::text_static("tools.waiting_for_tool_output")
            } else {
                conduit::i18n::text_static("tools.no_output")
            },
        ),
    ] {
        content = content
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(palette().faint))
                    .child(label),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .child(gpui_kit::base::SelectableText::new(
                        SharedString::from(format!("{tool_id}-{section}")),
                        if text.trim().is_empty() {
                            empty.into()
                        } else {
                            text
                        },
                    )),
            );
    }
    let viewport = div()
        .id(SharedString::from(format!("{tool_id}-content")))
        .h(px(240.))
        .min_h_0()
        .flex_shrink_0()
        .w_full()
        .min_w_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        // GPUI otherwise delivers the same wheel event to the chat viewport.
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .relative()
        .child(content)
        .vertical_scrollbar(scroll)
        .into_any_element();
    (viewport, copy_text)
}

/// Shared expanded body for tool cards and compact activity rows.
pub(super) fn tool_detail_panel(
    detail_key: &(u64, String),
    item: &ChatItem,
    scroll: &ScrollHandle,
    typography: &ChatTypography,
    diffs: &mut DiffCache,
    analysis: &mut tool_analysis::Cache,
    cx: &mut Context<ConduitApp>,
) -> AnyElement {
    let tool_id = format!("tool-{}-{}", detail_key.0, detail_key.1);
    let dock_key = item.key.clone();
    let dock_session = detail_key.0;
    let edit = is_tool_edit(item);
    let (content, copy_text) = if edit {
        edit_diff_content(detail_key, item, scroll, typography, diffs, analysis, cx)
    } else {
        tool_content(&tool_id, item, scroll, typography)
    };
    let copy_item = item.clone();
    v_flex()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface))
        .child(
            h_flex()
                .px_3()
                .pt_1()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child(if edit {
                            conduit::i18n::text_static("tools.code_changes")
                        } else {
                            conduit::i18n::text_static("tools.tool_call_content")
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("{tool_id}-dock")))
                        .ghost()
                        .small()
                        .compact()
                        .icon(gpui_kit::assets::IconName::PanelRight)
                        .tooltip(conduit::i18n::text_static(
                            "tools.view_tool_details_on_the_right",
                        ))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_tool_details(dock_session, &dock_key, window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("{tool_id}-copy")))
                        .ghost()
                        .small()
                        .compact()
                        .icon(gpui_kit::assets::IconName::Copy)
                        .tooltip(conduit::i18n::text_static("tools.copy_input_and_output"))
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            let text = if edit {
                                conduit::i18n::format(
                                    "tools.input_output_result_eddfa0",
                                    &[
                                        ("arg0", format_tool_text(&copy_item.input).to_string()),
                                        ("arg1", format_tool_text(&copy_item.detail).to_string()),
                                    ],
                                )
                            } else {
                                copy_text.clone()
                            };
                            cx.write_to_clipboard(ClipboardItem::new_string(text))
                        }),
                ),
        )
        .child(content)
        .into_any_element()
}

pub(super) fn activity_container(content: AnyElement, grouped: bool) -> AnyElement {
    let mut wrapper = v_flex().relative().w_full().min_w_0().pb(px(2.));
    if grouped {
        // The guide decorates the group without shifting its activity rows.
        wrapper = wrapper.child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(1.))
                .bg(rgb(palette().border)),
        );
    }
    wrapper.child(content).into_any_element()
}
