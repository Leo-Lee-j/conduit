# OpenCode ACP 接入

2026-10-28，基于本机 OpenCode 1.19.2。实现以该版本官方 ACP 源码为基线，不宣称兼容所有旧版或未来版本。

## 使用

先在 OpenCode CLI 完成安装、登录和 Provider 配置。在 Conduit 输入区引擎菜单选择 OpenCode；路径支持环境变量 `CONDUIT_OPENCODE_PATH`、设置中的自定义路径、PATH 上的 `.exe` / `.cmd`、npm 包的原生 executable，以及 `~/.opencode/bin/opencode.exe`。

第一次发送可使用默认模型；新建/恢复会话后，ACP 返回该工作区的有效模型与思考档位。目录按会话保存，避免不同工作区配置串用。模型使用完整 `provider/model` ID；思考档位保存 OpenCode 原始值，例如 `minimal`、`max`，不会把 Codex 的 `xhigh` 静默映射为 `max`。

切换模型时清除旧档位；新模型的档位目录在下一次连接时返回。首次打开模型弹窗或切换到 OpenCode 时，在当前工作区后台执行 `opencode models --verbose --pure`，直接展示 CLI 返回的原生模型目录并按原生 Provider 分组，无需先发送消息。项目配置、JSONC、环境覆盖和登录 Provider 由 OpenCode 自行解析；Conduit 不合并 CCS 默认模型、CCS Provider 或自定义路由。目录按工作区缓存，刷新重新查询；修改 CLI 路径会清空缓存。查询最多等待 20 秒、输出上限 16 MiB，取消或关闭应用会清理查询进程树。读取失败会提示重试，不回退到 CCS。不支持的模型/档位会在发送 prompt 前报错。

每轮创建独立受管进程，通过 ACP 新建或恢复原生会话；重启 Conduit 后仍使用保存的原生 ID。优先 `session/resume`，不支持时回退到 `session/load` 并忽略旧历史回放，避免重复消息。无法恢复时显式报错，不自动创建丢失上下文的新会话。

## 权限与生命周期

运行 `opencode acp --pure --hostname 127.0.0.1 --port 0 --mdns=false --cwd <workspace>`。内部服务使用每个进程随机生成的密码，登录凭证继续由 OpenCode 管理；自动升级在受管进程内关闭。

Conduit 注入名称随机的专用 primary agent，并在发送 prompt 前选择它。它的权限规则追加在用户全局规则之后，避免用户已有的具体 allow 规则绕过应用选择。原生 build/plan 模式不用于代表 Conduit 的权限级别。

- **仅可查看**：默认禁用工具，仅开放 read/glob/grep/list。命令、写入、MCP、未知工具和权限扩展均不自动开放。
- **工作区内修改**：读取工具直接开放；写入、命令、目录外访问及其他工具按请求审批。命令经用户批准后仍可访问目录外文件，这不是文件系统沙箱。
- **完全权限**：工具权限直接允许；问答、模式切换和子代理仍不在首版范围内。

审批按 ACP option 的 `kind` 选择实际 `optionId`，只授予本次允许，不自动选择永久允许。审批 ticket、原生工具 ID 和 RPC 请求 ID 分开处理。未知客户端反向 RPC 返回明确错误。

取消先发送 `session/cancel`，最多等待 750 ms，再进入现有 Windows Job 进程树清理屏障。正常结束也会清理进程树；只有确认进程退出后才发布 Finished。每个会话仍只允许一轮，同引擎同原生 ID 不重复执行。

自动命名与 Git 说明生成使用 `opencode run --pure --format json`，通过 stdin 传入内容并关闭输入管道；使用独立临时数据库、忽略项目配置、禁用全部工具及 OpenCode 自带标题生成，进程清理后删除临时历史目录。JSON `error` 事件直接结束任务并显示模型错误，避免 ACP 1.19 在模型请求失败后等待 idle，最终只显示 60 秒超时。额度不足需更换可用模型或处理额度，应用不会自动切换模型。

模型目录按工作区和 CLI 路径持久化到数据目录的 `opencode-models/`，有效期为 24 小时；打开模型列表和设置页复用缓存，手动刷新强制查询。缓存只保存解析后的模型名称、ID、上下文大小等目录信息，不保存 Provider 凭据或 CLI 原始输出。新会话发送前选择的权限会传给实际创建的会话；切换引擎准备创建新会话时，权限选择作用于新会话。

## 首版边界

已实现流式文字、工具输入/输出/状态/diff、计划事件、用量结束快照、允许/拒绝、执行和待审批取消、原生续接、模型与动态思考档位、引擎/设置入口及持久化。

尚未接入 OpenCode 终端历史自动导入、Conduit 自定义 Provider/CCS 桥接、外部插件、问答工具、图片和子代理。OpenCode 原生 Provider 配置可直接使用；Conduit 内创建的会话可保存及恢复。`agent_thought_chunk` 不作为公开推理摘要显示。

新增状态字段兼容旧状态文件；包含 OpenCode 枚举值的新状态文件不保证能被旧版 Conduit 读取。

## 验证

协议单测覆盖新建/恢复顺序、配置重设、回放抑制、无效档位拒绝、审批 option 映射、错误会话隔离、未知反向 RPC、工具增量合并和辅助任务权限/目录清理。

以下测试使用真实安装的 OpenCode 加本地模拟 OpenAI-compatible HTTP/SSE 端点。Provider、数据库、配置与工作区全部隔离，不访问模型账户，不需要 API Key：

```powershell
cargo test --locked opencode -- --test-threads=1
cargo test --locked installed_opencode_local_protocol_smoke -- --ignored --nocapture --test-threads=1
cargo test --locked --features ui-tests opencode_model_picker -- --test-threads=1
```

本地端点测试覆盖两轮原生续聊、允许后写入、拒绝后不写入、待审批取消、只读工具裁剪、完全权限，以及两个并发会话的取消隔离和辅助任务无工具调用。它验证真实 CLI 协议与本地工具路径，不代替真实服务商登录、计费模型兼容性和长时间人工 UI 验收。

仓库常规检查：`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked -- --test-threads=1`、`cargo build --locked`，以及 `scripts/smoke.ps1` 的 chat/settings 启动冒烟。

2026-10-28 验证结果：

- 全量常规测试：149 通过、13 项显式集成测试跳过；随后新增的 diff 用例及最终取消边界修改，分别由目标测试与运行时回归覆盖。
- OpenCode 协议与 UI 目标测试：7 通过、1 项真实 CLI 测试按默认规则跳过；随后显式运行该真实 CLI + 本地端点测试，通过（最终一轮约 27 秒）。
- UI 测试集：20 通过，包括 OpenCode 动态档位、原有模型面板、并发会话和审批隔离。
- 最终运行时回归：8 通过、10 项需要真实账户或额外安装条件的测试跳过。
- 新状态字段默认值、模型设置往返和 OpenCode 版本解析目标回归通过。
- `cargo clippy --locked --all-targets --features ui-tests -- -D warnings` 与 `cargo build --locked` 通过。
- chat/settings 两种窗口启动冒烟通过。证据目录分别为 `target/smoke-chat-daae4bd048a64c29bc6863251608ce06` 和 `target/smoke-settings-534673a65446499082cdf2155b7126d2`。

以上测试没有调用真实模型账户，也不替代真实服务商和长时间人工验收。
