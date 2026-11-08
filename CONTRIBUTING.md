# 参与贡献

问题反馈与 Pull Request 应说明具体触发条件、预期和实际行为。较大的功能或架构调整先明确范围、实现边界和验证方法。

## 开发环境

- Windows x64、Rust MSVC 工具链、Visual Studio Build Tools 的 C++ 工具和 Windows SDK。
- 工具链与依赖版本以 [CI](.github/workflows/ci.yml)、`Cargo.lock` 和固定 GPUI Kit revision 为准。
- 手工验证对话需安装并登录对应 CLI；Git 功能需本机 Git。

```powershell
cargo run --locked
cargo run --locked -- --settings
```

使用独立数据目录，避免影响日常会话：

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

修改界面还需启用 `ui-tests` 做 Clippy 和对应回归；根目录 CI 执行完整 `app::` 范围。模块筛选、隔离环境、真实 CLI 和人工验收见[测试与验收](docs/testing.md)。只修改文档时检查链接、命令与源码一致性及差异即可，不用无关运行测试代替文档校验。

测试按行为归属放在现有模块或相邻 `tests.rs`，不按审查日期新建文件。显式集成/性能探针不混入普通回归；零用例、忽略、中断和超时不能记成通过。

## 文档维护

[文档首页](docs/README.md)是总入口，按使用、开发发布、实验扩展分组。

- 一个主题维护一份当前说明。功能修改更新相应文档，不继续新增“评估—实施—第 N 轮验证”的重复链条。
- 当前实现、未实现方案和验收缺口分开标注。源码已变化时删除失效描述，不把旧问题永久当作待办。
- 操作命令放在负责该流程的文档或工具 README，其他页面链接引用。命令、参数、路径和版本应能由仓库验证。
- 逐次测试日志、截图、基准原始数据放在 `target/` 或工具 artifacts；长期文档保留方法、约束和未完成项。历史过程从 Git 追溯。
- 使用相对链接，迁移或删除文件时同时更新 Markdown、图片、脚本注释及入口。相邻资源 README 保留来源、生成方法与许可说明。
- 不提交密钥、原始会话数据或构建产物。清理临时目录先确认归属及使用状态；已注册 worktree 通过 Git 管理。

## Pull Request

说明解决的问题、用户可见行为、实现取舍及实际验证结果；界面修改附必要截图。保留失败和未验证范围，不把历史通过数当作本次结果。

架构见[架构说明](docs/architecture.md)，发布版本与安装包规则见[发布与更新](docs/releasing.md)，实验插件见[插件文档](docs/plugins/README.md)。
