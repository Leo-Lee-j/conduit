# Conduit 技术设计文档

版本 0.3 · 2026-10-19 · Rust + GPUI + GPUI Kit · Chat M0

依据：[产品设计](product-design.md)。本版整体取代终端主界面方案；事实与未完成项见 [验证记录](m0-validation.md)。

## 1. 技术结论与取舍

使用 GPUI 原生 Chat UI，通过官方 CLI 的结构化 stdio 协议驱动任务。**不解析终端屏幕，不直接调用模型 API。** Windows 使用匿名管道与 Job Objects 管理进程。

Rust 适合本地进程、数据与界面共用一套语言；GPUI Kit 提供输入、按钮、布局与 Markdown。优势是原生渲染及单语言维护，代价是组件生态和跨平台验证成本需要自己承担。现阶段无需为了换技术栈重做已验证的协议与进程层。

Electron 的终端和 Web UI 生态更成熟但增加运行时与双语言集成；Tauri 适合 Rust + Web 前端但需要 WebView；Flutter 需要另一套语言和本地桥接。当前需求没有充分理由迁移。若 GPUI 的 IME、可访问性或 Markdown 性能持续达不到日用标准，再以同一协议核心评估替换界面。

Windows MSVC 构建通过 `build.rs` 为 Conduit 主线程指定 8MiB 栈预留。新增设置页后的 Debug 构建曾在原 1MiB 预留下启动栈溢出，扩大预留后 Chat 与设置页都通过实际启动检查；该设置只对本二进制生效，不改变所有依赖的编译参数。

GPUI Kit 锁定 revision `2963d6d880b9dffe59b48b0533bdc83c03201209`（0.7.3 / gpui-pre 0.3.9）。使用 Cargo.lock 固定依赖，不承诺兼容任意 GPUI Git HEAD。GPUI Shell 不列为额外必选依赖。

## 2. 模块边界

| 文件 | 职责 |
| --- | --- |
| main.rs | 应用启动、主题、窗口、按数据目录限制单实例 |
| app.rs | Chat 视图、会话选择、草稿、工作区占用、事件合并、持久化触发 |
| protocol.rs | Provider 特定的请求与事件、原生 ID 校验、审批响应 |
| chat_runtime.rs | 一轮任务的后台线程、NDJSON、输入队列、取消和清理屏障 |
| child.rs | Windows 句柄、CreateProcessW、管道与 Job 生命周期 |
| cli.rs | CliSettings、路径优先级与校验、版本解析、受管后台诊断 |
| models.rs | Claude / Codex 本机模型配置的只读解析与模型目录合并 |
| ccs.rs | cc-switch SQLite 数据库模型字段的只读查询与 Provider 分组 |
| git.rs | Git 快照、分支/待推送计数与显式 commit/upstream push；状态读取只读 |
| domain.rs | Provider、Workspace、SessionRecord、ChatItem |
| store.rs | v1→v2 迁移、校验、原子保存与读取保护 |

CLI 设置作为 v2 存储的可选 `cli` 字段，字段缺失时使用默认配置，兼容已有 v2 历史。修改配置保存失败会恢复原内存配置，避免显示已生效但未落盘。环境变量优先于应用配置，其后为 PATH 与常见原生安装位置。显式路径失效时不回退；仅接受存在的绝对 `.exe` 路径。

设置页的 `--version` 在独立 Job 中运行，10 秒超时，每个输出流最多保留 64KiB；结果区分已验证版本、未回归版本和不匹配的程序标识。预发行版本不按相同数字前缀认定已验证。路径变更时取消旧检测，并校对结果对应的有效路径，避免把旧程序的结果显示为新配置。关窗同时取消并等待诊断任务。

