# OpenCode CLI 接入评估

评估日期：2026-10-27。代码基线：`c238780`。本机 CLI：OpenCode **1.19.2**；以下结论针对该版本，不表示已经兼容所有版本，也不把本机版本当作官方最新版本。

## 结论

**建议接入，首选 `opencode acp`，总体复杂度中等偏高。** 现有 Windows 子进程、NDJSON 管道、审批队列和聊天事件可以复用；主要新增工作是 ACP 会话状态机、动态模型配置、权限策略和历史导入。不能只增加一个引擎枚举和启动命令就达到现有两款 CLI 的体验。

本次完成代码审查、官方版本源码核对和隔离协议探测；没有修改应用实现，没有发送模型请求，没有验证真实工具执行或带对话内容的上下文续接。

## 接入入口比较

| 入口 | 适配情况 | 决策 |
| --- | --- | --- |
| `opencode run --format json` | 单次任务、指定原生会话和事件输出方便；1.19.2 非交互运行遇到 `permission.asked` 时，默认自动拒绝，`--auto` 则自动允许；没有与现有 UI 对接的双向 stdio 审批接口 | 不作为完整聊天接入入口 |
| `opencode acp` | JSON-RPC 2.0 over NDJSON/stdin/stdout；支持会话、流式更新、双向审批、配置选择和取消 | **首选**，复用现有传输与进程监管 |
| `opencode serve` + HTTP/SSE | 原生 API 更完整，便于直接查询消息和 Provider；需要新增 HTTP/SSE 客户端、鉴权、服务地址和生命周期管理 | 保留为后续原生能力扩展方案 |

ACP 命令在内部也启动本地 HTTP 服务，并通过 SDK 访问它；不是完全无网络监听的纯管道程序。Conduit 仍可只实现 stdio 客户端，无需为第一阶段引入 Node SDK 或 HTTP 客户端。建议明确指定回环地址、随机端口，检查环境/配置中的网络设置，并为内部服务配置进程级随机密码。

## 已有代码的可复用部分与改动位置

| 位置 | 现状 / 建议 |
| --- | --- |
| `src/domain.rs` | `Provider` 只有 Claude/Codex；增加 OpenCode，保持旧序列化值不变。会话已有 `native_id`、模型、权限、usage，可继续使用 |
| `src/cli.rs` | 支持 `.exe`/`.cmd`、路径优先级与受管版本探测；新增 OpenCode 路径配置和 `CONDUIT_OPENCODE_PATH`。其 `--version` 只输出 `1.19.2`，当前要求品牌标识的解析逻辑不能直接套用 |
| `src/protocol.rs` | `Step { writes, events, done, error }` 和 `Event` 能覆盖主要 ACP 事件；建议把 ACP 状态机放独立模块，避免继续膨胀现有两引擎解析器 |
| `src/chat_runtime.rs` / `src/child.rs` | 可复用 NDJSON、有界写入、审批 ticket、Job 进程树及退出屏障；取消入口需支持先发协议取消，再以 Job 清理兜底 |
| `src/models.rs` / `src/app/model_picker.rs` | 目前模型目录主要读取本地配置、思考强度固定五档；OpenCode 应按工作区读取有效配置和动态选项，后台刷新，不能只扩充固定数组 |
| `src/providers.rs` / `src/ccs.rs` | 当前端点注入仅覆盖 Anthropic/Responses。OpenCode 有自身 Provider ID、SDK 类型及 options；首期跟随 OpenCode 自身配置，定制 Provider/CCS 单列后续工作 |
| `src/history.rs` | 当前只扫描两款 CLI 的 JSONL；优先用 ACP `session/list` + 按需 `session/load`，避免第一版直接依赖 OpenCode SQLite 私有表结构 |
| `src/app.rs`、`src/app/settings*`、`src/app/components.rs` | 引擎列表、目录加载、设置导航、图标和部分控件 ID 都有两引擎硬编码，需要统一检查；不建议为此先做全项目插件系统重构 |
| `src/app/naming.rs` / Git 说明生成 | 使用 `text_only()` 的临时任务也必须适配；仅在 UI 收到工具事件后取消，不足以保证没有执行工具 |

现有 `session_busy()` 已按引擎和原生 ID 限制重复执行，可以复用。但 `history::merge_session()` 目前只按原生 ID 去重，接入时应补上引擎条件。工作区历史筛选还应使用 Windows 路径规范化与精确目录匹配，不能直接假定 API 目录筛选等同当前产品语义。

## ACP 协议映射

