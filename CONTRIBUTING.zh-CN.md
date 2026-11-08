# 参与贡献

[English](CONTRIBUTING.md) · **简体中文** · [项目首页](README.zh-CN.md)

感谢你帮助改进 Conduit。代码、问题反馈、文档、翻译和实际使用体验都可以成为贡献。改进文档或反馈问题，无需准备 Rust 开发环境。

## 选择参与方式

- **反馈问题：** 先搜索[已有 Issue](https://github.com/Leo-Lee-j/conduit/issues)，再提供可复现的示例。
- **提议功能：** 说明你想完成的任务，以及当前流程缺少什么。较大的功能、新依赖或架构调整，请先讨论再实现。
- **改进文档：** 修正不清楚的步骤、失效链接或过时说明。修改共用内容时，同步维护中英文项目首页和贡献指南。
- **提交修复：** 聚焦具体问题，补充适当的回归测试，并在 Pull Request 中说明修改后的行为。

## 提供有效的问题反馈

反馈 Bug 时，请包含：

- Conduit 版本、Windows 版本，以及相关 CLI 的名称和版本（如适用）。
- 复现步骤、预期结果与实际行为。
- 相关错误文本、简短日志片段或截图。界面问题请附上系统缩放比例与主题。
- 能否在小型示例工作区中复现。

分享日志或截图前，请移除凭据、私有代码与对话数据。有效的问题报告不需要上传完整数据目录。

## 准备开发环境

Conduit 当前面向 **Windows x64**。修改 Rust 代码需要：

- Git、Rust MSVC 工具链，以及 `rustfmt` 和 `clippy` 组件。
- Visual Studio Build Tools 中的 C++ 构建工具与 Windows SDK。
- 与 [Windows CI](.github/workflows/ci.yml) 一致的 Rust 工具链版本。版本以该工作流为准；依赖由 `Cargo.lock` 和 `Cargo.toml` 中的 GPUI Kit revision 固定。

手工验证某个 CLI 集成时，再安装并登录对应的 Claude Code、Codex 或 OpenCode。需要已安装 CLI 或真实账户的测试必须显式选择，详见[测试与验收](docs/testing.md)。

### 克隆并运行

如果没有仓库写权限，请先 Fork。以下命令克隆上游仓库；通过 Fork 贡献时，将克隆地址替换为你的 Fork 地址。在 PowerShell 中运行：

```powershell
git clone https://github.com/Leo-Lee-j/conduit.git
cd conduit
git switch -c fix/describe-your-change
$env:CONDUIT_DATA_DIR = Join-Path (Get-Location).Path 'target/dev-data'
cargo run --locked
```

`CONDUIT_DATA_DIR` 将开发实例的 Conduit 数据与日常安装版分开保存，但不会隔离所选 CLI 的配置或打开的工作区文件。测试文件修改和 Git 操作时，请使用临时项目或独立 Git worktree。

如需直接进入设置，保留上述数据目录设置并运行：

```powershell
cargo run --locked -- --settings
```

该环境变量对当前 PowerShell 会话中启动的命令生效。排查启动失败时，请保留终端及其输出。

## 聚焦修改范围

遵循所修改模块的现有约定。一个 Pull Request 聚焦一个问题或一组相关修改，避免混入无关格式调整或依赖更新。

回归测试放在已有行为模块或相邻 `tests.rs` 中，验证可能被破坏的行为，并覆盖相关失败或取消路径。显式集成测试和性能探针与普通回归分开维护。

行为变化时同步更新相关文档。可先从[架构说明](docs/architecture.md)找到负责该行为的模块。

## 验证修改

以下命令均在仓库根目录执行。开发过程中先运行受影响模块的测试；筛选范围与人工检查方法见[测试与验收](docs/testing.md)。

提交 Rust 修改前，运行：

```powershell
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
cargo build --locked --bin conduit --bin conduit-updater
git diff --check
```

涉及应用界面时，还需运行：

```powershell
cargo clippy --locked --all-targets --features ui-tests -- -D warnings
cargo test --locked --features ui-tests --bin conduit app:: -- --test-threads=1
```

这些检查与 [CI](.github/workflows/ci.yml) 的构建及测试范围一致。请核对测试摘要：筛选后零用例不代表验证通过。准确记录失败、跳过、中断与未验证范围。不要用笼统的 `--ignored` 启用所有忽略测试；需要真实账户、模型调用或额外软件的测试应单独选择。

界面修改还应在原生应用中实际操作受影响流程，检查相关主题、窗口尺寸、DPI 和键盘行为。必要时附截图说明变化；自动化测试不能替代视觉验证。

**仅修改文档时**，检查链接、代码示例、语言跳转及其与仓库的对应关系，再运行 `git diff --check`，无需执行 Cargo 构建或测试。

## 维护有用的文档

[文档首页](docs/README.md)按使用指南、开发与发布、实验扩展组织内容。

- 同一主题、同一语言维护一份当前指南。详细命令放在负责该流程的文档中，其他页面通过链接引用。
- 中英文项目首页与贡献指南保持对应。翻译页面后，同步更新语言跳转并移除过时的语言标签。
- 描述当前已经实现的行为。明确标注方案与尚未完成的验证，修复后移除失效约束。
- 仓库内部内容优先使用相对链接。移动文件时，同步更新入口链接、图片引用和相关脚本注释。
- 长期文档保留可复用的方法与约束。逐次测试日志、截图和基准输出放入已忽略的 `target/` 或工具产物目录。
- 在资源目录保留来源、生成方式与许可信息。不提交密钥、原始对话数据或构建产物。

清理临时文件前确认归属及使用状态；已注册的 worktree 通过 Git 管理。

## 提交 Pull Request

将分支推送到你的 Fork 或有权限的仓库分支，再向 `main` 发起 Pull Request。

清晰的 PR 描述应包含：

- **问题：** 触发条件和需要改变的行为；如有 Issue，附上链接。
- **结果：** 用户将看到的变化，以及重要实现取舍或兼容性影响。
- **验证：** 实际执行的命令和人工步骤、结果，以及剩余验证缺口。
- **文档或截图：** 相关指南更新，以及必要的界面截图。

标题与描述应对应最终补丁。评审后继续修改时，重新执行受影响的检查并更新验证说明，不把较早版本的测试结果当作新修改的验证结果。

## 延伸阅读

详细指南目前以中文维护：

- [架构](docs/architecture.md)：模块、产品边界与数据流。
- [测试与验收](docs/testing.md)：定向回归与原生 Windows 检查。
- [性能与指标](docs/performance.md)：测量方法、有界工作与缓存。
- [发布与更新](docs/releasing.md)：打包、签名与失败恢复。
- [插件开发](docs/plugins/README.md)：已支持的贡献机制与实验运行时。
