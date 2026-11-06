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

## 测试归属与运行范围

测试按行为所属模块组织。小型单元测试放在模块内的 `mod tests`；较大的测试组放在模块旁的 `tests.rs`。新增回归用例优先加入已有测试组，避免按审查批次、日期另建文件。相同初始化过程可以复用，但不同失败场景保留独立断言。

| 范围 | 位置或筛选条件 | 运行时机 |
| --- | --- | --- |
| 协议、存储、进程等单元测试 | 对应模块的 `tests` | 修改相关逻辑及提交前 |
| 消息队列 | `app::queue::tests` | 入队、撤回、后台续发、会话切换 |
| 保存确认与失败回滚 | `app::runtime::saves::tests` | 持久化、发送、取消和退出 |
| 历史导入、大纲更新、搜索刷新 | `app::ui_tests::history_and_search` | 历史合并与搜索逻辑 |
| 其他 UI 回归 | 各功能模块的 `tests` 或 `app::ui_tests` | 修改对应交互与布局 |
| Shell 原型与桌面适配 | `tools/shell-prototype` 的验证入口 | 修改插件或 Shell 接入，见该目录 README |
| 桌面性能探针 | `verify-desktop.py --phase performance` | 显式运行；默认忽略，不混入普通回归 |

UI 测试需加 `--features ui-tests --bin conduit`，例如：

```powershell
cargo test --locked --features ui-tests --bin conduit app::ui_tests::history_and_search -- --test-threads=1
```

完整 UI 回归使用筛选条件 `app::`；根目录 CI 仍执行该范围。整理测试时应核对用例名称与断言，避免因模块迁移而漏注册测试。`target/` 中的验证副本、测试数据和编译产物不属于测试源码；清理前确认没有相关进程使用，注册的 Git worktree 用 `git worktree remove` 移除。

## Pull Request

说明解决的问题、用户可见的变化和验证方式；界面修改可附截图。保持改动集中，新增行为更新相应文档，不提交凭证、会话数据或构建产物。

协议适配与进程管理见 [技术设计](docs/technical-design.md)，安装包和版本标签规则见 [自动发布与更新](docs/auto-update.md)。
