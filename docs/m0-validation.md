# Chat M0 验证记录

更新日期：2026-10-23。结论：**双 CLI Chat、原生续接、审批协议回归（含 Claude 待审批取消与 Codex 权限扩展审批映射）、CLI / cc-switch 模型只读同步、工作区只读 Git 变更面板已验证；会话级模型选择已接通协议层；已完成七轮 UI 自动预验并修复设置弹窗窄窗裁切；审批弹窗点击与视觉、中文输入、复制、滚动跟随与 DPI 等界面项仍未人工验收。**

本记录取代旧终端原型验证记录。旧 ConPTY 测试、终端截图与 ANSI 能力不作为当前 Chat 实现的验收证据。产品、技术范围分别见 [产品设计](product-design.md) 和 [技术设计](technical-design.md)。

## 环境

| 项目 | 验证值 |
| --- | --- |
| 平台 | Windows 11 x64，原生 exe |
| Rust / Cargo | 1.99.1，MSVC |
| GPUI Kit | 0.7.3，revision `2963d6d880b9dffe59b48b0533bdc83c03201209` |
| GPUI | gpui-pre 0.3.9 |
| Windows API | windows-sys 0.61.2 |
| Claude Code | 2.1.301 |
| Codex | 0.163.0 |
| 当前协议 | Codex app-server stdio；Claude stream-json + stdio control |

本机 Claude 使用自定义 Anthropic 兼容网关。默认模型别名映射到网关不支持的 `gpt-6.1-sol[1M]`，Sonnet 4.5 的 1M 路由返回 503；本轮审批回归通过 `CONDUIT_CLAUDE_TEST_MODEL` 显式使用 `claude-haiku-4-5-20251001`，未修改全局配置。2026-10-19 的 Claude 180 秒超时未直接复现；本轮首次失败先表现为模型不可用，改用显式模型后通过。

## 已执行验证

