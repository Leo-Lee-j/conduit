//! Tool grouping, summaries, output formatting, and file edit previews.

use super::*;

/// Tool activity collapses into one quiet "执行工具" group. File edits and
/// unresolved or rejected approvals stay outside so their UI keeps its place.
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
    if session.messages[start..end]
        .iter()
        .filter(|item| is_groupable_activity(item))
        .count()
        < 2
    {
        return None;
    }
    Some((start, end))
}

/// Providers can start empty text blocks before issuing tools. Keep their keys
/// for future stream updates, but give them no transcript chrome or spacing.
pub(super) fn is_empty_chat_placeholder(item: &ChatItem) -> bool {
    matches!(item.kind, ItemKind::Assistant | ItemKind::Reasoning) && item.text.trim().is_empty()
}

pub(super) fn is_groupable_activity(item: &ChatItem) -> bool {
    match item.kind {
        ItemKind::Tool => is_compact_tool(item),
        ItemKind::Reasoning => !item.text.trim().is_empty(),
        _ => false,
    }
}

/// Approval history must not permanently change a normal tool's layout.
/// Keep pending and unsuccessful decisions visible outside collapsed groups.
pub(super) fn is_compact_tool(item: &ChatItem) -> bool {
    item.kind == ItemKind::Tool
        && !is_tool_edit(item)
        && item
            .approval
            .as_ref()
            .is_none_or(|approval| approval.status == "已允许")
}

/// A later turn must not hide the tool history of a turn that never replied.
pub(super) fn tool_group_has_reply(session: &SessionRecord, end: usize) -> bool {
    session.messages[end..]
        .iter()
        .take_while(|item| item.kind != ItemKind::User)
        .any(|item| item.kind == ItemKind::Assistant && !item.text.trim().is_empty())
}

pub(super) fn is_tool_edit(item: &ChatItem) -> bool {
    matches!(
        item.text.trim(),
        "修改文件" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "apply_patch"
    )
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
    if item
        .approval
        .as_ref()
        .is_some_and(|approval| matches!(approval.status.as_str(), "已拒绝" | "已中断"))
    {
        return true;
    }
    let status = item.status.trim();
    status.contains("失败")
        || status.contains("错误")
        || matches!(status, "已中断" | "已拒绝")
        || status.eq_ignore_ascii_case("failed")
        || status.eq_ignore_ascii_case("error")
        || status.eq_ignore_ascii_case("aborted")
        || serde_json::from_str::<serde_json::Value>(&item.detail)
            .ok()
            .and_then(|value| {
                value
                    .get("exitCode")
                    .or_else(|| value.get("exit_code"))
                    .and_then(serde_json::Value::as_i64)
            })
            .is_some_and(|code| code != 0)
}

