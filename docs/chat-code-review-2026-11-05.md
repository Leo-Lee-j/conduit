# Chat 相关功能代码审查

审查日期：2026-11-05。审查范围包括 ChatRuntime、会话发送与重试、流式事件、审批通知、队列消息、Transcript 加载和持久化。审查阶段先落地本文档；随后按用户指示实施以下修复。工作区原有的插件、Git/UI 等改动保留，未提交或重置工作区。

审查时判断：流式事件队列、审批 ticket、Transcript 身份校验和队列消息恢复已经有较完整的回归覆盖。需要优先处理运行终态、发送与持久化之间的确认关系，以及审批通知的生命周期一致性。

## 修复进展（2026-11-05）

以下五项均已实现，最后一轮离线验证结果在本文后部记录。原始发现保留在下节，行号为审查时版本。

| 发现 | 实施结果 | 主要代码 |
| --- | --- | --- |
| 1. 外层 panic 无 Finished | 工作线程在 unwind 边界外持有 Child，panic 也先进入统一清理，再发送一次 Finished；发送前重新采样取消状态。 | [chat_runtime.rs](../src/chat_runtime.rs) |
| 2. 入写队列被误当成落盘 | 发送快照取得独立回执，成功前保留草稿/附件，CLI 在门闩后等待；发送快照不参与普通快照合并。失败、取消或关闭会恢复发送前正文、错误卡及队列内容。 | [persistence.rs](../src/persistence.rs)、[runtime/saves.rs](../src/app/runtime/saves.rs)、[runtime.rs](../src/app/runtime.rs)、[queue.rs](../src/app/queue.rs) |
| 3. 路由失败留下空会话 | 先构造临时 SessionRecord 并校验工作区、CLI 和 Provider 路由，再插入会话。临时模型选择在保存成功且用户未改变选择时才消费。 | [sessions.rs](../src/app/sessions.rs)、[runtime.rs](../src/app/runtime.rs) |
| 4. 回答通知键复用 | 工具审批和用户问题的通知键均包含 runtime identity；完成操作按对应运行与请求精确解除。问题通知仍只提供导航，不提供工具允许/拒绝按钮。 | [notifications/approvals.rs](../src/app/notifications/approvals.rs) |
| 5. 清理无可见超时 | 强制清理超过 10 秒发送 CleanupBlocked，显示错误、暂停队列并取消关闭等待；继续持有 Job/Child 和会话互斥，后台每 500ms 重试，确认进程树退出后才发送 Finished。 | [chat_runtime.rs](../src/chat_runtime.rs)、[stream_events.rs](../src/app/stream_events.rs)、[event_queue.rs](../src/event_queue.rs) |

第 5 项采用“可见故障 + 后台清理”，没有在超时时直接释放 busy。超时不能证明进程已经退出；同一 native 会话及 Git 写操作仍需互斥，其他会话和界面可继续使用。清理故障和最终完成事件各有独立保留容量；命名及提交说明生成也处理该故障。

发送确认还覆盖以下边界：

- 写入期间修改文字、附件、模型或切换会话，不会清掉后来编辑的草稿或改变当前导航；未保存的新会话暂不允许切入编辑。
- 后台发送失败通知对应来源，保留前台会话的提示；队列写入未确认时禁止发送、撤回和删除其会话/工作区。切换引擎发送时，保存确认前也保护来源会话及其草稿。
- 保存取消后的迟到回执不能重新启动 CLI。关闭时写入失败会撤销关闭状态，保留可操作的恢复界面。
- 附件回收等待最新状态保存成功且没有待确认发送；退出前不再临时提交一个无人等待的保存任务。
- 删除 `persist()` 中仅在测试构建启用的同步 flush；需要读取磁盘的旧测试显式等待完成，生产与测试均走异步路径。

## 发现

### 1. [P1] ChatRuntime 外层 panic 不会发送 Finished，运行状态可能永久占用