模型同步同样是只读外围能力：Claude 解析 `~/.claude/settings.json` 的模型字段及 `env` 中的模型变量，Codex 解析 `~/.codex/config.toml`、相对其目录解析 `model_catalog_json`，并按 `visibility` 过滤隐藏模型。另以 `SQLITE_OPEN_READ_ONLY` 打开 `~/.cc-switch/cc-switch.db`，使用 `json_extract` / `json_each` 只选择 `providers` 中的模型 ID、显示名、上下文大小与可见性；查询结果不包含 `auth`、Token、端点认证或完整 `settings_config`，也不写回 CLI / cc-switch 配置。读取结果仅存在于内存，刷新或启动时重新解析；单个 Provider 配置缺失或损坏时只影响该来源列表，不阻断会话。

Git 面板不引入第二套 Git 实现：`git.rs` 的状态探测调用 `rev-parse --is-inside-work-tree`、`symbolic-ref --quiet --short HEAD`（未提交分支也可识别；分离 HEAD 回退为 `rev-parse --short HEAD`）、`for-each-ref`、`status --porcelain=v1 -z` 与 `diff --cached`，并对 status/diff 加 `--no-optional-locks`，避免读取时占用索引锁。命令在独立线程中运行，复用 `Child` 的 Job/管道，15 秒超时并结束进程树；暂存 diff 最多保留 200 KiB、最多列出 2000 个文件，超出部分截断并显示说明。分支元数据使用独立 `BranchProbe`，启动/切换工作区时加载、每 10 秒后台更新、任务完成或手动刷新时重读；只列本地 refs/heads 与 refs/remotes，排除 origin/HEAD 等符号引用，不 fetch，不切换分支。变更快照在 Git 侧栏打开或点“刷新”时启动一次探测，CLI 一轮任务结束且 Git 侧栏可见时自动重读；结果只保留在内存，切换工作区会丢弃旧快照并重新读取。不是 Git 仓库、未安装 Git 或命令失败时显示可重试的错误；状态探测不向仓库写入任何内容。

Git 侧栏下方是固定提交区，草稿按工作区路径保留为独立 TextareaState。`OperationProbe` 后台执行显式 `git commit -m`（仅 index，不自动 add）和向配置 upstream 的 `git push --porcelain -- <remote> HEAD:<merge-ref>`，各限时 120 秒。提交前核对预览的分支/HEAD/暂存对象指纹；成功提交、失败推送用独立结果状态表达，清空已提交草稿并刷新 ahead badge，不自动重试。Git 写操作与聊天任务互斥。分支探测新增 upstream 和 ahead（有 upstream 时 rev-list upstream..HEAD；无 upstream 时 HEAD --not --remotes）。AI 生成复用 `ChatRuntime::start_text` 的独立临时会话，使用当前引擎/模型、只输入暂存 diff；结果按来源路径回填草稿，取消/超时/错误不覆盖。详见 [Git 界面设计](git-ui-design.md)。

仍是单二进制项目，不提前拆多个 crate、插件宿主、守护服务或 RPC。旧终端代码放在 `src/legacy/`，没有 mod 声明且不参与构建；活跃依赖不再包含 alacritty_terminal。

架构图源文件为 [chat-architecture.mmd](diagrams/chat-architecture.mmd)，可在支持 Mermaid 的工具中查看。

## 3. 数据模型

| 实体 | 字段 / 规则 |
| --- | --- |
| Provider | Claude / Codex 枚举、显示名称、exe 检测 |
| Workspace | 规范化绝对 path、sessions；暂不另建 Project |
| SessionRecord | id、provider、created_at、title、archived、native_id、model、messages、interrupted；固定工作区与 Provider |
| ChatItem | key、User / Assistant / Tool / Notice、text、detail、status |
| ActiveRun | runtime、审批集合、当前轮 prefix、状态；不落盘 |
| Child | 当前 OS 根进程句柄与 Job 句柄；不作为恢复依据 |

`exit_code` 为兼容 v1 保留的旧字段，不用于表达 Chat 成功。Chat 的正常完成以 Provider 的最终事件为准，之后还必须确认进程树退出。

本地 ID 是单实例约束下的递增 u64；加载拒绝 0、MAX 与重复 ID。事件 key 为 `turn-prefix + native-item-key`，把同一轮的 delta 与最终快照合并，并防止不同轮次误合并。

