# 参与贡献

欢迎提交问题反馈和 Pull Request。较大的功能或架构调整，建议先用 Issue 讨论范围。

## 开发环境

- Windows x64。
- Rust MSVC 工具链；CI 使用 Rust 1.99.1。
- Visual Studio Build Tools 的 C++ 工具与 Windows SDK。
- 手工验证对话功能时，需要安装并登录相应 CLI；Git 功能需要本机 Git。

```powershell
cargo run --locked
cargo run --locked -- --settings
```

使用独立数据目录，避免测试覆盖日常会话：

```powershell
$env:CONDUIT_DATA_DIR = Join-Path (Get-Location).Path 'target/dev-data'
cargo run --locked
```

## 提交前检查

```powershell
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
```

修改界面时，补充相关 UI 验证：

```powershell
cargo test --locked --features ui-tests --bin conduit app::update::tests -- --test-threads=1
```

上例是更新界面的测试筛选，其他改动应选择对应测试。依赖真实 CLI、登录或模型额度的测试默认忽略，不要在常规 CI 中启用。

## Pull Request

说明解决的问题、用户可见的变化和验证方式；界面修改可附截图。保持改动集中，新增行为更新相应文档，不提交凭证、会话数据或构建产物。

协议适配与进程管理见 [技术设计](docs/technical-design.md)，安装包和版本标签规则见 [自动发布与更新](docs/auto-update.md)。