建议每个执行轮次启动独立受管进程，保持当前取消隔离方式。握手后先创建或恢复会话，再应用本轮模型/档位/agent 配置，收到成功响应后才发送 prompt。

| ACP 消息 / 行为 | Conduit 处理 |
| --- | --- |
| `initialize` | 协商协议版本与能力；不声明尚未实现的客户端文件读写/terminal 能力 |
| `session/new` | 保存返回的原生 `sessionId`；读取 `configOptions` |
| `session/resume` | 能力声明支持时优先用于已有本地记录的续聊；1.19.2 源码不回放完整历史 |
| `session/load` | 用于历史导入或恢复回退；会回放历史，必须区分回放阶段和本轮输出，避免重复追加 |
| `session/set_config_option` | 设置 `model` / `effort` / `mode`；按返回的合法 options 验证，不静默降级 |
| `session/prompt` | 发送文本 content block；以该请求响应判定本轮结束，处理正常、取消、长度限制、拒绝和 RPC 错误 |
| `session/update` 的 `agent_message_chunk` | 映射 `Event::Delta`，使用稳定消息标识；工具更新同样按 ID 合并 |
| `tool_call` / `tool_call_update` | 映射工具输入、详情、状态和 diff；处理增量字段合并 |
| `session/request_permission` | 保留请求 ID、`toolCallId` 与 options；按 `kind=allow_once/reject_once` 查找实际 `optionId`，不硬编码展示名或默认选择永久允许 |
| `session/cancel` | 先发取消通知并短暂等待本轮结束，再退出与清理进程树；仍需保证 `Finished` 不早于清理屏障 |
| prompt response `usage` / `usage_update` | 区分轮次 token 用量和上下文占用；缓存读写、thought token 不能混成同一指标 |
| 未实现的反向 RPC | 返回明确 JSON-RPC 错误，不能忽略而让 CLI 永久等待 |

首期保留现有文件路径引用行为即可，无需同时引入 ACP 图片/嵌入内容。`agent_thought_chunk` 不直接改名当作公开推理摘要；第一阶段可跳过，后续另行确认内容语义与展示策略。

## 必须解决的兼容性问题

1. **权限是上线门槛。** OpenCode 的 agent（例如 build/plan）不等于 Conduit 的权限等级；plan 不能当成只读沙箱。现有 `automatic_approval()` 只处理已经发出的审批，对 CLI 配置为 allow 的工具无效。只读需要在启动时限制写入、shell、外部工具等能力；“工作区内修改”需明确目录外访问及 shell 的审批规则；完全权限要核对配置与 agent 层规则的合并顺序。OpenCode 源码会深度合并 `OPENCODE_PERMISSION`，仅覆盖 `"*"` 不能据此证明已有具体规则被覆盖。不能承诺 Codex 式文件系统隔离。
2. **模型和档位必须动态处理。** 模型标识是 `provider/model`，思考选项来自模型 variants，可出现 `minimal`、`max` 等。不要把当前 `Xhigh` 自动转换成 `max`。建议增加可选的引擎专属档位字符串，保留旧字段兼容；不支持的选择显示明确错误或要求重选。目录可能因工作区配置不同而变化，缓存不能只用引擎作 key。
3. **每轮重新应用会话配置。** 本次探测中，空会话设置 `effort=max`、`mode=plan` 后重启进程，load/resume 返回默认档位和 build。Conduit 应保存用户选择并在恢复后重设；该实测不代表已发送消息的会话一定丢失配置。
4. **历史导入不能阻塞首帧。** 首期后台枚举元数据、打开时再加载内容；当前搜索按本地消息全文工作，尚未加载的会话不能宣称可全文搜索。若需要完整搜索，单列后台索引任务。ACP 列表目前按 100 条分页且源码带 `roots: true`，需明确是否导入子会话并测试分页边界。
5. **辅助任务需要独立策略。** 自动命名和 Git 说明生成要使用工具禁用、无项目副作用的独立配置；要处理 OpenCode 保存辅助会话带来的历史污染。第一阶段可继续使用现有引擎作为辅助任务后端，界面明确其选择，不把 OpenCode 接入等同辅助任务已安全可用。
6. **并发与数据兼容需要回归。** 每轮独立进程会增加冷启动和数据库并发压力，不能由握手结果推断并发已经可用。新增序列化枚举值后，旧版本应用可能无法读取含 OpenCode 的状态文件，需保留备份和现有未知数据保护。

## 本机协议探测结果