草稿以 Session ID 为内存键，切换时保存和恢复。当前不持久化草稿。工具详情拆为输入、流式输出和最终结果三部分保存；右侧面板按工具 key 动态读取，避免完成快照覆盖原始输入。

`SessionRecord.model` 为可选覆盖：`None` 表示沿用 CLI 默认，新会话创建时复制 Provider 当前默认模型的 ID。切换后保存到会话，只影响该会话下一轮启动；读取模型目录本身不落盘。旧 v2 文件缺少该字段时按 `None` 加载。

## 4. 会话与运行生命周期

创建 Session 不启动 CLI。发送前校验工作区、引擎路径、消息大小与互斥，并读取该会话的模型覆盖；写入用户消息并设 interrupted=true，保存成功后才开始运行。

每轮启动一个进程，第一轮建立原生会话，后续精确 resume。该方案使进程的存活范围等于任务范围，会话身份跨进程延续。局限是每轮都有初始化成本，CLI 的进程内暂态不保证保留。

通用展示状态：空闲、连接中、执行中、等待审批、停止中、完成或失败。状态是 UI 摘要，不等于统一的供应商内部状态机。

收到原生 ID 立即保存；收到部分内容按节流保存；最终事件且进程树确认清理后，清除 interrupted，解除占用。恢复失败、ID 不匹配或协议错误都不自动新建，也不重试。

关闭窗口时：标记 closing → 禁止新任务 → 取消全部运行 → 等待每项 Finished → 保存 → 关闭。没有后台守护。应用异常退出时由 kill-on-close Job 约束本地后代；机器重启后只读历史，不以旧 PID 重连。

## 5. Codex 适配

已实测 Codex 0.163.0。启动 `codex.exe app-server --stdio`，以 JSON-RPC 结构逐行读写：

1. `initialize` → 校验结果 → `initialized`。
2. 无 ID 用 `thread/start`；有 ID 用 `thread/resume`，指定精确 threadId。
3. 返回的 thread ID 必须匹配已保存 ID，才发送 `turn/start`。
4. 输入类型 text；接收 agentMessage delta 与 completed 快照，统一同一个 item key。
5. commandExecution、fileChange、mcpToolCall 映射工具卡片；reasoning 不渲染。
6. `turn/completed` 决定任务协议结果。忽略其他 thread 及已知非当前 turn 的通知。

线程参数请求 `approvalPolicy=on-request`、`approvalsReviewer=user`、`sandbox=workspace-write`。有会话模型覆盖时，`thread/start`、`thread/resume` 与 `turn/start` 都携带 `model`。不使用 bypass，也不把 cwd 当沙箱边界。CLI / 平台如何落实权限仍取决于本机版本和配置；初始化失败不自动降低权限要求。

支持 `item/commandExecution/requestApproval`、`item/fileChange/requestApproval` 与 `item/permissions/requestApproval`：保留 JSON 原始请求 ID，命令与文件修改响应 accept / decline，仅限本次；权限扩展请求（0.163.0 协议中除前两者之外的显式提权）允许时回传请求的 profile 并限定 `scope=turn`，拒绝时回传空 profile——该响应没有 deny 枚举，空 profile 是协议内的安全拒绝，伪造未知 scope 可能导致 CLI 侧反序列化失败。适配器给 UI 分配本轮唯一 ticket，避免 wire ID 重用导致迟到点击误授权；Codex `serverRequest/resolved` 和 Claude `control_cancel_request` 都会撤销 ticket。`availableDecisions` 不包含 accept 时禁用允许按钮。Codex 命令拒绝、文件修改允许/拒绝、待审批取消与 Claude `can_use_tool` 允许/拒绝/待审批取消已在真实 CLI 通过；权限扩展请求与真实并发多审批仍只有夹具覆盖。未知 server request 返回 -32601，并在 UI 留 Notice。暂不实现所有表单输入、动态工具、MCP 认证或实验性功能。