/// Human category shown before the one-line summary, mirroring Codex's
/// activity rows (terminal command, workspace search, reasoning, ...).
pub(super) fn tool_category(item: &ChatItem) -> String {
    if item.kind == ItemKind::Reasoning {
        return "已思考".into();
    }
    let title = item.text.trim();
    if is_tool_edit(item) {
        return "修改代码".into();
    }
    if is_tool_command(item) {
        return "终端命令".into();
    }
    let lowered = title.to_lowercase();
    match lowered.as_str() {
        "websearch" | "web_search" | "web search" | "search" => "网络搜索".into(),
        "webfetch" | "web_fetch" => "读取网页".into(),
        "read" | "read_file" | "readfile" | "view" => "读取文件".into(),
        "grep" | "glob" | "ls" | "list" => "检索工作区".into(),
        "task" | "agent" => "子任务".into(),
        "todowrite" | "todo_write" => "规划任务".into(),
        "" => "工具调用".into(),
        _ if lowered.contains("search") || lowered.contains("grep") => "检索工作区".into(),
        _ if lowered.contains("read") => "读取文件".into(),
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
        return title;
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
/// old/new pair (Claude). Parse both into a small, bounded line model so the
/// expanded card can show a red/green diff instead of raw JSON.
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

#[derive(Clone)]
pub(super) struct EditFileDiff {
    pub(super) path: String,
    pub(super) note: String,
    pub(super) lines: Vec<EditLine>,
    pub(super) added: usize,
    pub(super) removed: usize,
    pub(super) truncated: bool,
}

pub(super) const MAX_EDIT_DIFF_LINES: usize = 400;

impl EditFileDiff {
    pub(super) fn new(path: String, note: String, mut lines: Vec<EditLine>) -> Self {
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
        Self {
            path,
            note,
            lines,
            added,
            removed,
            truncated,
        }
    }
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

pub(super) fn parse_unified_patch(patch: &str) -> Vec<EditLine> {
    let mut lines = Vec::new();
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
        let line = if raw.starts_with("@@") {
            edit_line(EditLineKind::Hunk, raw)
        } else if let Some(text) = raw.strip_prefix('+') {
            edit_line(EditLineKind::Add, text)
        } else if let Some(text) = raw.strip_prefix('-') {
            edit_line(EditLineKind::Remove, text)
        } else if raw.starts_with('\\') {
            continue;
        } else {
            edit_line(EditLineKind::Context, raw.strip_prefix(' ').unwrap_or(raw))
        };
        lines.push(line);
        if lines.len() > MAX_EDIT_DIFF_LINES {
            break;
        }
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
        Some("add") | Some("added") | Some("create") => "新增".into(),
        Some("delete") | Some("deleted") | Some("remove") => "删除".into(),
        Some("update") | Some("modified") | Some("edit") => "更新".into(),
        _ => String::new(),
    }
}

pub(super) fn codex_edit_diffs(input: &str, detail: &str) -> Option<Vec<EditFileDiff>> {
    // `item/started` can carry an empty changes array before the patch is
    // applied; prefer whichever payload actually has changes.
    let changes = |text: &str| {
        serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|value| value.as_array().cloned())
            .filter(|changes| !changes.is_empty())
    };
    let changes = changes(input).or_else(|| changes(detail))?;
    let mut files = Vec::new();
    for change in changes.iter().take(8) {
        let path = json_path(change).unwrap_or_else(|| "未知文件".into());
        let note = codex_change_note(&change["kind"]);
        let mut lines = parse_unified_patch(
            change
                .get("diff")
                .and_then(|value| value.as_str())
                .unwrap_or(""),
        );
        if lines.is_empty()
            && let Some(content) = change.get("content").and_then(|value| value.as_str())
        {
            lines.extend(
                content
                    .lines()
                    .map(|line| edit_line(EditLineKind::Add, line)),
            );
        }
        if lines.is_empty() {
            continue;
        }
        files.push(EditFileDiff::new(path, note, lines));
    }
    (!files.is_empty()).then_some(files)
}

pub(super) fn claude_edit_diffs(title: &str, input: &str) -> Option<Vec<EditFileDiff>> {
    let value: serde_json::Value = serde_json::from_str(input).ok()?;
    let path = json_path(&value).unwrap_or_else(|| "未知文件".into());
    let mut lines = Vec::new();
    let mut note = String::new();
    match title {
        "Write" => {
            let content = value.get("content").and_then(|value| value.as_str())?;
            lines.extend(
                content
                    .lines()
                    .map(|line| edit_line(EditLineKind::Add, line)),
            );
            note = "写入".into();
        }
        "MultiEdit" => {
            let edits = value.get("edits").and_then(|value| value.as_array())?;
            for (index, edit) in edits.iter().enumerate().take(20) {
                lines.push(edit_line(
                    EditLineKind::Hunk,
                    &format!("@@ 修改 {} @@", index + 1),
                ));
                for line in edit
                    .get("old_string")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .lines()
                {
                    lines.push(edit_line(EditLineKind::Remove, line));
                }
                for line in edit
                    .get("new_string")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .lines()
                {
                    lines.push(edit_line(EditLineKind::Add, line));
                }
            }
            note = format!("{} 处修改", edits.len());
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
                note = "全部替换".into();
            }
            lines.push(edit_line(EditLineKind::Hunk, "@@ 修改 @@"));
            lines.extend(
                old.lines()
                    .map(|line| edit_line(EditLineKind::Remove, line)),
            );
            lines.extend(new.lines().map(|line| edit_line(EditLineKind::Add, line)));
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(vec![EditFileDiff::new(path, note, lines)])
}

pub(super) fn edit_tool_diffs(item: &ChatItem) -> Option<Vec<EditFileDiff>> {
    match item.text.trim() {
        "修改文件" => codex_edit_diffs(&item.input, &item.detail),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            claude_edit_diffs(item.text.trim(), &item.input)
        }
        _ => None,
    }
}

pub(super) fn edit_diff_line(line: &EditLine, typography: &ChatTypography) -> AnyElement {
    let (background, color, marker) = match line.kind {
        EditLineKind::Add => (Some(palette().diff_add_bg), palette().diff_add_text, "+"),
        EditLineKind::Remove => (
            Some(palette().diff_remove_bg),
            palette().diff_remove_text,
            "-",
        ),
        EditLineKind::Hunk => (None, palette().warning_text, ""),
        EditLineKind::Context => (None, palette().muted, ""),
    };
    h_flex()
        .w_full()
        .min_w_0()
        .when(line.kind == EditLineKind::Hunk, |row| {
            row.bg(rgb(palette().surface_strong)).py(px(1.))
        })
        .when_some(background, |row, background| row.bg(rgb(background)))
        .font_family(typography.code_font.clone())
        .text_size(px(typography.code_size))
        .line_height(relative(1.45))
        .child(
            div()
                .w(px(22.))
                .flex_shrink_0()
                .pr_1()
                .text_right()
                .text_color(rgb(palette().faint))
                .child(marker),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(rgb(color))
                .child(line.text.clone()),
        )
        .into_any_element()
}

pub(super) fn edit_diff_content(
    tool_id: &str,
    item: &ChatItem,
    scroll: &ScrollHandle,
    typography: &ChatTypography,
) -> (AnyElement, String) {
    let input = format_tool_text(&item.input);
    let output = format_tool_text(&item.detail);
    let copy_text = format!("输入\n{input}\n\n输出 / 结果\n{output}");
    let Some(files) = edit_tool_diffs(item) else {
        return tool_content(tool_id, item, scroll, typography);
    };
    let mut content = v_flex().w_full().min_w_0().flex_shrink_0().p_3().gap_3();
    for file in files {
        let mut rows = v_flex().w_full().min_w_0();
        for line in &file.lines {
            rows = rows.child(edit_diff_line(line, typography));
        }
        let mut header = h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .bg(rgb(palette().surface_strong))
            .child(
                Icon::new(FullIconName::File)
                    .size(px(13.))
                    .text_color(rgb(palette().faint)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(typography.code_font.clone())
                    .text_size(px(11.))
                    .text_color(rgb(palette().text))
                    .child(file.path.clone()),
            );
        if !file.note.is_empty() {
            header = header.child(
                div()
                    .flex_shrink_0()
                    .px_1()
                    .rounded(px(4.))
                    .bg(rgb(palette().surface))
                    .text_size(px(9.))
                    .text_color(rgb(palette().faint))
                    .child(file.note.clone()),
            );
        }
        if file.truncated {
            header = header.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(9.))
                    .text_color(rgb(palette().warning_text))
                    .child("已截断"),
            );
        }
        header = header
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(rgb(palette().diff_add_text))
                    .child(format!("+{}", file.added)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(rgb(palette().diff_remove_text))
                    .child(format!("-{}", file.removed)),
            );
        content = content.child(
            v_flex()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(palette().border))
                .child(header)
                .child(rows),
        );
    }
    let viewport = div()
        .id(SharedString::from(format!("{tool_id}-diff-content")))
        .max_h(px(320.))
        .min_h_0()
        .flex_shrink_0()
        .w_full()
        .min_w_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .relative()
        .child(content)
        .vertical_scrollbar(scroll)
        .into_any_element();
    (viewport, copy_text)
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
    let copy_text = format!("输入\n{input}\n\n输出 / 结果\n{output}");
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
        ("input", "输入", input, "未提供输入"),
        (
            "output",
            "输出 / 结果",
            output,
            if matches!(item.status.as_str(), "运行中" | "输出中") {
                "等待工具输出…"
            } else {
                "没有输出内容"
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
    tool_id: &str,
    item: &ChatItem,
    scroll: &ScrollHandle,
    typography: &ChatTypography,
    cx: &mut Context<ConduitApp>,
) -> AnyElement {
    let dock_key = item.key.clone();
    let edit = is_tool_edit(item);
    let (content, copy_text) = if edit {
        edit_diff_content(tool_id, item, scroll, typography)
    } else {
        tool_content(tool_id, item, scroll, typography)
    };
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
                            "代码变更"
                        } else {
                            "工具调用内容"
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("{tool_id}-dock")))
                        .ghost()
                        .small()
                        .compact()
                        .icon(gpui_kit::assets::IconName::PanelRight)
                        .tooltip("在右侧查看工具详情")
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.details = true;
                            this.detail_view = DetailView::Tool;
                            this.detail_text = Some(dock_key.clone());
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("{tool_id}-copy")))
                        .ghost()
                        .small()
                        .compact()
                        .icon(gpui_kit::assets::IconName::Copy)
                        .tooltip("复制输入与输出")
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()))
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
