# CLI 消息与异常处理审查

本文件记录 Claude Code、Codex、OpenCode、Qoder CLI 从原生消息到聊天 UI 的覆盖范围、缺陷修复和验证边界。审查基于当前工作区；离线夹具通过不等于真实账户和桌面人工验收通过。

## 缺陷清单

| 编号 | 优先级 | 触发条件与影响 | 状态 |
| --- | --- | --- | --- |
| D1 | P1 | Codex 子代理 `thread/read` 返回 RPC 错误，被主任务通用错误分支提前结束 | 已修复，子代理观察器先接收自己的 RPC 响应 |
| D2 | P1 | OpenCode 普通聊天返回原生 `cancelled`，被当作成功并可能续发队列 | 已修复，意外取消进入失败路径，UI 保留并暂停排队消息 |
| D3 | P2 | 新消息的首个 Delta 与上一条正文相同，被完整消息去重逻辑吞掉 | 已修复，仅完整 Text 快照适用相邻去重 |
| D4 | P2 | 工具取消、未返回结果或状态未知时，UI 默认显示绿色成功图标 | 已修复，列表与详情只对明确完成状态显示成功，其他结束状态使用中性图标及原状态文字 |
| D5 | P2 | Qoder 把翻译后的 Running 写入领域状态，英文运行/取消状态识别失败 | 已修复，写入稳定状态；兼容历史英文 Running 的结束处理 |
| D6 | P2 | Qoder 上报 token 数但未设置 known 标志，用量账本视其为未知 | 已修复，按原生字段是否存在设置 known，保留零值与缺失的区别 |
| D7 | P2 | Codex initialize 后提前解除启动超时，thread/turn 启动无回复时无限等待 | 已修复，覆盖线程恢复和回合启动确认；60 秒总启动计时不被轮询和无关消息重置 |

## 消息覆盖事项

| 编号 | 范围 | 预期行为 | 状态 |
| --- | --- | --- | --- |
| C1 | Claude 状态、压缩、工具进度、认证和限流通知 | 展示原生可公开的进度和异常；保留正文/工具身份，不把进度混作回复 | 已接入，见下方映射与边界 |
| C2 | Codex 已知进度和未识别通知 | 已知通知有明确映射；未知通知有有界提示，不冒充成功 | 已接入，未知载荷不原样转存 |
| C3 | OpenCode/Qoder 非文本回复 | 文本资源和资源链接可阅读；图片、音频等提供明确降级提示，不静默丢弃或保存回复中的编码数据 | 已接入，保留消息 ID，二进制回复仅提示 |
| C4 | OpenCode/Qoder 不支持的反向请求/表单 | 返回协议错误同时给出可见提示，说明交互未完成 | 已接入，不改变权限判定和表单校验 |

## 消息呈现约定

- Claude：`system/status`、`api_retry`、`tool_progress`、hook 运行及认证进度更新运行状态；`compact_boundary`、限流、hook 失败和信息通知显示提示。认证错误和压缩错误使用错误卡片；`informational.prevent_continuation` 结束任务并报错。心跳、初始化和重复工具摘要不增加正文。认证 `output/error` 原文不展示，避免把登录码或凭据写入记录。
- Codex：MCP 工具进度、文件变更进度、线程 active 状态及 hook 运行更新状态；配置告警、压缩完成、模型切换、MCP 启动失败、线程 systemError 和 hook 失败显示提示。未知通知按类型去重，并受当前线程/回合过滤。公开推理及其最终快照沿用既有专用映射，未公开推理不展示。
- OpenCode/Qoder：`agent_message_chunk` 的 `text`、带 `resource.text` 的嵌入资源，以及 `http/https/file` 的资源链接可阅读。带用户名/密码的链接、其他协议的链接、图片、音频和二进制资源均降级为提示；不提取其编码载荷。该规则针对代理回复，不对任意工具的自由格式原始输出作自动脱敏承诺。恢复回放与其他会话仍被过滤。
- 兼容提示每个运行最多保留 32 个不同键，单条最多 2048 字符，未知类型名最多 96 字符。重复进度在队列中合并，但不能跨越正文、审批或结束事件。短暂进度不新增聊天消息，也不会覆盖待处理审批或取消状态。
- 未知反向请求返回 `-32601`；不支持的 Qoder 表单返回 `-32602`，同时显示一次兼容提示。拒绝表单时不回显可能包含敏感字段的原始请求。不支持的功能不会自动获批。

这些映射以本地 Codex app-server 生成的 JSON schema、Claude Agent SDK 类型定义和 ACP v1 schema 为核对依据；不保证未来 CLI 新增类型都已有专用 UI。新增类型走兼容提示，图片/音频内联播放和真实账户端到端验收仍不在本轮完成范围。

## 回归覆盖