## 6. Claude Code 适配

已实测 Claude Code 2.1.301。使用原生 exe，关键参数：

```text
--print --verbose
--output-format stream-json --input-format stream-json
--include-partial-messages
--permission-prompt-tool stdio
--permission-mode default
[--resume=<exact-session-id>]
[--model <session-model>]
```

先发送 initialize control_request，成功后发送 user envelope。`system.session_id` 等合法 session_id 绑定记录；恢复时若返回不同 ID 立即失败。

stream_event 的 message ID + block index 作为文本键；assistant 完整快照替换同键增量，防止重复。tool_use / tool_result 合并到工具卡片。不展示私有推理。

`can_use_tool` 映射审批；允许仅回传原输入 updatedInput 和 allow，不写 updatedPermissions；拒绝回 deny。未知控制类型回错误，取消请求移除其待审批项。允许、拒绝与待审批取消均已在真实 CLI（测试专用 `manual` 模式）通过。

适配依据是官方 Agent SDK 的 subprocess transport 与 control query 实现，而非 CLI TUI 文本。当前 Rust 实现没有 Python / Node SDK 运行时依赖，但承担 wire protocol 的版本维护风险。不能把 SDK 内部控制格式当成永久稳定的跨版本公共协议。

## 7. 统一事件与 UI

协议层输出 NativeId、Delta、Text、ToolInput、Tool、Approval、ApprovalCancelled、ApprovalResolved、Notice、Finished；原始 JSON 仍属于 Provider 适配器，UI 不通过正则猜测语义。

