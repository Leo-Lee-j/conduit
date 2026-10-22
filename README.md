# Conduit

Windows 本地 AI 编程 Chat 工作台，使用 **Rust + GPUI + GPUI Kit**，底层运行 Claude Code CLI / Codex CLI。

当前是 **Chat M0 原型**：工作区、会话历史、流式回复、工具卡片、审批映射、取消、原生对话续接和会话级模型选择已接通。两款 CLI 的真实对话与续接测试已通过；审批协议层的允许、拒绝和待审批取消回归已通过，已完成一轮 UI 自动预验并修复设置弹窗窄窗裁切；界面输入、审批弹窗交互与长时间使用仍需验收。不是完整的 Codex 桌面端复刻。

## 运行

```powershell
cargo run --locked
# 直接打开设置
cargo run --locked -- --settings
```

也可运行 `target/debug/conduit.exe`。需要 Windows、Rust MSVC 工具链及 Visual Studio C++ Build Tools；本次验证使用 Rust / Cargo 1.99.1。依赖由 Cargo.lock 与 GPUI Kit 精确 revision 固定。

1. 先在官方 CLI 中完成安装和登录。
2. 点击“添加目录”，选择你信任的本地工作区。
3. 点击“+ Claude”或“+ Codex”，创建绑定该引擎的 Chat 会话。
4. 输入框上方可为本会话选择模型；默认继承 CLI 当前模型，选择在下一条消息生效。
5. 输入任务后点击发送，或按 **Ctrl+Enter**；Enter 换行。
6. 回复以 Markdown 显示；工具卡片可打开右侧执行详情。CLI 请求审批时，可允许本次或拒绝。
7. 同一会话再次发送会恢复其原生 CLI 上下文；不同工作区可并行，同一工作区仅一个活动任务。
8. 取消任务会停止该进程树。关闭窗口会清理所有受管任务，等待进程退出后关闭。

没有切换引擎、后台保活、自动重试和恢复失败时自动新建上下文。

## 设置、CLI 路径与模型同步

左下角齿轮打开设置弹窗，左侧切换通用与各引擎页面。可为两款引擎选择本机 `.exe`、清除应用配置并检测版本。配置保存后用于后续启动。路径优先级为：**环境变量 → 应用配置 → PATH → 常见原生安装位置**；无效显式路径会报错，不会自动改用另一份程序。当前要求 Windows 原生 `.exe`；npm `.cmd`、Shell 包装脚本与 WSL 尚未适配。

设置左侧新增 **模型同步** 页面，只读展示两组来源：

- **当前 CLI 模型**：Claude 读取 `%USERPROFILE%\.claude\settings.json` 的 `ANTHROPIC_MODEL`、各档位默认模型与子代理模型；Codex 读取 `%USERPROFILE%\.codex\config.toml` 的 `model`、`model_provider`、`model_reasoning_effort`、`model_catalog_json`。
- **CCS 同步模型**：以只读方式查询 `%USERPROFILE%\.cc-switch\cc-switch.db` 的 `providers`，仅提取 Claude 模型字段与 Codex `modelCatalog` 中的模型名、显示名、上下文大小和可见性；按 Provider 分组并标记当前 Provider。

设置页不查询或展示 API Key、Token、认证配置或其他 `settings_config` 字段；不写回 CLI / cc-switch 配置，也不切换 cc-switch Provider。模型目录在启动或手动刷新时读取，不写入应用状态。

对话输入区会合并当前 CLI 模型与 cc-switch **当前 Provider** 的模型；其他 Provider 的模型仅在设置页展示，避免跨 Provider 误用凭证。会话模型选择只影响本会话后续启动，并随会话保存；旧会话未选择时继续使用 CLI 默认。模型是否可调用仍由 CLI、Provider 与账户决定。

```powershell
$env:CONDUIT_CLAUDE_PATH = 'C:\path\to\claude.exe'
$env:CONDUIT_CODEX_PATH = 'C:\path\to\codex.exe'
cargo run --locked
```