| 项目 | 结果 | 证据与边界 |
| --- | --- | --- |
| 构建 | 通过 | `cargo build --locked`，生成 `target/debug/conduit.exe` |
| 格式 | 通过 | `cargo fmt --check` |
| 静态检查 | 通过 | `cargo clippy --locked --all-targets -- -D warnings` |
| 常规测试 | 通过 | 43 passed，0 failed，9 ignored（2026-10-22 本轮）；ignored 为需要本机 CLI 的集成测试 |
| 模型配置只读同步 | 单测与界面冒烟通过；交互待人工验收 | 解析 Claude settings 与 Codex config/catalog，验证去重、相对目录、隐藏项、缺失/损坏配置；不读取密钥。新增设置“模型同步”菜单，分别展示 CLI 当前模型与 CCS Provider 模型；输入区选择 `gpt-6.1-sol[1M]` 后写入隔离 `state.json`，标签与弹窗状态更新。仅 UI 自动化观察，未人工点击 |
| cc-switch 模型目录 | 单测与真实数据库查询通过；界面待人工验收 | 只读打开 `~/.cc-switch/cc-switch.db`，仅投影模型 ID、显示名、上下文与可见性；本机读回 Claude 6 个 Provider / 11 个模型、Codex 3 个 Provider / 12 个模型，正确标记 anyrouter 与公司为当前 Provider。夹具含伪造 Token 与端点，断言未进入结果；未修改数据库 |
| 模型目录合并 | 单测通过 | CCS 当前 Provider 模型并入会话选择器；同名模型合并备注，新增模型带 `CCS · Provider` 来源，其他 Provider 不进入选择器 |
| 模型覆盖协议映射 | 单测通过 | Claude 启动参数包含 `--model <id>`；Codex `thread/start`、`thread/resume`、`turn/start` 均携带 `model`；未选择时不注入覆盖 |
| 本机版本检测 | 通过，2026-10-17 单独执行 | `installed_clis_report_versions` 读回 Claude 2.1.301 / Codex 0.163.0；不调用模型 |
| 双 CLI 握手 | 通过，2026-10-16 单独执行 | 两款 CLI 初始化结构化协议成功，不调用模型 |
| Codex 真实对话与续接 | 通过，2026-10-16 单独执行 | 两轮跨进程、相同 thread ID，第二轮能够回答上一轮指定 token |
| Claude 真实对话与续接 | 通过，2026-10-16 单独执行 | 两轮跨进程、相同 session ID，第二轮能够回答上一轮指定 token |
| 回复去重 | 单测通过 | 文本 delta 合并，完整快照替换；同名 item 跨轮隔离 |
| 工具状态 | 单测通过 | 工具输入、流式输出和最终结果分开保存；取消仅标记本轮未完成项，保留部分结果 |
| 审批映射 | 单测通过 | 本地审批 ticket 与 CLI wire ID 分离；精确请求 ID；文件修改审批保留决策约束；多个 ticket 独立排队；撤销后陈旧 ticket 不可授权；Claude 原输入保留；未知交互返回错误；Codex `serverRequest/resolved` 可撤销待审批 |
| Codex 真实审批拒绝 | 通过，2026-10-19；2026-10-22 合并回归再次通过 | 使用测试专用 `untrusted + read-only` 参数，临时目录中收到真实 `item/commandExecution/requestApproval` 后拒绝，收到对应 resolved，目录无变化；2026-10-22 约 15 秒。生产参数不变 |
| Claude 真实工具审批允许/拒绝 | 通过，2026-10-22 单独及合并执行 | 测试专用 `manual` 模式 + 显式 `claude-haiku-4-5-20251001`；Write 工具先拒绝（文件未创建）再允许（文件内容匹配），两次均收到对应 resolved；单项约 151 秒。生产默认权限模式未改 |
| Codex 文件修改审批允许/拒绝 | 通过，2026-10-22 单独及合并执行 | 测试专用 `untrusted + read-only`；`apply_patch` 先拒绝（目录无变化）再允许（文件内容匹配）；约 21 秒。允许后等待模型最终回复可能超过 180 秒，测试以审批已决且文件落盘为判定点 |
| Codex 待审批取消 | 通过，2026-10-22 单独及合并执行 | 收到真实审批请求后取消；Finished(cancelled)、无 ApprovalResolved、目录无变化；约 13 秒 |
| 多审批排队 | 单测通过，2026-10-22 | 运行时协议夹具连续发出两个审批请求，生成独立本地 ticket，两个决策分别映射回 wire ID；非真实 CLI 并发 |
| 运行时取消隔离 | 单测通过，2026-10-22 | 两个真实 PowerShell 协议夹具并行，取消第一个后仅第一个 `Finished(cancelled)`，第二个继续并正常完成；验证多工作区取消不互相终止进程树 |
| 日用交互修复 | 编译、单测与启动检查通过；交互待人工验收 | Ctrl+Enter 改用 `ConduitComposer > Input` 绑定避免被输入框换行抢占；切换会话、发送、审批、取消后焦点回到输入框；后台会话事件不再拉动当前会话滚动；审批详情高度增至视口 40%（120–360px）；只可拒绝时显示说明；只读存储禁用输入框 |
| 恢复身份 | 单测通过 | 不同原生 ID 拒绝；Codex 忽略其他 thread / turn 通知 |
| 进程清理 | 单测通过 | 真实 PowerShell 父子进程；显式 terminate 和 Child Drop 都结束后代；核对 Job 归零与根进程退出 |
| 运行时取消与异常 | 单测通过 | 经 ChatRuntime 启动真实 PowerShell 协议夹具，取消后的 Finished 到达时子进程已退出；提前退出和非法 JSON 均报告失败 |
| 版本检测边界 | 单测通过 | 无效显式路径不回退、拒绝脚本、未知与预发行版本不标已验证；检测超时、取消、输出超限、非零退出均停止并返回错误 |
| 参数与目录 | 单测通过 | Unicode / 空格目录、空参数、内嵌引号、末尾反斜线经 CreateProcessW 完整传入 |
| 存储与迁移 | 单测通过 | 原子替换、坏文件保留、重复 ID 拒绝、v1 仅迁移元数据、native ID / 中文内容 / CLI 配置保留、重启中断提示不会重复落盘 |
| 启动失败 | 单测通过 | 不存在的 exe 产生 Finished(error)，不会伪装为模型回复 |
| Chat 与设置页启动 | 启动检查通过 | `scripts/smoke.ps1` 的 chat / settings 模式均通过：隔离数据、非零窗口句柄、Responding=True，首帧后继续存活。另保留此前合成历史启动检查；不等同视觉、点击或输入验收 |
| UI 自动预验 | 部分通过 | 隔离数据 + `PrintWindow` 截图 + UI Automation 驱动，覆盖清单 3/4/7/10/11/12 中可自动观察的部分；发现设置弹窗窄窗底部裁切并修复；清单 1/2/5/6/8/9 仍需人工。证据与方法见 [UI 自动预验记录](m0-ui-acceptance-run.md) |
| Claude 待审批取消 | 通过，2026-10-22 | 真实 CLI 测试专用 `manual` 模式 + 显式 `claude-haiku-4-5-20251001`；收到 `can_use_tool` 后取消，Finished(cancelled)、无 ApprovalResolved、临时目录无变化；约 12 秒 |
| 生产参数 Codex 审批弹窗 | 自动预验通过，2026-10-22 | 真实 Codex 0.163.0 在生产参数 `on-request` + `workspace-write` 下请求越界写文件，界面显示审批卡片；脚本点击“拒绝”后探针文件不存在，点击“允许本次”后文件内容为 `ok`、模型回复 `APPROVAL_TEST_DONE`；均为 UI Automation 合成点击，人工点击手感与视觉仍待确认。证据见 [UI 自动预验记录](m0-ui-acceptance-run.md) |
| 历史整理 | 单测与界面预验通过，2026-10-22 | 会话可重命名（输入对话框）、归档/取消归档（侧栏“已归档”切换）、删除（二次确认）；搜索为弹窗，按标题与消息内容匹配并在标题/摘要中高亮命中词；工作区行提供“在此工作区新建会话”与“移除工作区”（菜单向右弹出，只移出应用路径，保留磁盘文件与 CLI 历史）。每个工作区默认显示最近 5 条会话，设置页可调 1–30，超出可展开。`archived` 随 v2 状态保存，旧文件缺省为未归档。43 passed；界面证据见 [UI 自动预验记录](m0-ui-acceptance-run.md) |
| 输入与引擎交互 | 界面预验通过，2026-10-22 | Enter 发送、Ctrl/Shift+Enter 换行（合成按键验证：Ctrl+Enter 插入换行、Enter 触发发送并生成用户消息）；移除输入区快捷键提示；引擎下拉移到输入区左下角并向上弹出，切换后同步切换到该引擎最近会话，右上角模型选择器同步。证据见 [UI 自动预验记录](m0-ui-acceptance-run.md) |
| Codex 权限扩展审批映射 | 单测通过 | `item/permissions/requestApproval` 映射为审批；允许回传请求的 profile 且 `scope=turn`，拒绝回空 profile（协议无 deny 枚举，空 profile 为安全拒绝）；真实 CLI 触发未回归 |