UI 使用 GPUI Kit Textarea；Enter 发送、Ctrl/Shift+Enter 换行（在 `ConduitComposer > Input` 与 `ConduitComposer` 上绑定 `SendPrompt`/`Newline`，覆盖输入框默认 Enter 行为），原生输入组件承担文本编辑。助手消息使用可选择的 Markdown，用户消息独立显示。窗口使用自绘标题栏（`TitleBar`，含品牌区、拖动、双击最大化与窗口控制命中区），不再显示系统原生标题栏。左侧工作区栏仍是可拖拽面板组（200–420px）；右侧侧边栏由应用自己记录宽度，拖动左边缘时按指针位置更新，宽度上限为剩余内容区的一半，窄窗（<1100px）时改为覆盖层，工具详情与 Git 变更有独立入口，按入口切换停靠内容，无共用页签或 split。聊天左上角的工作区选择器可切换已有工作区或添加目录；输入区在会话、无会话和无工作区三种状态下使用同一个 composer 结构（宽度、高度与内边距一致），无会话时发送会用 `UiSettings.last_provider`（上次使用的引擎）自动创建会话。侧栏顶部是一行图标操作：新建会话（使用当前“新建会话引擎”）与搜索。新建会话引擎由输入区左下角的下拉决定（向上弹出），切换引擎会切换到该工作区该引擎最近的未归档会话；没有则会清空选中，右上角模型选择器随选中会话同步，选择器右侧不再重复显示引擎标识。工作区默认只显示最近 N 条会话（`UiSettings.sessions_per_workspace`，默认 5，设置页可调 1–30，超出显示“显示更多”并可展开）；工作区行提供“在此工作区新建会话”图标与向右弹出的“移除工作区”菜单（只移除应用内路径，保留磁盘文件与 CLI 历史）；会话行菜单同样使用 `Anchor::LeftCenter` 向右弹出并与工作区菜单对齐，提供重命名、归档/取消归档与删除；可点击的图标与菜单项使用 `cursor_pointer`（禁用态不加），工作区标题右侧可切换是否显示已归档会话。搜索是独立弹窗：输入框绑定 `InputState`，结果实时匹配标题与消息正文，命中词用 `StyledText::with_highlights` 在标题与摘要中高亮；摘要取首个命中消息的上下文，点击结果切换到对应会话。重命名与删除分别使用输入/确认对话框。审批卡片与对话文字列同宽（最大 840px 居中），显示图标、标题与队列序号、说明、以及默认折叠、可手动展开的请求详情，底部为“拒绝 / 允许本次”。对话区右下角（输入区右上角上方）在消息列表未停留底部时显示 向下箭头按钮：`ConduitApp::at_bottom` 由既有的 33ms 轮询按“距底部 < 60px”维护，跨过阈值时通知重绘，点击 `scroll_to_end` 并立即隐藏；底部保留横向工具栏用于后续扩展。设置左侧导航分为通用 / 外观 / Git / CLI / 引擎 / 模型同步：CLI 页展示路径查找顺序、可执行文件与版本检测说明，可执行文件与版本检测在各引擎页，通用页保留本机数据与诊断设置。设置左侧同时提供“模型同步”独立页，上半部分展示 CLI 当前模型，下半部分按 Provider 分组展示 cc-switch 模型目录并标记当前 Provider；外观页提供跟随系统 / 浅色 / 深色主题，并包含“工作区会话数”步进设置（默认 5，范围 1–30）。设置弹窗垂直居中并保留滑入入场动画，打开时先显示弹窗、再由后台线程读取 CLI 配置与 cc-switch 数据库并通知刷新，避免阻塞首帧。输入区上方提供会话模型选择器，选项合并 CLI 模型与 cc-switch 当前 Provider 的模型，另有“CLI 默认”；选择写入会话并作用于下一条消息。跨 Provider 模型不进入选择器，避免把模型名错误应用到其他端点或凭证。审批在输入区上方，详情可滚动，允许 / 拒绝按钮和确切请求绑定。工具调用卡片标题与左右内边距一致（不再为未嵌入的图标预留空白）；应用注册 `gpui_kit::assets::AllAssets` 完整图标目录，ArrowDown 等非默认组件图标才能渲染。点击工具调用卡片在其下方展开固定 240px 的独立滚动区域；展开区按输入与输出显示可选择的纯文本，保留换行与缩进、清理终端颜色控制码，并对完整 JSON 进行排版，支持复制。需要时从展开区图标另行打开右侧详情。对话内容区底栏位于输入区下方，不延伸至工作区栏与右侧停靠区：左侧分支下拉只读列出当前工作区本地/远程分支（非 Git 工作区隐藏），右侧 Git 图标打开专属 Git 侧栏；会话标题的侧栏图标只打开工具详情。Git 侧栏只读展示分支、暂存/未暂存/未跟踪数量与按新增/修改/删除分组的文件列表（支持折叠，不渲染 diff），点“刷新”或在 CLI 任务结束时重读；检测结果与选中工作区绑定，切换工作区会丢弃旧快照。窗口不足 1100px 时详情以覆盖侧栏呈现；审批一次展示队首项及总数，详情高度随窗口限制，取消会移除待审批入口。

外观设置保存在 `UiSettings.appearance`（`system` / `light` / `dark`，默认 `dark` 兼容旧数据）。`app.rs` 用 `Palette` 统一提供暗 / 浅两套语义颜色，切换时先更新调色板再调用 `Theme::change`；`system` 通过 `Window::appearance` 与 `observe_window_appearance` 跟随 Windows 明暗变化。主题选项与设置导航使用可访问 Button，只读存储时主题与会话数控件禁用并显示当前选择。

每约 33ms 读取每项运行最多 1,000 个事件；在用户距底部约 60px 内时跟随。完整消息替换增量，命令最终聚合输出替换部分输出。运行 key 保持每轮隔离。

当前是普通消息列表，尚无长对话虚拟化和完整可访问性验收。最小窗口为 850×500，侧栏 240px，详情 300px；窄窗已采用覆盖布局，实际渲染仍待人工验收。结束或重启时未完成的工具卡片会标记取消、中断或未返回结果，避免历史永久显示运行中。

## 8. Windows 进程与管道

使用 CreatePipe 建立三组 stdio，父端不可继承；通过 STARTUPINFOEX handle list 限定继承为三项子端句柄。Job 不可继承，设置 KILL_ON_JOB_CLOSE。