版本检测支持取消，只执行 `--version`，最多等待 10 秒，输出有大小限制；取消或关窗会清理检测进程树。检测不调用模型，也不验证登录或全部协议兼容性。未知版本和预发行版本会标记为尚未回归。

真实审批回归（2026-10-22）：Codex 命令拒绝、文件修改允许/拒绝、待审批取消均通过；Claude 在测试专用 `manual` 权限模式下通过 Write 工具的 `can_use_tool` 允许/拒绝，拒绝不落盘，允许按批准内容落盘。Claude 回归使用 `CONDUIT_CLAUDE_TEST_MODEL` 指定本机可用模型，因为默认模型别名当前不可用。审批弹窗的点击与视觉仍未人工验收。

本机实测：Claude Code **2.1.301**、Codex **0.163.0**。这表示验证过的版本，不代表兼容所有旧版本或未来版本。Conduit 不代管登录凭证，不自动安装或升级 CLI。登录失效请在官方 CLI 中处理。

Codex 使用 `app-server --stdio`；Claude 使用 stream-json 与 stdio 控制协议。工具配置、模型执行及凭证仍由 CLI 负责，不直接调用模型 API。Claude 的控制协议适配参考官方 Agent SDK 源码，需要随 CLI 版本回归。

## 本地数据

默认位置：`%LOCALAPPDATA%\Conduit\state.json`。通过 `CONDUIT_DATA_DIR` 可指定独立目录。

- 保存工作区、固定 Provider、消息、工具详情、原生会话 ID、可选模型选择、中断标记和可选 CLI 路径配置；版本为 2，兼容没有配置或模型字段的旧 v2 文件。
- 模型目录与当前默认值在启动或手动刷新时从 CLI 配置及 cc-switch 模型目录读取，不写入应用状态；应用状态只保存会话选中的模型 ID。
- **对话与工具输出以明文保存**，可能包含代码、命令输出或其他敏感内容。使用系统账户文件权限保护。
- 不额外保存登录凭证，不记录完整原始协议流；CLI 自身的存储与日志由其配置决定。
- 同目录临时文件刷盘后原子替换；坏文件和未知版本不覆盖，界面进入只读状态。
- v1 终端原型仅迁移会话元数据，不伪造旧对话上下文。
- 重启加载历史，不重连进程，不重放任务；未完成运行标记为中断。
- 草稿按会话隔离，当前只保留在内存。
- 同一数据目录只允许一个应用实例。

Conduit 的进程监管不是权限沙箱。Codex 请求 `workspace-write` + `on-request`；Claude 使用默认权限模式并保留 CLI 配置。界面只审批 **CLI 实际发出的请求**，不保证所有命令或文件操作都逐次弹窗。未适配的交互请求返回错误，不自动放行。

## 验证

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
cargo build --locked
# 分别检查 Chat 页、设置页能否启动响应窗口
./scripts/smoke.ps1 -View chat
./scripts/smoke.ps1 -View settings
# 人工验收前的自动预验：隔离数据 + 窗口截图 + UIA 快照（不替代人工签署）
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action seed
pwsh -NoProfile -File scripts/ui-acceptance.ps1 -Action capture -Name chat-markdown -DataDir target/ui-acceptance/data-claude
```

`ui-acceptance.ps1` 以 UTF-8 BOM 保存。Windows PowerShell 5.1 直接执行时需带执行策略参数（`powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\ui-acceptance.ps1 -Action seed`）；PowerShell 7（`pwsh`）无需额外参数。自行编辑脚本时请保留 BOM，否则 5.1 会按系统 ANSI 读取中文而解析失败。

启动冒烟只确认窗口出现并保持响应，不覆盖输入、点击、输入法或 DPI。逐项人工验收步骤见[验证记录的手工 UI 验收清单](docs/m0-validation.md#手工-ui-验收清单)。

可选集成测试：版本检测与握手不调用模型；其余测试会发送少量合成提示或工具调用，消耗所选 CLI 账户额度。不会自动运行或授予工具权限。Claude 审批回归使用测试专用 `manual` 权限模式，可用 `CONDUIT_CLAUDE_TEST_MODEL` 指定本机可用模型。

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
```

