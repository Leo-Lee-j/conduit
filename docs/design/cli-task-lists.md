# CLI 任务列表

## 行为

输入框右上方显示已完成数 / 总数，点击展开任务列表；右侧会话数据面板使用同一份状态。长标题换行显示，Claude Tasks 显示原生编号、负责人和未完成的前置依赖，浮层悬停可查看描述。CLI 没有提供任务数据时不生成任务。

任务展示是只读的。任务创建、修改、删除仍由 CLI 执行；本功能不改动 CLI 权限、用户配置或原生任务文件。

| 来源 | 处理方式 | 生命周期 |
| --- | --- | --- |
| Codex `turn/plan/updated` | 完整步骤快照 | 当前用户轮次 |
| Claude `TodoWrite` / `todo_write` / `update_plan` | 工具成功后接收完整计划 | 当前用户轮次 |
| Claude `TaskCreate` | 从结果获取真实任务 ID | 会话内跨轮次 |
| Claude `TaskUpdate` | 只应用 CLI 确认修改的字段；支持删除、负责人、依赖 | 会话内跨轮次 |
| Claude `TaskList` | 替换完整任务集合，包括空列表 | 会话内跨轮次 |
| Claude `TaskGet` | 更新指定任务 | 会话内跨轮次 |
| OpenCode / Gemini / Qoder ACP `plan` | 完整步骤快照 | 当前用户轮次 |

## 协议依据

2026-11-25 使用本机 Claude Code **2.1.322**，在独立配置目录和工作目录中连接本地合成 Anthropic SSE 服务，实际执行 TaskCreate → TaskUpdate → TaskList → TaskGet → 删除 → 空列表 → 不存在任务更新。

可复现探针：`scripts/probe-task-list.py`。原生 stdout 和 JSONL 样例：`src/protocol/fixtures/claude-task-list.json`。样例只包含合成任务，目录和会话 ID 已归一化。探针不调用付费模型 API，关闭 hooks 和 MCP，并将可用工具限定为四个 Task 工具。请每次使用新的输出目录。

```powershell
python scripts/probe-task-list.py --claude C:/path/to/claude.exe --output target/task-probe-new --fixture src/protocol/fixtures/claude-task-list.json
```

观察到的关键区别：

- stdout 使用 `tool_use_result`，原生 JSONL 使用 `toolUseResult`。
- 创建结果在 `task.id` 中给出真实 ID，不能按调用顺序猜测。
- 更新结果包含 `success`、`taskId`、`updatedFields`，状态变更另有 `statusChange`。
- 更新不存在的任务会返回 `success: false`，但工具结果块可能没有 `is_error`。
- `TaskList` 的 `{ "tasks": [] }` 表示明确清空，不能回退到更早的 TODO。
- `TaskList` 不包含完整描述，刷新列表时保留已知描述；`TaskGet` 提供完整任务信息。
- 本机 CLI 在 `--bare` 模式下未提供 Task 工具。探针使用普通 print 模式并在隔离环境中设置 `CLAUDE_CODE_ENABLE_TASKS=true`；Conduit 不强制修改用户的此项配置。

官方文档站本次访问返回 403，因此适配以本机 CLI 实测协议为依据，未将网络检索结果作为兼容性证明。

## 状态和恢复

`src/task_list.rs` 保存有界的任务快照，`src/task_list/claude.rs` 按 tool-use ID 关联调用与结果。重复结果不重复添加任务；失败、未知 ID、畸形列表和子代理消息不会冒充主会话任务。仅有可读工具结果时，按已捕获的原生格式保守解析，不扫描任意聊天文本。

会话快照与会话记录一起保存，包括明确为空的列表。普通计划关联最新用户消息，开启新轮次后隐藏；Claude Tasks 跨轮次保留。创建分支时不复制分支点之后的最新快照。

原生 Claude 历史导入复用同一个关联器，并保存增量扫描所需的待匹配调用。历史索引解析版本递增，旧缓存会重新解析。导入覆盖本地快照时，必须包含本地最后一次成功任务调用以及后续成功调用，避免延迟扫描回滚状态，同时允许继续同步 CLI 侧的新工作。旧版本会话仍可从已完成的计划工具输入恢复列表。

## 验证

回归覆盖真实事件样例、原生 JSONL 增量导入、跨运行更新、失败和重复调用、批量结果关联、空列表、跨轮次作用域、磁盘保存和冷历史恢复，以及两处 UI 的长标题、依赖、清空和会话隔离。

```powershell
cargo test --locked --features ui-tests task_list -- --test-threads=1
cargo test --locked --features ui-tests plan_updates_from_both_providers -- --test-threads=1
```

本次验证结果：

- 本机 CLI 探针成功，13 次本地模型请求，覆盖成功和预期失败结果。
- 任务列表、UI、历史导入、存储、旧计划兼容回归：84 通过、1 个依赖真实用户历史的既有测试按默认设置忽略。
- 协议、后台子任务及分支兼容测试：50 通过（与上一组有 1 项重叠）。
- 修改文件的 rustfmt、`git diff --check` 和中英文文案检查通过。
- 严格全仓 Clippy 被其他文件的 `dead_code` 和 `cloned_ref_to_slice_refs` 阻断；仅放行这两类后，`cargo clippy --locked --features ui-tests --bin conduit --tests -- -D warnings -A dead_code -A clippy::cloned_ref_to_slice_refs` 通过。
- 全仓格式检查另报 `src/update/tests.rs`、`src/transcript/chunks.rs` 的差异；vendor 检查报两个 highlighter 文件的未声明变更。这些文件不属于本次任务修改范围。