CreateProcessW 使用挂起 + 无控制台窗口 + 扩展启动信息；先绑定 Job 再 ResumeThread，避免先执行后监管的竞态。参数按 Windows argv 规则逐项转义，不拼进外层 Shell。cwd 独立传入，环境继承应用启动环境。

Job 的成员包含本地后代。取消通过原子标志通知 worker，最终终止整个 Job；Finished 只在 Job active=0 且根进程已 signaled 后发送。终止失败只提示并保持占用，不能假装成功。

写入在单独线程通过有界 128 项队列完成，避免 CLI 不读 stdin 卡住 UI 和取消。stdout/stderr 通过 PeekNamedPipe 轮询；每轮协议 stdout 累计上限 32MiB，单消息 8MiB；UI 单提示 256KiB。stderr 最多保留 8KiB 诊断，仍持续排空管道。

初始化超时当前按协议消息活跃度判断，约 60 秒；不设置一刀切模型执行超时。协议最终完成后关闭 stdin，最多等待 1.5 秒并排空尾部输出，之后停止残留 Job。此宽限期不是 CLI 历史落盘的绝对保证。

协议循环使用 unwind 边界，Child 在边界外保留所有权；循环 panic 会变成失败结果，并经过同一 Job / 根进程退出屏障后发送 Finished。启动或清理阶段未预期 panic 仍采取保守策略：RAII 清理句柄、发送 Notice、保留 UI 占用，不发送未经验证的 Finished；该罕见故障的独立恢复入口尚未实现。

macOS / Linux 后续需要新的进程组 / 信号后端与 CLI 路径检测。当前代码含 Windows 专用 imports，尚不是可跨平台编译的完成品；不要仅凭 GPUI 支持范围宣称产品已跨平台。

## 9. 存储、安全与恢复

默认 `%LOCALAPPDATA%\Conduit\state.json`，CONDUIT_DATA_DIR 可覆盖。v2 保存对话、工具详情、原生 ID、会话模型覆盖、中断标记，不额外接管凭证。模型目录来自 CLI 配置与 cc-switch 数据库且不写入状态，只有会话选中的模型 ID 落盘。cc-switch 数据库始终以只读连接访问，查询仅投影模型字段；应用不保存其 Provider 配置。对话与工具输出是本地明文，可能敏感；不宣称加密。

保存：同目录 `.json.tmp` → write → sync_all → MoveFileExW replace + write-through。单数据目录互斥锁避免两个应用覆盖。启动拒绝未知版本、重复目录或 Session ID、相对路径和坏 JSON；原文件不覆盖，禁止新任务。

v1→v2 保留身份及元数据，加迁移 Notice，不生成假 native_id。interrupted 的 v2 会话加重启 Notice，不自动重试；保存后重启不会反复增加同一恢复提示。

模型历史由 CLI 管理，Conduit 的消息是展示记录，不能直接重建工具执行上下文。暂停磁盘写入或掉电可能损失最后一段未落盘文本，不能承诺零损失。NativeId 与最终状态立即保存，其余内容约一秒节流。

安全边界：不执行模型输出中的 UI 操作指令，不自动授权未知请求，不隐式扩权。应用复用 CLI 的现有权限规则；允许规则匹配的操作可能不发审批，不能宣称逐命令拦截。MCP / hooks / 子代理仍由 CLI 配置决定，不承诺识别其全部外部副作用。

## 10. 验证与后续实施

常规：fmt、严格 Clippy、单测、locked build，并执行 `scripts/smoke.ps1 -View chat` / `-View settings` 检查实际响应窗口。启动参数 `--settings` 可直接进入诊断页面。冒烟脚本使用隔离数据、结束自己的无任务实例，不代替按钮、输入和正常关闭测试。单测覆盖恢复 ID 校验、命令与文件修改审批、权限扩展审批、决策约束、多 ticket 隔离与撤销、未知请求拒绝、回复去重、工具输入 / 输出分离、跨轮隔离、进程树清理、Unicode 参数、存储与迁移。真实 Codex 命令拒绝与 Claude 工具路径测试按 ignored 集成测试运行。