采用临时工作区、独立 XDG 配置/数据/缓存目录、独立 `OPENCODE_DB`、禁用项目配置与自动更新。使用测试专属模型目录，端点指向本机不可用端口；**没有发送 `session/prompt`，模型调用数为 0**。以 `--pure` 避免加载外部插件。该设置只用于探测，不表示正式产品默认禁用用户插件。

实际执行文件：`D:\enviroments\nodeJS\npm-global\node_modules\opencode-ai\bin\opencode.exe`。本机虽然 `Get-Command opencode` 首先命中 `.ps1`，同时存在原生 `.exe`，可纳入现有启动器支持范围。

| 检查 | 结果 |
| --- | --- |
| `--version` 和 acp/run/serve help | 通过，版本 1.19.2 |
| ACP initialize | 通过，协议版本 1；声明 load/list/resume 等能力 |
| session/new | 通过，返回真实原生 ID 和模型/档位/模式目录 |
| session/list | 通过，返回隔离会话及 cwd |
| 指定模型配置、切换 `effort=max` 与 `mode=plan` | 通过，返回值确认生效；模型设置使用创建时相同的 ID，未测试更换真实模型；档位目录为测试配置注入的 variants，不代表真实模型都支持 |
| 结束原进程后新进程 session/load | 通过，原生 ID 可恢复；仅验证空会话 |
| 再启动第三个进程后直接 session/resume | 通过，无需先 load；仅验证空会话 |
| 带消息上下文续聊、文本流、工具卡片、真实审批与拒绝 | 未测 |
| 执行中/待审批取消、两个会话并行、CLI 与 Conduit 同时访问 | 未测 |

最后一组探测的三次握手耗时约 1.03–1.22 秒，创建/冷加载/冷恢复约 0.58–0.64 秒；仅少量本机启动样本，含隔离配置且无真实插件/MCP，不能作为生产性能基准。

原始探测结果保存在本次机器临时目录：`C:\Users\leo.li5\AppData\Local\Temp\conduit-opencode-acp-review-a__xiu9h\probe-result.json`。探测脚本及核对源码在 `C:\Users\leo.li5\AppData\Local\Temp\conduit-opencode-review-1.19.2`，不依赖这些临时文件运行应用。

## 建议实施顺序与投入

按熟悉本仓库的一名开发者估算，以下是规划范围，不是交付承诺：

- **基础接入：3–5 个开发日。** 引擎/路径/设置、ACP 状态机、纯文本流、原生 ID 续接、模型选择和基础取消。形成可验证的纵向原型，完成前不开放到常规使用入口。
- **达到现有核心能力：额外 5–8 个开发日。** 权限规则及真实允许/拒绝、待审批取消、动态档位、按需历史、并发隔离、状态兼容和 UI 验收。
- **另行估算。** 自定义 Provider/CCS 完整映射、所有导入历史全文索引、OpenCode 专属辅助任务、问答型工具与图片等扩展。若要求这些在首版齐全，上述核心能力预算不覆盖。

第一阶段先跟随 OpenCode 自身已登录的 Provider 配置；第二阶段再做 Conduit Provider 注册表的完整桥接。避免在协议未验证前同时扩大凭证、Provider 和历史数据库适配范围。

完成标准至少包括：真实两轮续聊且原生 ID 不变；重启 Conduit 后保持上下文；工具允许后生效、拒绝后不落盘；只读和目录外访问规则符合 UI 说明；执行及待审批时取消不影响另一个会话；历史回放不重复；模型/档位恢复不静默变化；窗口关闭后受管进程树清空。最后执行仓库常规 fmt、clippy、test、build 和 UI 验收。

## 依据

官方文档：`https://opencode.ai/docs/acp/`、`https://opencode.ai/docs/server/`、`https://opencode.ai/docs/permissions/`。本次已下载并核对；具体版本行为以官方仓库 `anomalyco/opencode` 的 **v1.19.2** 源码与本机探测为准：

- `packages/opencode/src/cli/cmd/acp.ts`：stdio NDJSON 和内部服务。
- `packages/opencode/src/cli/cmd/run.ts`：非交互 run 的默认拒绝 / auto 允许。
- `packages/opencode/src/acp/service.ts`、`event.ts`：会话、恢复、回放、配置与事件。
- `packages/opencode/src/acp/config-option.ts`、`permission.ts`、`usage.ts`：动态档位、审批选项及用量语义。
- `packages/opencode/src/config/config.ts`、`permission/index.ts`：配置深度合并和权限规则评估。
- `packages/core/src/database/path.ts`：原生数据库位置与覆盖配置。

版本源码根地址：`https://github.com/anomalyco/opencode/tree/v1.19.2/`。
