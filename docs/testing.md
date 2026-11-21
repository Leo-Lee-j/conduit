# 测试与验收

测试按行为模块组织；下面是可运行入口，不是当前提交的测试结果。逐次日志、截图和性能报告写入忽略目录，不再新增按日期或轮次命名的进度文档。

## 环境隔离

使用 Windows x64 / Rust MSVC 和 Windows SDK，工具链版本以 [CI](../.github/workflows/ci.yml) 为准。常规测试使用隔离数据，不以日常 `state.json`、真实仓库或已安装 CLI 作为可修改夹具。

```powershell
$env:CONDUIT_DATA_DIR = Join-Path (Get-Location).Path 'target/manual-acceptance-data'
cargo run --locked
# 直接打开设置：cargo run --locked -- --settings
```

真实模型、账户、付费调用或额外软件依赖的测试需明确选择，不默认启用全部 `--ignored`。正在使用的程序被 Windows 锁定时，保留现有进程，采用隔离构建/预览程序记录实际验证对象。

## 核心检查

本地、普通 CI 和发布补充验证统一使用：

```powershell
./scripts/check-core.ps1
```

脚本依次检查翻译、格式、Clippy，运行核心测试，最后构建不带测试特性的主程序和更新助手。测试统一开启 `ui-tests`，一次执行普通测试与核心 UI 集成测试，避免先运行普通测试、再重复执行 `app::`。任一步失败都会停止；发布仍要求标签对应提交通过 CI。

只保留能发现功能或数据风险的自动测试：

| 核心行为 | 定向检查范围 |
| --- | --- |
| 发送前保存、失败回滚、草稿与附件、队列恢复 | `app::runtime::saves::tests`、`app::queue::tests`、`persistence::` |
| 子代理键盘展开、历史、任务复制与草稿保护 | `app::subagents::`、`app::ui_tests::subagent_panel_tracks_concurrent_tasks_and_disconnects_without_false_success` |
| 数据持久化、迁移、正文完整性、过期加载隔离 | `store::`、`transcript::`、`app::transcripts::tests` |
| 多引擎协议、权限审批、用户输入、会话恢复 | `protocol::`、`cli_adapter::`、`chat_runtime::` |
| 取消、异常退出、子进程与终端清理 | `child::`、`chat_runtime::`、`terminal::` |
| 跨会话隔离、审批失效、插件权限与路径边界 | `app::ui_tests`、`app::notifications::tests::approvals`、`plugins::`、`plugin_packages::` |
| 内容分栏的发送目标、预览归属、异步刷新及引用定位 | `app::files::panes::tests`、`app::github::tests`、`app::usage::tests`、`store::tests::ui_settings_default_to_five_sessions_and_roundtrip` |
| 文件变化、工具证据核对、Unicode 回放及新版本取消 | `content_review_`，覆盖不明确替换/截断 patch/竞争工具拒绝关联、可见快照语义、Markdown 恢复、减少动态效果与跟随暂停 |
| Git 写操作、更新验签、安装回滚、重启恢复 | `git::`、`update::`、`app::update::tests` |

纯布局尺寸、渲染次数、缓存命中次数、主题样式和品牌图标映射交由界面改动时人工验收。滚动保留异步正文、流式输出与待处理输入的行为回归，不逐一锁定控件几何尺寸。缓存测试检查过期结果隔离和数据正确性，不要求指针复用或固定解析次数。新增测试应说明它保护的核心行为；已有覆盖足够时扩展现有用例，不为每个控件或辅助函数增加一组测试。

示例文件若被演示脚本或测试作为输入读取，应随夹具保留。Shell 原型的资源隔离、测量验证器和显式性能探针有独立用途，按[插件验证](plugins/validation.md)运行，不并入主程序常规检查。

小改动可先运行相关范围，例如：

```powershell
cargo test --locked --features ui-tests --bin conduit chat_runtime:: -- --test-threads=1
cargo test --locked --features ui-tests --bin conduit app::runtime::saves::tests -- --test-threads=1
cargo test --locked --lib update:: -- --test-threads=1
```

选择筛选条件后核对实际执行数量，避免零测试通过。默认忽略的真实 CLI / 账户测试不进入 CI，按需明确选取单个用例。子进程测试以握手信号确认启动，再检查取消和资源释放，不把冷启动耗时算作清理耗时。

## Windows 人工验收

[smoke.ps1](../scripts/smoke.ps1)提供窗口启动冒烟，[ui-acceptance.ps1](../scripts/ui-acceptance.ps1)提供自动预验。运行前阅读脚本参数和环境要求。隐藏进程仍在运行、一次截图或 GPUI 模拟输入均不能替代完整人工验收。