| 范围 | 关键用例及所在模块 |
| --- | --- |
| D1 | `protocol::subagents::tests::child_observer_rpc_error_does_not_finish_main_turn`：观察失败后主回合继续接收正文 |
| D2、进度 UI | `app::queue::tests::cli_activity_updates_status_without_transcript_rows_or_successful_queue_dispatch`；OpenCode `unexpected_native_cancel_is_an_error_and_unsupported_requests_are_visible` |
| D3 | `app::tests::new_delta_matching_previous_reply_is_preserved_and_snapshots_still_replace`；保留既有完整块去重测试 |
| D4、D5 | `app::tests::tool_success_requires_explicit_completion_and_legacy_running_can_finish`；Qoder 中英文工具生命周期回归 |
| D6 | `cli_adapter::qoder::tests::qoder_tool_lifecycle_and_usage_presence_are_independent_of_locale`：零值已知、缺失缓存计数未知 |
| D7 | `protocol::tests::codex_startup_deadline_covers_initialize_thread_and_turn_acknowledgements`；`codex_commands` 中命令 RPC 启动确认回归 |
| C1、C2 | `protocol::notifications::tests`：进度与正文分离，认证数据不泄漏，异会话/旧回合过滤，未知类型有界去重 |
| C3、C4 | OpenCode/Qoder 的资源、媒体和反向请求用例；`protocol::presentation::tests` 的不安全链接、缺字段及二进制降级回归 |
| 事件队列 | `event_queue::tests::activity_updates_coalesce_preserve_barriers_and_failed_send_preserves_old_state` |

## 验收要求

- 按原生会话、回合、消息和工具 ID 隔离；恢复回放、旧回合和其他会话不能污染当前 UI。
- 原生失败、取消、拒绝、限额、未知完成原因不能进入成功通知或自动续发队列路径。
- 运行中、成功、失败、停止、未返回结果和未知状态分别呈现；工具状态识别不依赖当前界面语言，Qoder 写入稳定的领域状态值。
- 快照替换与增量追加语义分开；最终内容可缩短或清空，流式首段不能因去重丢失。
- 不展示加密/隐藏思考、认证凭据和非文本 base64 数据。未知协议采用有界提示，避免每个增量生成一张卡片。
- 关键回归覆盖适配器输出、消息保存及 UI 状态；真实 CLI 付费调用保持显式启用。

## 验证记录

验证日期：2026-11-17。修复前相关范围：173 通过，0 失败，15 个真实 CLI/安装环境用例默认跳过。

本次修复后的验证结果：

| 检查 | 结果 |
| --- | --- |
| 协议、适配器、事件队列、消息处理和 UI 测试 | 169 通过，0 失败 |
| `app::queue::tests` | 8 通过，0 失败，包含运行状态展示及意外取消暂停队列 |
| `chat_runtime` | 22 通过，0 失败，15 个真实 CLI/安装环境用例默认跳过 |
| 上述相关范围合计 | **199 通过，0 失败，15 跳过** |
| `cargo test --locked --lib -- --test-threads=1` | 61 通过，0 失败，1 跳过 |
| `cargo fmt --all --check` | 通过 |
| `python scripts/check-i18n.py` | 通过，2 种语言各 2996 条消息；新增插值已验证 |
| `cargo build --locked --bin conduit --bin conduit-updater --bin conduit-launcher` | 通过；存在下述 GitHub 模块的未使用方法警告 |

相关测试使用 `cargo test --locked --features ui-tests --bin conduit --no-run` 编译的同一测试程序，分别运行 `protocol::`、`cli_adapter::`、`event_queue::`、`app::tests::`、`app::ui_tests::`、`app::queue::tests::`、`chat_runtime::` 过滤范围，均使用单测试线程。

全仓验证仍有以下阻塞，不能据此宣称全仓测试和 lint 已通过：

- 首次完整主程序测试为 705 通过、5 失败、21 跳过。其中 4 个消息链路相关失败已经修复，并包含在上述最终通过的复验范围中；剩余 GitHub 模块测试 `app::github::tests::failed_check_task_preserves_workspace_drafts_and_user_acceptance` 单独复现仍失败（`src/app/github/tests.rs:483`，选中工作区为 `Some(0)`，预期 `Some(1)`）。该问题未纳入本轮 CLI 消息修复；未重新运行整套主程序测试。
- `cargo clippy --locked --all-targets --features ui-tests -- -D warnings` 仍因 `src/app/github/document.rs:22` 的 `open_github` 未使用而失败。本轮新增模块的 Clippy 问题已修复，该 GitHub 警告未在本轮修改。

本轮完成代码修复、离线协议夹具和 GPUI 状态回归；未执行真实账户模型调用、实际桌面交互和图片/音频内联展示验收。表格中的“已接入”表示有映射和回归覆盖，不代表所有 CLI 版本及现场环境已完成端到端验证。

### v0.3.9 发布前复验

2026-11-17 发布收尾已处理上述两个 GitHub 阻塞：`open_github` 仅用于 UI 测试，现按测试配置编译；工作流侧栏打开任务时会切换至任务所属工作区，测试已验证切回原会话后草稿仍然保留，任务内容与验收状态不丢失。

- 库测试：61 通过、1 跳过。
- 完整主程序测试：709 通过、1 个旧工作区选择断言失败、21 跳过；修正该断言后，相关 GitHub UI 测试单独复验 8 通过、0 失败。未重复运行完整套件。
- 发布签名工具测试：1 通过；启动器与更新器测试目标均编译通过。
- 格式、国际化与 Clippy 检查通过；主程序、更新器和启动器的非测试构建通过。
