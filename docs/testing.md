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

## 常规检查与定向回归

```powershell
cargo fmt --all --check
cargo clippy --locked --all-targets --features ui-tests -- -D warnings
cargo test --locked -- --test-threads=1
cargo test --locked --features ui-tests --bin conduit app:: -- --test-threads=1
```

按改动选取以下筛选范围；小修改先运行相关回归，发布或较大改动再执行完整检查。

| 改动 | 推荐范围 |
| --- | --- |
| 保存回执、发送取消 | `app::runtime::saves::tests`、`app::queue::tests` |
| 正文、迁移、回收 | `transcript`、`store::`、`app::transcripts::tests` |
| 历史与搜索 | `history::`、`app::ui_tests::history_and_search` |
| 协议和适配器 | `protocol::`、`cli_adapter::`、`chat_runtime::` |
| Git 与通知 | `git_` / `git::`、`notifications::` |
| 终端 | `terminal::`、`app::terminal::tests` |
| 文件与图表 | `file_preview`、`file_search`、`file_ref`、`mermaid` |
| 设置与 CLI 更新 | `settings`、`cli::updates` |
| 应用更新 | 库 `update::`、`app::update::tests` |
| 插件 | [插件验证](plugins/validation.md)及[原型命令](../tools/shell-prototype/README.md) |

UI 范围使用 `--features ui-tests --bin conduit`。例如：

```powershell
cargo test --locked --features ui-tests --bin conduit app::update::tests -- --test-threads=1
cargo test --locked --lib update:: -- --test-threads=1
```

选择筛选词后核对实际执行用例，零用例不算通过。默认忽略的真实 OpenCode + 本地模拟端点测试可以单独运行：

```powershell
cargo test --locked installed_opencode_local_protocol_smoke -- --ignored --nocapture --test-threads=1
```

该测试不访问真实模型账户，但需要已安装的 CLI；它不替代服务商认证和真实模型兼容验收。

## Windows 人工验收

[smoke.ps1](../scripts/smoke.ps1)提供窗口启动冒烟，[ui-acceptance.ps1](../scripts/ui-acceptance.ps1)提供自动预验。运行前阅读脚本参数和环境要求。隐藏进程仍在运行、一次截图或 GPUI 模拟输入均不能替代完整人工验收。

| 场景 | 验收重点 |
| --- | --- |
| 输入与 IME | 微软拼音和另一常用输入法；候选确认不误发送，换行、`@`、双 Shift、Esc 与粘贴互不抢占 |
| 会话并发 | 多工作区/多引擎持续输出，切换保留草稿和阅读位置，停止只影响目标任务，审批不会串会话 |
| 队列与恢复 | 排队、撤回编辑、保存失败、取消、普通退出及更新后恢复正文与附件，不重复发送 |
| 阅读与文件 | 长回复、代码、Mermaid、文本选择复制、冷正文加载/重试、精确/模糊查找和独立滚动 |
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