| 场景 | 验收重点 |
| --- | --- |
| 输入与 IME | 微软拼音和另一常用输入法；候选确认不误发送，换行、`@`、双 Shift、Esc 与粘贴互不抢占 |
| 会话并发 | 多工作区/多引擎持续输出，切换保留草稿和阅读位置，停止只影响目标任务，审批不会串会话 |
| 工作区长列表 | 100 条会话展开后从中间精简；标题吸顶与跨组替换、标题上的滚轮、置顶保留、当前聊天与草稿不变；标题右侧浅底列表图标和底部“精简列表”均可点击，鼠标精简无边框，键盘圆角焦点不改变行尺寸与位置 |
| 队列与恢复 | 排队、撤回编辑、保存失败、取消、普通退出及更新后恢复正文与附件，不重复发送 |
| 阅读与文件 | 长回复、代码、Mermaid、文本选择复制、冷正文加载/重试、精确/模糊查找和独立滚动；Markdown 悬浮大纲的层级、折叠、重复标题跳转、窄视口和内容刷新 |
| 内容分栏 | 跨工作区参考文件不改变发送目标；全区/并排搬移、组内批量关闭、窄窗切组与比例恢复；双文件刷新、旧快照保留及删除重建恢复 |
| 文件变化与回放 | 普通刷新不滚动或切文件；修改块与删除边界；主动回放、暂停/继续/重播、返回当前 Markdown；中文/组合字符/emoji、查找和复制当前可见内容；滚动/换会话/隐藏暂停跟随，新版本结束旧回放 |
| Git | 部分暂存与未选中文件、按钮状态、分支鼠标菜单、冲突恢复、标签推送失败重试、统一通知 |
| 终端 | 实际 cwd、中文/空格路径、TUI/鼠标、Ctrl+C 与复制区别、多行粘贴、切换/收起/关闭及子进程清理 |
| 设置 | 表单未保存离开、只读、模型/来源切换、敏感信息、CLI 检查与安装状态、MCP 检测取消 |
| 通知 | 后台消息不抢焦点，导航来源正确，过期审批不可执行，长错误可阅读复制 |
| 更新 | 真实签名 A → B 升级、助手权限、运行任务屏障、磁盘不足、坏包、失败恢复、重启草稿 |
| 无障碍与窗口 | 键盘与 Narrator、可见焦点、弹窗 Esc、系统缩放、多屏、长路径和状态颜色之外的文字表达 |

至少覆盖明暗主题，850×500、1100×700、1200×800、1600×1000 逻辑视口，以及 100%/125%/150%/200% DPI。重点操作不能被长说明或错误挤出可见区；测量输入和滚动延迟，不用静态尺寸推断实际响应。

## 结果记录

每次验收记录提交或工作区差异、可执行文件/源码身份、工具链、数据目录、命令、退出码、通过/失败/忽略数及产物路径。中断、超时、环境缺失和未执行分别记录；重跑成功保留首次失败，不追认整轮一直通过。

人工结果需要实际操作步骤、窗口/DPI/主题、预期与观察。截图和日志保留在 `target/` 或原型 artifacts，不提交账户数据或凭据。性能使用固定负载和完整报告，见[性能与指标](performance.md)；Shell 的 G1–G6 准入另见[插件验证](plugins/validation.md)。

新增回归放进所属模块，不按审查批次另建测试文件。更新实现约束和验收清单即可，无需将每次命令输出追加到长期文档。

子代理导航与编辑回归：`cargo test --locked --features ui-tests --bin conduit app::subagents:: -- --test-threads=1`。覆盖主任务持续输出、任务行/详情/返回的键盘操作、根草稿与输入选区保留、错误目标与重复原生 ID 拒绝关联、读取失败返回、标签路由恢复及主记录缺失时禁止误发。同时覆盖独立子草稿、附件恢复、来源身份隔离、发送前保存、定向控制回执和根记录删除保护；真实 CLI 的子会话写入或停止能力由下述原生探针验证。


子会话交互回归使用 `app::subagents::`、`protocol::subagents::`、`cli_adapter::child_history::` 三组过滤器，覆盖独立输入恢复、历史/实时去重、保存门闩、旧回合拒绝及未知命令不重发。跨模块回归包括 `app::runtime::saves::`、`app::queue::`、`app::transcripts::`、`app::update::` 和通知/审批测试。

原生协议探针：`python scripts/probe-subagent-protocol.py --codex <绝对可执行文件路径>`。探针使用独立 CODEX_HOME 和本地合成模型响应，验证真实父子关系的定向控制、历史与续聊，不连接真实模型服务。成功必须包含全部断言；结果和原生事件在 `target/subagent-protocol-probe/`，不作为实际模型质量或真实窗口验收证据。

多引擎子历史回归增加 `history::children::` 和 `protocol::claude_tasks::`，覆盖 OpenCode 全量快照/祖先链、Claude 分支/sidecar/结构化 agent 身份、截断尾行与损坏记录、累计读取预算和数据源覆盖。`app::subagents::` 同时验证原生 provider 身份、父连接持有时禁止续聊、Claude 只读降级以及错误来源不重新绑定。

OpenCode 子会话端到端验收显式执行：`cargo test --locked --features ui-tests --bin conduit chat_runtime::opencode_tests::installed_opencode_subagent_lifecycle -- --exact --ignored --nocapture --test-threads=1`。使用已安装 OpenCode、隔离 XDG/数据库目录及本地 HTTP/SSE 模拟模型；真实创建子任务，读取其原生历史，等待父连接清理后恢复原有子会话 ID，断言旧子上下文保留且父消息不变。Claude 前台/后台子代理生命周期与只读历史验收显式执行：`cargo test --locked --features ui-tests --bin conduit chat_runtime::claude_subagent_tests:: -- --ignored --nocapture --test-threads=1`。隔离 `CLAUDE_CONFIG_DIR`，使用本地 Anthropic SSE 模拟模型，验证真实 Agent/Read 工具流、主子正文隔离、后台子任务延迟完成时父 runtime 等待、子代理 JSONL 历史读取及读取不修改源文件。两组测试均不需要模型账户，但不代表真实模型服务、认证或窗口验收；Claude 子历史仍不可独立恢复、追加或定向中断。