## 当前边界

- 界面：使用自绘标题栏（品牌区与窗口控制），不再显示系统原生标题栏；左侧工作区栏与右侧执行详情可拖动调宽（各有上限），窄窗时详情覆盖显示；审批卡片与对话文字列同宽，请求详情默认折叠、可手动展开；输入区右上角“最新消息”，底部保留横向工具栏；设置弹窗居中并保留滑入动画，模型目录与 cc-switch 读取在后台完成，打开时不阻塞首帧。
- 构建：Debug 构建已对依赖启用优化以改善窗口拖动与动画的流畅度；追求最流畅手感可用 `cargo build --release` 后运行 `target/release/conduit.exe`。
- Windows 后端已实现；macOS / Linux 尚未实现。
- 原生会话恢复依赖 CLI 本地记录；历史消息不能替代原生上下文。恢复报错须显式处理。
- 取消采用结束 Job 的方式，模型服务端请求停止与 CLI 历史完整落盘无法保证；本地消息保留最近一次落盘内容。
- Codex 命令与文件修改审批、Claude `can_use_tool` 允许/拒绝、Codex 待审批取消、Claude 待审批取消已有真实回归；Codex `item/permissions/requestApproval` 权限扩展映射目前只有单测，真实触发待回归。审批弹窗点击与视觉、真实 CLI 多审批并发排队和中文输入仍待人工验收；多审批排队目前由运行时协议夹具覆盖，不等同真实 CLI 并发。
- 2026-10-22 已修复：Ctrl+Enter 被输入框换行绑定抢占；切换会话、发送、审批和取消后的输入焦点丢失；后台会话事件拉动当前会话滚动；审批详情过小及只可拒绝时缺少说明；只读存储仍允许输入。新增双运行时取消隔离回归。以上通过编译、单测和启动检查，但实际按键、输入法、复制与弹窗点击仍须按验证记录手工验收。
- 中文输入法、最小窗口、多屏 DPI、长对话性能、跨工作区并行压力待人工验收。
- 模型同步只读取上述 CLI 配置与 cc-switch `providers` 中的模型字段；不查询凭证、不改写配置、不切换 cc-switch Provider。其他 CCS Provider 的模型只展示，不自动搬运其端点或账号。模型可用性和计费仍由 CLI 与账户决定。
- 不支持任意 CLI 交互、问答型工具请求、图片附件、终端 TUI、Git Diff 面板或自动代理协作。
- 协议处理循环的 panic 会进入同一进程清理屏障后报告失败；启动或清理阶段的未预期 panic 仍会保守保留占用，需重启应用检查。

## 文档与代码

- [产品设计](docs/product-design.md)
- [技术设计](docs/technical-design.md)
- [验证记录及待验收项](docs/m0-validation.md)
- [UI 自动预验记录](docs/m0-ui-acceptance-run.md)
- [人工验收交接单](docs/m0-manual-acceptance-guide.md)
- `build.rs`：Windows MSVC 可执行文件的主线程栈预留。
- `scripts/smoke.ps1`：使用隔离数据检查两种启动页面，不调用模型。
- `scripts/ui-acceptance.ps1`：隔离数据下启动界面、截图与 UIA 证据采集，供人工验收前预验。
- `src/app.rs`：Chat 界面、草稿、工作区互斥、事件合并与退出屏障。
- `src/protocol.rs`：Codex / Claude 各自协议及审批映射。
- `src/chat_runtime.rs`：每轮任务、管道读写、取消与清理。
- `src/child.rs`：Windows stdio、句柄白名单与 Job 进程树。
- `src/cli.rs`：本地路径配置、来源检测与受管版本检测。
- `src/models.rs`：CLI 模型配置的只读解析与模型目录合并。
- `src/ccs.rs`：cc-switch 数据库模型字段的只读查询。
- `src/domain.rs` / `src/store.rs`：数据实体、迁移与原子存储。
- `src/legacy/`：未编译的旧终端原型，仅保留参考。