真实对话测试各发送两个合成提示，不包含用户项目内容：第一轮要求回复 `CONDUIT_CHAT_OK`，第二轮在新进程恢复同一原生会话并询问上一轮 token。未授予工具权限，未修改实际项目文件或认证信息。本次收尾未重复执行这些模型请求。

## 可复现命令

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
cargo build --locked
./scripts/smoke.ps1 -View chat
./scripts/smoke.ps1 -View settings
```

本机 CLI 集成测试按需单独运行。版本和握手不调用模型；对话与审批回归会调用模型，可能消耗账户额度；默认测试不执行。Claude 审批回归需用 `CONDUIT_CLAUDE_TEST_MODEL` 指定本机可用模型，测试本身使用 `manual` 权限模式，不修改生产默认模式。

```powershell
cargo test --locked installed_clis_report_versions -- --ignored --nocapture --test-threads=1
cargo test --locked installed_cli_protocol_handshakes -- --ignored --nocapture --test-threads=1
cargo test --locked real_codex_chat_and_resume -- --ignored --nocapture --test-threads=1
cargo test --locked real_claude_chat_and_resume -- --ignored --nocapture --test-threads=1
cargo test --locked real_codex_command_approval_can_be_denied -- --ignored --nocapture --test-threads=1
cargo test --locked real_codex_file_change_approval_allow_and_deny -- --ignored --nocapture --test-threads=1
cargo test --locked real_codex_cancel_during_pending_approval -- --ignored --nocapture --test-threads=1
$env:CONDUIT_CLAUDE_TEST_MODEL='<available-model>'
cargo test --locked real_claude_tool_approval_allow_and_deny -- --ignored --nocapture --test-threads=1
cargo test --locked real_claude_cancel_during_pending_approval -- --ignored --nocapture --test-threads=1
# 本轮合并回归：4 个审批测试通过，327.54 秒
$env:CONDUIT_CLAUDE_TEST_MODEL='claude-haiku-4-5-20251001'
cargo test --locked approval -- --ignored --nocapture --test-threads=1
```

早期带合成消息的 GUI 启动使用独立 `CONDUIT_DATA_DIR=target/chat-ui-smoke`；新增冒烟脚本每次在 `target/smoke-chat-*` 或 `target/smoke-settings-*` 创建空白隔离数据。其中消息明确写为合成演示，不能作为真实 CLI 输出或真实 native ID 展示。没有写入正式 `%LOCALAPPDATA%\Conduit` 数据。启动检查结束后停止该无活动任务的测试实例；没有把强制停止替代正常关窗验收。

## 手工 UI 验收清单

截至本记录，以下项目**尚未执行**，不得预先标记为通过。建议在隔离 `CONDUIT_DATA_DIR` 下逐项执行，记录构建时间、实际缩放和结果；涉及 CLI 时只使用临时工作区与合成提示。2026-10-22 的可自动观察部分已先行预验（见 [UI 自动预验记录](m0-ui-acceptance-run.md)），但本清单仍需逐项人工执行与签署；可照 [人工验收交接单](m0-manual-acceptance-guide.md) 一次走完。2026-10-22 起主窗口为自定义标题栏、左右面板可拖拽调宽，验收 7/8 项时按当前布局核对。

| # | 场景 | 操作 | 通过标准 |
| --- | --- | --- | --- |
| 1 | Ctrl+Enter | 聚焦输入框输入两行：第一行后按 Enter，再按 Ctrl+Enter | Enter 只换行；Ctrl+Enter 只发送一次，输入框清空并保持焦点 |
| 2 | 中文输入法 | 用微软拼音逐字输入、选候选、在组合中退格后再发送 | 组合文本与候选窗正确；组合期间按键不误触发送；发送后无乱码或重复 |
| 3 | Emoji 与复制 | 用 Win+. 输入 emoji；发送含标题、列表、行内代码和围栏代码块的回复；鼠标选择回复后 Ctrl+C；再点击代码块复制 | Emoji 正常显示；选区与剪贴板内容一致；代码块复制内容精确，不含围栏 |
| 4 | 滚动跟随 | 发送长回复，先停留底部，再上滚阅读；回复结束后回到底部 | 停留底部时跟随新内容；上滚后不被拉回；回到底部后继续跟随 |
| 5 | 审批弹窗 | 在临时工作区发起需要审批的无害操作，分别点击允许本次、拒绝；再触发长详情与多请求队列 | 按钮只提交一次；状态依次为“正在提交审批…/正在执行…”；拒绝后目标无变化；详情可滚动；队列序号正确；只可拒绝时显示提示。若生产参数不产生审批，记录该场景不可复现，不伪造通过 |
| 6 | 焦点恢复 | 依次切换会话、发送消息、允许/拒绝审批、取消任务后直接打字 | 每次按键都进入输入框，无需再次点击 |
| 7 | 最小窗口与 DPI | 窗口缩至 850×500；在 100%、125%、150%、200% 缩放下检查会话列表、输入框、审批覆盖层和设置页 | 无控件裁切、重叠或不可点击；文字与审批详情可读；滚动可用。切换缩放若需重新登录，记录实际测试值，未测项保持未通过 |
| 8 | 多工作区 | 两个工作区各启动会话并同时运行；在 A 中上滚阅读，等 B 完成；再取消 A | B 完成不改变 A 的滚动位置；取消 A 不影响 B；侧栏状态与各自任务一致 |
| 9 | 快速关闭 | 有运行中任务和待审批请求时关闭窗口；另测无任务正常关窗 | 应用退出；任务管理器无遗留 conduit / claude / codex 子进程；重新启动可加载数据，未完成运行标记为中断 |
| 10 | 设置页 | 打开设置页，选择或清除 CLI 路径，执行检测并取消一次 | 点击命中正确控件；取消后检测进程退出；来源与检测结果一致 |
| 11 | 启动冒烟 | 运行 `scripts/smoke.ps1 -View chat` 与 `-View settings` | 隔离数据下窗口出现并持续响应；该脚本不覆盖上述输入与点击验收 |
| 12 | 模型选择与同步 | 打开设置左栏“模型同步”，核对 Claude / Codex 的 CLI 当前模型、CCS Provider 分组与当前标记；在会话中打开模型选择器，切换模型后发送一条合成消息；新建同引擎会话观察默认模型；刷新后再看列表 | 菜单可进入且滚动正常；只展示配置来源、模型清单与非敏感信息，无密钥；CCS 当前 Provider 模型出现在选择器，其他 Provider 不出现；选择器可打开/关闭，标签随选择更新；新会话继承 Provider 当前默认；选择只影响当前会话且下一条消息生效；无配置时显示原因而非空白 |

## 验证中修复

- 将终端主界面改为 Chat，并移除活跃的终端依赖。
- 流式片段与快照使用相同消息键，避免重复显示；不同轮次独立。
- 切换会话或添加目录时隔离草稿，工具详情与错误提示独立。
- 恢复 ID 不一致时停止，不静默建立新上下文。
- 取消或关闭时移除待审批入口；侧栏显示执行、等待审批和停止中状态。
- 结束与重启不让工具卡片永久显示运行中。
- 进程清理必须同时确认 Job 活动数归零和根进程 signaled；测试清理临时脚本也等待该屏障。
- 窄窗详情改为覆盖侧栏，长审批详情滚动，队列逐项处理；审批详情最大高度调整为视口 40%（120–360px），CLI 未提供“允许本次”时明确提示只能拒绝。
- Ctrl+Enter 原先被 gpui-kit 输入框更深层的 `Input` 上下文 `secondary-enter` 绑定抢占，表现为插入换行而不发送；现以 `ConduitComposer > Input` 后代上下文注册发送绑定，并保留组合根绑定作为回退，处理器停止继续传播。
- 切换会话、发送、提交审批和取消任务后主动把焦点交还输入框；只读存储状态下禁用输入框，避免输入后才失败。
- 后台会话的事件不再驱动当前可见会话的滚动：仅当本批事件包含当前选中会话时才执行自动跟随，其他工作区完成不会把正在阅读的对话拉到底部。
- 新增双运行时取消隔离回归：两个真实 PowerShell 协议夹具并行运行，取消其中一个后另一个不受影响并正常完成。
- 新增 CLI 设置页、路径选择与来源展示、受管版本检测；配置结果与检测路径绑定，避免过期结果。
- 新增只读模型同步：Claude 读取 `settings.json` 模型字段，Codex 读取 `config.toml` 与可选 catalog；另以只读 SQLite 查询 cc-switch `providers` 的模型字段。不查询凭证、不回写任一配置。模型选择按会话保存，仅下一条消息生效；旧会话保持 CLI 默认。
- 设置左栏新增“模型同步”独立页，展示 CLI 当前模型与按 Provider 分组的 CCS 模型；当前 Provider 的 CCS 模型合并进会话选择器，其他 Provider 仅展示，避免跨 Provider 误用。
- 协议循环的 panic 在持有 Child 的外层捕获，仍经过验证清理后才解除工作区占用。
- 工具输入和输出分离保存，审批使用一次性本地 ticket；Codex resolved 通知撤销陈旧请求。
- 审批集成测试在 Finished 后直接删除临时目录会与退出中的 CLI 抢占；测试改为显式释放运行时并重试清理，避免断言已通过却因清理失败。
- Claude 审批测试增加测试专用模型与 `manual` 权限模式参数，用 Write 工具落盘验证允许/拒绝；生产启动参数不变。
- 新增 Codex 文件修改审批允许/拒绝、待审批取消和运行时双审批排队回归。
- 新设置页引入后实际启动暴露 Debug 主线程栈溢出。检查 PE 头确认原栈预留为 1MiB；通过 Windows MSVC 链接参数设为 8MiB，重新核对 PE 头，Chat 与设置页启动冒烟均通过。编译和单测通过不能替代实际启动检查。
- UI 自动预验发现：设置弹窗固定 560px 高，对话框层在 850×500 最小窗口下会裁切超出视口的内容，底部“完成”按钮不可见。改为按视口高度自适应（`视口高度 - 96px`，限制 360–560px），正文区域自行滚动；窄窗与常窗截图均复验通过。
- Claude 待审批取消补齐真实回归：收到 `can_use_tool` 后取消，任务以 cancelled 结束、审批不会被迟到解析、目录无变化。
- Codex `item/permissions/requestApproval` 纳入审批队列：这是 0.163.0 协议中除命令/文件修改之外的权限扩展请求；允许时按请求 profile 授予本轮权限，拒绝时回空 profile，未知请求仍返回 `-32601` 而不自动放行。
- 界面按反馈完成一轮改进：移除 Windows 原生标题栏改用自定义标题栏（品牌区、拖动与最小化/最大化/关闭命中区）；侧栏与执行详情改为可拖拽面板（侧栏 200–420px，详情 240–480px，窄窗详情覆盖）；审批卡片宽度对齐对话文字列并重做视觉；输入区右上角“最新消息”、底部新增横向预留工具栏；设置弹窗垂直居中，模型目录与 cc-switch 读取移到后台线程，打开不再阻塞首帧，入场动画改为轻量 fade。
- 用生产参数（`on-request` + `workspace-write`）在隔离工作区跑真实 Codex 审批：越界写文件触发真实审批请求，脚本点击拒绝后无文件落盘，点击允许后探针文件内容正确；此前“生产参数可能不产生审批”的未知项已获得一次真实证据（合成点击，非人工）。
- 针对第二轮反馈继续修正：Debug 构建下窗口拖动会整窗重绘，依赖未优化导致逐帧卡顿，dev profile 改为依赖 `opt-level = 2`、应用本体 `opt-level = 1`；审批卡片详情默认折叠并可用“展开详情/收起”切换；设置弹窗在模型读取移出 UI 线程后恢复默认滑入入场动画；失败信息若已出现在助手文本中则不再重复追加结果通知，相邻重复的 completed 助手文本块合并（新增两个单测）。
- 定位“AI 回复重复”的真实原因：自定义网关的流式事件包含隐藏 thinking 块，使文本块的 stream index 与 assistant 快照中的可视块序号错位，同一段文本被存成 `...-1` 与 `...-0` 两条（工具卡片与审批通知随之成对出现）。协议改为按“文本块序号”对齐流式增量与快照；加载旧数据时清理相邻重复助手文本和已由助手文本承载的失败通知。新增 3 个协议/存储测试（共 38 passed），旧会话在下次加载时即不再重复显示。
- 实现 P1 历史整理：侧栏新增会话搜索（标题与消息正文）、会话行操作菜单（重命名对话框、归档/取消归档、删除二次确认）和“已归档”显示开关；`SessionRecord.archived` 为兼容 v2 的可选字段（缺省 false）。新增搜索匹配与归档往返测试（共 40 passed）。
- 侧栏信息架构调整：移除侧栏的 Claude/Codex 新建入口，改为顶部“新建会话/搜索”图标行；新建会话引擎切换到输入区底部工具栏左侧，切换不改变已打开会话；工作区行新增“在此工作区新建会话”图标与“移除工作区”菜单（只移除应用内路径）；搜索改为弹窗，命中列表在标题与摘要中高亮。自动预验捕获到多字节文本下高亮范围映射越界导致进程 panic，已改为逐字节映射并新增 Unicode/摘要测试（共 42 passed）。
- 交互细节第二轮：工作区菜单改为向右弹出；每个工作区默认只渲染最近 N 条会话（`UiSettings.sessions_per_workspace`，默认 5，设置页步进可调 1–30，超出可展开/收起）；Enter 改为发送、Ctrl/Shift+Enter 换行；移除输入区快捷键提示，引擎选择改为输入区左下角的下拉（向上弹出），切换引擎即切到该工作区该引擎最近的未归档会话，右上角模型选择器随之同步。预验中发现工作区行“+”按钮的点击会冒泡到行选择、覆盖新建后的选中状态，已加 `stop_propagation` 修正（共 43 passed）。
- 第七轮界面反馈（2026-10-23）：工作区行与会话行“⋯”菜单触发按钮改为同一垂直位置（会话行右内边距 4→8px）；会话行菜单改为向右弹出（`Anchor::LeftCenter`）并与工作区菜单对齐；移除输入区右上角“最新消息”文本按钮，改为消息区右下角（输入区右上角上方）的 Pin 图标，仅在消息列表未停留底部时自动显示，点击回到底部并立即隐藏（复用 33ms 轮询的“距底部 < 60px”判定）；移除模型选择器右侧的引擎圆点与引擎名称；为图标按钮、菜单项与弹窗按钮补 `cursor_pointer`（禁用态不加）；工具调用卡片标题去掉未嵌入的 Terminal 图标占位后左右内边距一致；设置左侧新增 CLI 页，通用页只保留本机数据与界面设置；应用改用 `gpui_kit::assets::AllAssets` 完整图标目录以渲染 Pin 图标。`cargo fmt --check`、`clippy -D warnings`、`cargo test`（43 passed）与双视图启动冒烟通过；截图见[UI 自动预验第七轮](m0-ui-acceptance-run.md#第七轮2026-10-23-界面细节反馈)。
- 只读 Git 变更面板（2026-10-23）：新增 `src/git.rs`，只执行 `rev-parse` / `status --porcelain=v1 -z` / `diff` / `diff --cached`，status 与 diff 加 `--no-optional-locks`；后台线程 + Job 进程树、15 秒超时、单段 diff 200 KiB 与 2000 文件上限。右侧执行详情改为“工具详情 / Git 变更”两页，展示分支与暂存/未暂存/未跟踪数量和彩色 diff；打开、点刷新或一轮 CLI 任务结束时重读，切换工作区丢弃旧快照，非 Git 目录或未安装 Git 显示可重试提示。新增 5 个 git 单测（状态解析、diff 分类、截断边界、计数、真实临时仓库只读指纹），`cargo test` 48 passed；自动预验截图见[第八轮](m0-ui-acceptance-run.md#第八轮2026-10-23-只读-git-变更面板)。
- 第九轮界面反馈（2026-10-23）：输入区统一为单一 composer，会话 / 无会话 / 无工作区三种状态宽高一致；无会话时发送会用上次使用的引擎（`UiSettings.last_provider`，随本地状态持久化）自动新建会话并发送，移除“新建会话”按钮；新建会话前先校验 CLI 路径与工作区，失败不会留下空会话；聊天左上角新增工作区选择器，可切换已有工作区或添加目录；会话标题右侧的“变更 / 详情”文本按钮合并为单个侧边栏图标，右侧面板保留“工具详情 / Git 变更”页签并支持并排 split；右侧侧边栏改为应用自管宽度，拖动左边缘按指针位置 1:1 更新（修复原先拖动时宽度跳变），宽度上限为剩余内容区的一半；窄窗仍以覆盖层显示。`cargo test` 48 passed（新增 last_provider 往返断言），fmt/clippy/build 与双视图冒烟通过；截图见[第九轮](m0-ui-acceptance-run.md#第九轮2026-10-23-输入区与侧边栏重构)。

## 未完成验收与具体限制

1. **真实工具审批**：Codex 命令拒绝、文件修改允许/拒绝、待审批取消，Claude `can_use_tool` 允许/拒绝与待审批取消均已通过。仍需审批弹窗的点击与视觉人工验收、真实 CLI 并发多审批排队、Codex 网络/权限扩展请求类型的真实回归（`item/permissions/requestApproval` 目前仅单测映射）；Claude 回归使用测试专用 `manual` 模式，生产默认模式仍依赖本机 CLI 权限配置。
2. **UI 输入与渲染**：Ctrl+Enter 的绑定抢占已修复，焦点恢复、后台滚动隔离、审批详情高度与只可拒绝提示也已补齐；设置弹窗窄窗裁切已修复。自动预验只覆盖可自动观察部分，仍需按上表人工验证实际按键、微软拼音、emoji、文本复制、Markdown 代码块、滚动跟随、最小窗口和 100%–200% DPI；新增的工作区选择器、无会话输入建会话、侧边栏 split 与拖动跟手也需人工手感验收。
3. **并行与稳定性**：运行时取消隔离已有自动回归；仍需人工验证双工作区持续运行、同工作区互斥、高输出、长对话内存、应用快速关闭和异常终止后清理。
4. **恢复异常**：CLI 原生历史被删除、登录过期、升级改变协议、本地磁盘满及断电后的实际体验。
5. **界面功能**：只读 Git Diff 已实现并完成自动预验，仍需人工验收大仓库 / 宽 diff 的渲染、滚动与真实工作区刷新；模型启动参数设置、更多交互类型尚未实现。历史整理（重命名、归档、删除、搜索）已实现；图形化 CLI 路径配置、版本检测、CLI / CCS 模型同步已实现；设置页、模型同步页与 Git 面板的自动预验通过，点击、文件选择、模型选择器交互与布局仍待人工验收。
6. **平台**：macOS、Linux、WSL、npm 包装脚本未实现。

Windows Computer Use 的 native pipe 连续连接失败；按恢复流程重试并重置会话后仍返回“系统找不到指定的文件 (os error 2)”。因此没有使用旧坐标或其他 UI 自动化绕过，也没有把界面验收标为通过。可运行 `cargo run --locked` 手工完成上述 UI 验证。

架构图保留 `.mmd` 源文件；本机未发现 mmdc，Kroki 导出返回 HTTP 403，因此没有交付声称已渲染验证的图片。该限制不影响应用构建。

## 进入日用试验的门槛

自动侧已补 Ctrl+Enter 绑定、焦点恢复、后台滚动隔离、审批详情可读性、运行时取消隔离、Claude 待审批取消回归，并完成一轮 UI 自动预验（含设置弹窗窄窗修复）；手工侧仍需完成上表全部项目，重点是审批弹窗点击与视觉、取消与正常关窗、多工作区隔离、中文输入和阅读。任何错目录、串上下文、错授权或受管进程残留均阻断扩展。通过后再开展两周个人试用，不把本次自动测试等同完整 MVP 验收。



## Git 提交区增量验证（2026-10-23）

Git 专属侧栏新增固定提交区、AI 提交说明生成、Commit / Commit and Push，以及底栏待推送 badge；仅提交已暂存变更。常规测试 55 通过，fmt/clippy/build 与 Chat / 设置 / 隔离 Git 工作区启动检查通过；本地 bare remote 回归覆盖成功推送、pending 计数与提交成功/推送失败。Claude/Codex 生成辅助会话初始化均单独验证成功，未发送模型请求。真实 AI 出文、认证推送与 UI 点击/视觉仍待验收；细节见 [Git 界面设计](git-ui-design.md)。此前的“只读 Git”记录描述当时版本，当前版本仅状态查询保持只读，写操作由用户显式按钮触发。

## Git 界面收尾验证（2026-10-23）

提交区改为 Commit Message，模式菜单整行左对齐；设置新增 Git 多行生成提示词（自动保存、恢复默认、留空回退）；底栏归属对话内容列；引擎标识使用本地嵌入的 Claude / OpenAI SVG。常规测试 56 passed、10 ignored，fmt / Clippy / build 通过，Chat / 设置 / 隔离 Git 工作区启动检查通过。实际 UI 点击、视觉与真实模型生成尚未验证。

## 聊天导航与工具展开（2026-10-23）

- 最新消息入口改为 ArrowDown 原生图标按钮，保留滚动离开底部后出现、返回后隐藏的行为，并提供 tooltip 与可访问名称。
- 用户消息大纲已按反馈调整到聊天内容区消息列左侧：贴着居中的消息列并保留间距，整体垂直居中；纵向短线向右对齐，长度随用户消息摘要变化并限制最大值，悬停摘要与当前阅读位置高亮；按会话和消息 key 定位，处理重复文本、中英文多行摘要、长内容和空消息。点击后定位原始消息列表项并高亮两秒，避免当次流式事件抢回滚动位置。
- 工具调用卡片标题切换展开/收起；下方固定 240px 独立滚动正文，纯文本可选择与复制，保留换行缩进、清理 ANSI/OSC 控制码、格式化完整 JSON，部分流式 JSON 原样保留；另有明确图标打开右侧详情。
- `cargo test --locked -- --test-threads=1`：58 passed、0 failed、10 ignored。fmt、Clippy（all-targets / -D warnings）与 build 通过；Chat / 设置启动冒烟通过。
- 12 轮用户消息、工具卡片和长回复的隔离启动检查发现最新消息按钮重复 hover 样式会触发 GPUI 断言，已移除重复样式；重新构建后的 12 轮长聊天窗口保持响应，stderr 为空。实际展开、复制、大纲点击定位和窄窗视觉仍待人工验收；未通过脚本模拟点击或抓图替代不可用的 Computer Use。

### 大纲位置、工具滚动与审批标签修正（2026-10-23）

- 移除标题栏大纲弹出入口，改用内容区左侧独立的纵向短线导航，悬停显示摘要，点击跳转；无用户发言时隐藏。
- 工具正文使用自然高度文本列与固定 240px block 视口，独立 ScrollHandle 和滚动条。GPUI 回归发现滚轮默认同时移动外层聊天列表，已在正文处停止继续传播。
- 审批结果单独存储并显示在对应工具卡片左侧；新增工具 ID 精确关联、无 ID 输入匹配、跨轮隔离、早到审批合并及旧审批 Notice 迁移往返回归。
- `cargo test --locked --features ui-tests -- --test-threads=1`：63 passed、0 failed、10 ignored。包含实际 GPUI 布局与滚轮事件验证：200 行文本产生溢出、卡片内滚动不移动聊天或其他卡片、卡片外滚动仍移动聊天。该测试不检查渲染像素。
- fmt、严格 Clippy（all-targets / ui-tests / -D warnings）、生产配置 build 通过。Chat / 设置启动冒烟及 12 轮含旧审批提示的隔离聊天启动检查通过，窗口响应且 stderr 为空；视觉、复制和大纲点击手感仍待人工验收。
- 大纲进一步贴近居中正文列，保留 24px 间距，并通过对称预留保持正文与输入框对齐；短线右对齐，按消息摘要长度计算宽度，上限 24px（当前项 26px）。大纲以聊天视口垂直居中，条目溢出时由内部列表滚动。此次调整后 fmt、diff 检查与生产 build 通过，12 轮聊天夹具启动保持响应且 stderr 为空；实际视觉仍待人工验收。