`scripts/ui-acceptance.ps1` 以隔离数据驱动界面并抓取窗口截图与 UIA 快照，用于人工验收前的自动预验（限制与覆盖范围见 [UI 自动预验记录](m0-ui-acceptance-run.md)）；它只产生证据，不替代人工签署。

性能：Debug 构建会因未优化的 GPUI 依赖在整窗重绘时明显变慢，表现为窗口拖动和动画逐帧卡顿。`Cargo.toml` 的 dev profile 因此对依赖启用 `opt-level = 2`、应用本体 `opt-level = 1`（保留调试信息），Release 构建仍是无优化差异的手感基准。

集成测试采用本机 CLI；握手不调用模型，真实对话测试用合成提示并明确标为 ignored。测试不自动授予工具权限，也不把模型生成回复当单元测试。

下一步按风险排序：GPUI 输入/布局/滚动、审批弹窗与模型选择器、Git 面板的人工验收（自动预验证据已备）→ 双工作区压力及异常退出 → 用户连续日用 → 模型启动参数与更多交互类型。任一错目录、错上下文或错授权先阻断新功能。

## 11. 核实依据

以下为本次适配的官方资料入口；实现同时对照本机 CLI 版本与生成的 Codex JSON Schema，并通过真实任务验证。主分支文档本身不等于未来版本兼容承诺。

- Codex app-server：`https://developers.openai.com/codex/app-server/`
- Claude SDK transport：`https://raw.githubusercontent.com/anthropics/claude-agent-sdk-python/main/src/claude_agent_sdk/_internal/transport/subprocess_cli.py`
- Claude SDK control query：`https://raw.githubusercontent.com/anthropics/claude-agent-sdk-python/main/src/claude_agent_sdk/_internal/query.py`
- GPUI Kit 锁定源码：`https://github.com/longbridge/gpui-kit/tree/2963d6d880b9dffe59b48b0533bdc83c03201209`

本机参考缓存位于 `target/protocol-references/` 和 `target/codex-schema/`，为构建辅助资料，不是运行依赖。

Git 生成提示词保存在 `Store.git.commit_message_prompt`，缺省或留空使用内置默认值。设置页通过独立多行输入更新，在既有后台轮询中最多每秒保存；退出时持久化。生成时统一附加已暂存文件与 diff。引擎标识使用 `Icon::data` 嵌入官方 SVG，不依赖磁盘路径或运行时网络。

用户消息大纲位于聊天内容区居中消息列的左侧，与消息列保持固定间距，只派生自当前会话的 User 项，保留消息 key 与完整消息列表索引。大纲在聊天区域垂直居中，短线向消息列一侧对齐，长度由摘要字符数计算并限制最大值；悬停展示摘要，滚动时高亮当前阅读位置。点击后重新按 key 校验、调用 `ScrollHandle::scroll_to_top_of_item`，短暂高亮并抑制这一跳转的自动跟随。工具展开状态与独立 ScrollHandle 以 `(session id, message key)` 隔离并在删除时清理；展开正文使用 `base::SelectableText`，自然高度的文本列放入固定 240px 的 block 滚动视口并附滚动条，避免 flex 子项收缩导致无溢出，也避免将终端输出解释为 Markdown/HTML。`ui-tests` feature 下的 GPUI 布局回归验证实际内容溢出、内部滚轮及外层消息滚动的隔离。

`ChatItem.approval` 独立保存审批 request 与状态，避免被执行状态更新覆盖。审批优先按当前轮次与提供方工具 ID 关联；无 ID 时按工具名与输入匹配。早到审批创建同 key 工具占位，后续执行事件继续更新。加载旧会话时按消息先后顺序将审批 Notice 合并进对应工具；待审批在取消或重启时收尾为已取消 / 已中断。