位置：[src/chat_runtime.rs:256](../src/chat_runtime.rs#L256)、[src/chat_runtime.rs:262](../src/chat_runtime.rs#L262)、[src/app/stream_events.rs:256](../src/app/stream_events.rs#L256)、[src/app/runtime.rs:685](../src/app/runtime.rs#L685)。

CLI 工作线程外层 `catch_unwind` 捕获 panic 后只发送 `Event::Notice`，随后直接返回。UI 侧只有收到 `Event::Finished` 才会从 `session_control.runs` 移除运行；事件通道关闭本身不会触发同样的清理。因此，协议处理线程发生未预期 panic 时，会话仍被视为运行中，后续发送、重试和正常关闭都可能被阻塞。

建议：panic 路径也必须进入统一的进程清理和终态上报流程，保证发送一次 `Finished { error: Some(...), cancelled: ... }`。清理失败时应继续使用明确的失败终态，避免以永久 busy 作为唯一安全策略。

### 2. [P1] 发送前的持久化检查不代表消息已经写入磁盘

位置：[src/app/runtime.rs:405](../src/app/runtime.rs#L405)、[src/app.rs:761](../src/app.rs#L761)、[src/persistence.rs:52](../src/persistence.rs#L52)、[src/persistence.rs:66](../src/persistence.rs#L66)。

`start_turn` 在启动 CLI 前调用 `persist()`。正式构建中，`Persistence::submit` 只确认快照已经放入待写队列；真实磁盘错误通过后台线程稍后回报。`persist()` 因此可能返回成功，随后立即启动 CLI 并清空输入框，但用户消息尚未获得磁盘写入确认。应用在写入完成前异常退出时，消息可能丢失。

新会话还会先在 [src/app/sessions.rs:1332](../src/app/sessions.rs#L1332) 创建并保存空会话。若随后写入失败或后台保存失败，空会话可能留在 Store 中。

建议：区分“已提交写队列”和“已确认持久化”两个状态。发送前至少等待包含用户消息的快照得到成功回执，或设计可恢复的发送日志；失败时保留草稿和附件。关闭窗口也应等待该版本成功或显示明确的恢复状态。

### 3. [P2] Provider 路由校验失败会留下空会话并消费临时选择

位置：[src/app/runtime.rs:355](../src/app/runtime.rs#L355)、[src/app/runtime.rs:373](../src/app/runtime.rs#L373)、[src/app/sessions.rs:1386](../src/app/sessions.rs#L1386)。

发送新会话时先调用 `create_session`，然后才解析 `session_route()`。当 Provider 被删除、未同步或凭据不可用时，路由解析会失败，但已经创建的会话不会回滚。`create_session` 过程中也可能消费 `pending_model_selection`，失败后用户的临时模型选择无法恢复。

建议：在创建会话前完成 Provider、凭据、模型和工作区校验；如果必须延后创建，则为会话、选择状态、标签和草稿建立完整事务式回滚。

### 4. [P2] 重复生成可能复用“等待回答”通知键，导致新请求不再提醒

位置：[src/app/notifications/approvals.rs:83](../src/app/notifications/approvals.rs#L83)、[src/notifications.rs:280](../src/notifications.rs#L280)、[src/notifications.rs:294](../src/notifications.rs#L294)。

用户输入类审批通知的 key 是 `approval/{session}/{prefix}/{approval.id}`，没有包含本次 `ChatRuntime` 的唯一 identity。若重新生成复用了同一轮前缀和协议请求 ID，通知中心会把新通知识别为已见过的同一事件；即使旧记录已经 resolved，也会返回 `changed = false`，不会重新进入未读或 Toast 队列。

建议：所有等待中的运行通知都绑定运行 identity；通知完成、取消和过期时同时按 session、runtime、request 精确解除绑定。可以保留业务 key 用于展示去重，但不能让旧运行的生命周期决定新运行是否提醒。

### 5. [P2] 进程终止失败时清理循环没有超时终态

位置：[src/chat_runtime.rs:637](../src/chat_runtime.rs#L637)。

`execute` 在 `child.terminate()` 失败后持续重试，直到 `active_processes() == 0` 且进程退出。这样能避免遗留子进程，但如果 JobObject 或系统进程状态始终无法收敛，工作线程不会发送 `Finished`，应用关闭也会一直等待该运行结束。

建议：设置清理总超时，超时后上报带有清理失败详情的终态，并把“可能存在遗留进程”作为可见诊断；同时保留一次强制终止和后续后台观察机制，避免 UI 永久等待。

## 已确认的正确行为

- 审批 ticket 会区分不同请求，撤销旧 ticket 不会影响其他待处理审批。
- 审批提交前会重新验证 session、runtime、request 和审批状态，断开的运行会把通知标记为失效。
- 事件队列有事件数和字节上限，并为完成事件保留容量；流式 Delta 会在安全边界内合并。
- Transcript 加载会校验 session、reference 和当前版本，冷会话不会被误当成空会话。
- 队列消息在后台会话失败、取消、附件失效和重启后会保留为暂停状态，不会自动发送。

## 审查阶段验证（修复前）

本轮定向验证结果：

- ChatRuntime 专项测试：11 项通过，0 项失败，10 项忽略；忽略项需要真实 CLI。
- 并发会话测试：1 项通过。
- Chat 相关 app 纯逻辑测试：38 项通过。
- 队列、Transcript、审批通知测试：12 项通过。
- 队列与持久化测试：7 项通过。
- Diff、审批预览和工具详情测试：7 项通过。
- `cargo fmt --all --check`：通过。
- `cargo clippy --offline --locked --all-targets --features ui-tests -- -D warnings`：通过。
- `git diff --check`：通过；输出中的 LF/CRLF 提示是换行符警告。

更宽的筛选回归为 290 项通过、3 项失败、13 项忽略。3 个失败均属于与本次 Chat review 无关的 Git/UI 测试：

- `app::files::git_navigation_tests::git_navigation_staging_diff_tabs_and_unique_settings`
- `app::files::git_navigation_tests::start_in_git_opens_left_navigation_without_right_dock`
- `app::git_log::tests::branch_manager_navigation_filters_favorites_commit_diff_and_workspace_isolation`

## 修复后验证

- 新增 14 项受控写入与 UI 回归，覆盖慢写、磁盘失败、路由失败、取消与关闭、重试/重新生成、前后台草稿、模型选择、引擎切换、纯附件消息、队列以及清理故障，全部通过。
- 新增运行时 panic 和清理超时恢复回归；持久化回归确认独立失败回执不会被后续成功写入覆盖、写线程 panic 后仍可重试。
- `cargo fmt --all --check`：通过。
- `cargo clippy --offline --locked --all-targets --features ui-tests -- -D warnings`：通过。
- `git diff --check`：通过；LF/CRLF 提示为换行符警告。
- 最后一次 Chat 定向回归：137 项通过，0 项失败，10 项忽略。覆盖新增保存回归、队列、Chat UI、Transcript、app 逻辑、审批通知、ChatRuntime、Persistence 和事件队列。

定向回归命令：

```powershell
cargo test --offline --locked --features ui-tests --bin conduit -- app::runtime::saves::tests app::queue::tests app::ui_tests app::transcripts::tests app::tests app::notifications::tests::approvals chat_runtime::tests persistence::tests event_queue::tests --test-threads=1
```

完整离线回归未跑完：已运行部分重现了上节记录的 3 项 Git/UI 基线失败；随后原有 `models::opencode::tests::native_cache_survives_restart_expires_daily_and_manual_refresh_bypasses_it` 使用无效 exe 的失败路径触发 Windows「不支持的 16 位应用程序」对话框，阻塞测试。Computer Use 原生连接不可用，无法自动关闭提示；核对 PID、可执行文件路径和标题后，仅结束本轮自行启动的测试进程。该阻塞项不计为通过，完整套件不声明通过。

验证使用受控 writer、事件通道、临时目录中的离线 launcher 和本地测试进程。需要真实 CLI/模型的忽略项未启用；没有调用真实模型或提交代码。
