# P2-A：真实桌面开发接入

日期：2026-11-05。状态：**Shell 开发加载器已接入真实 ConduitApp；正式发布依赖仍未启用，G4–G6 继续按未覆盖项验收。**

后续更新：本文保留桌面接入当轮基线；文末的 ShellRoot 浮层所有权缺口已由 [G6 浮层清理](p2-overlay-cleanup-2026-11-05.md)接续实现。当前下一步为真实 IME / 浮层焦点及性能验收，未来存储写入屏障仍未完成。

本轮承接[开发加载适配器](p2-development-loader-2026-11-05.md)，将同一清单和运行时接到现有导航、右侧容器与底栏。验证入口直接编译 `src/main.rs`，不是另做一套模拟工作台。安装、信任持久化和版本回滚仍属 P3。

## 入口与构建边界

- 新增 `tools/shell-prototype/desktop` workspace member，二进制为 `conduit-shell-desktop`。只有该包设置 `conduit_shell_desktop` cfg。
- 正常 `cargo run --locked` 保持现有应用入口；Shell 候选依赖、上游补丁与实验 lockfile 留在 `tools/shell-prototype`。根 manifest 没有加入 Shell 依赖，根 lockfile 未因本轮改变。
- 验证窗口采用 Kit Root → ShellRoot → ConduitApp。原型库启用 `desktop` feature 时直接复用 `conduit::plugins` 类型。
- `verify-desktop.py` 检查桌面包版本和主工程依赖声明一致；实验锁中的 `ureq 3.5.0` / `ureq-proto 0.6.5` 与主工程保持一致。本轮未改上游候选补丁。

在仓库根目录运行：

```powershell
python tools/shell-prototype/verify-desktop.py --phase build
$env:CONDUIT_DATA_DIR = Join-Path (Get-Location).Path 'target/shell-desktop-dev-data'
target/shell-prototype/debug/conduit-shell-desktop.exe `
  --dev-plugin tools/shell-prototype/fixtures/development-overview `
  --allow-permission workspace.metadata.read `
  --allow-permission session.summary.read
```

需在验证窗口添加工作区后才能显示 workspace 作用域入口。也可使用下述冒烟脚本自动建立独立的合成工作区。桌面开发参数一次接受一个包，权限必须逐项批准；参数或清单错误以退出码 2 结束。无开发参数时不创建 Shell 插件宿主；原有 `--settings`、`--git`、`--history` 入口保留。

## 实现行为

主实现为 [plugin_shell.rs](../../src/app/plugin_ui/plugin_shell.rs)，由现有 `plugin_ui.rs` 按 cfg 引入：

1. 一个 UI Registry 登记 core 与 Shell 静态贡献，运行时适配器在 VM 边界再次核验声明。原生示例与 Shell 示例共用 `example.workspace-overview` 身份，互斥登记。
2. 发现和登记不执行脚本。点击导航入口、右侧视图或底栏命令才创建实例；命令先展示目标容器，再建立 ticket，避免可见性刷新取消新请求。
3. 宿主只复制当前工作区名称和会话数量；对外工作区 ID 是内部路径身份的 SHA-256，API 不返回路径或聊天正文。摘要相等时跳过刷新，避免 33ms 轮询反复构建脚本视图。
4. 左、右视图有各自的 ViewInstanceKey 和脚本状态；底栏命令按 owner、view 和当前 scope 路由。命令不抢占输入框焦点。
5. 工作区、会话、文档或可写状态变化使上下文失效，立即停止 VM、取消待完成结果；之后等待用户显式重新打开。隐藏实例保留有界缓存，重新显示时更新摘要。
6. 直接撤销 Registry 注册句柄也会停止对应运行时。每个脚本视图提供宿主“停止开发插件”按钮，停用后撤销三处入口，导航和右栏按 P1 规则回退，旧动作不能重新激活插件。
7. 现有关闭回调和更新准备调用 `stop_instances` 时同时停止 Shell。验证的是当前只读 HostModule 下的任务取消与 VM 释放；还不是未来插件存储写入的完整屏障。

## 验证

```powershell
python tools/shell-prototype/verify-desktop.py --phase check
python tools/shell-prototype/verify-desktop.py --phase test
python tools/shell-prototype/verify-desktop.py --phase fast-check
python tools/shell-prototype/verify-desktop.py --phase release-check
python tools/shell-prototype/verify-desktop.py --phase build
pwsh -NoProfile -File tools/shell-prototype/smoke-windows.ps1 -Desktop
```

runner 保存真实命令、退出码、耗时和摘要到 `tools/shell-prototype/artifacts/desktop-*-results.json`，并记录检查前后源文件摘要以及共享工作树是否在检查期间变化。`test` 使用独立 `CONDUIT_DATA_DIR`，插件与更新测试串行执行。格式检查针对本轮接入文件，使用 `skip_children=true`，不格式化共享工作树的其他模块。

| 检查 | 本轮结果 |
| --- | --- |
| 默认主应用普通 check | 通过 |
| 默认主应用 Clippy all-targets `-D warnings` | 通过 |
| 根工程全量 rustfmt | 最终复查通过；未全量格式化共享工作树 |
| Shell 桌面 all-targets + ui-tests check | 通过 |
| 接入文件 rustfmt | 通过 |
| Shell 桌面插件 UI 回归 | 37 项通过，其中 7 项为本轮桌面 Shell 用例 |
| 更新 UI 回归 | 3 项通过，包含本轮的 Shell 更新准备停止用例 |
| 默认桌面 dev 构建、原生窗口 | 编译链接通过；六张截图核对初始、激活、命令、850px 窄屏、设置弹窗和停用；WM_CLOSE 后退出码 0 |
| 桌面 CLI | 缺少批准、未知权限、无包授权均以 2 拒绝；旧 `--history` 入口以 0 完成，共 4 项通过 |
| 独立原型 adapter 回归 | 33 项库测试 + 28 项 UI / model 测试通过；6 项有意故障探针本轮未重跑，前轮结果仍见开发加载器记录 |
| 原型 rustfmt / check / Clippy all-targets `-D warnings` | 通过 |
| Fast / release 类型检查 | Fast all-targets + ui-tests 及默认后端 release 均通过；Fast 仍有上游滚动事件旧名称的弃用警告，未构建 / 运行这两种成品 |
| 桌面 Clippy all-targets + ui-tests `-D warnings` | 最终复查通过 |

新增测试覆盖静态入口、真实摘要、双视图、底栏返回与焦点、工作区切换、隐藏后刷新、只读、注册撤销、真实停止按钮、1600/1100/850px 容器、宿主设置弹窗、中文文本输入、关闭及更新准备。关闭测试模拟平台关闭回调，验证应用等待常规关闭轮询期间脚本已停止；更新用不存在的安装包验证准备接缝，不启动安装器。

原生脚本仅捕获及操作本次启动 PID 的 HWND。每次创建 GUID 数据目录和空工作区，关闭自动更新，最后发送 WM_CLOSE 并确认进程退出；截图内容须人工检查，截图文件生成本身不作为渲染通过依据。

本轮已核对 `desktop-native-{initial,activated,command,narrow,settings,stopped}.png`：左侧显示 `native-workspace` 与 `Sessions: 0`；底栏命令使右侧显示 `Command count: 1`；850px 时右栏覆盖中间内容并位于底栏上方；设置弹窗可打开并由 Esc 关闭；停用后左右脚本和底栏入口均撤销，导航回到工作区。日志、截图和 CLI 结果均留在忽略的 artifacts 目录。

验证期间共享工作树的异步保存改动影响了既有更新草稿用例。本轮把该用例调整为先 `persist` 并等待测试保存回执，再调用生产 `save_update_drafts`，移除了该生产函数中的测试专用提交分支。另外补上 `OpenCodeSettings` 的 `PartialEq` 派生及清理循环中 `termination_error.as_ref()`，解决并行改动暴露的编译阻塞；滚轮测试改用 `InputEvent::to_platform_input`，兼容默认与 Fast 的枚举名称差异；四处布尔条件按 Clippy 做等价简化。其余聊天、Git 等正在进行的改动不属于本轮插件实现。

本轮候选摘要：

```text
lock_sha256   ce5d9dfb36707658eee3509f4799bb2922f0598bb89ded55bc63ed5f185dbbc5
patch_sha256  d52758cff6094b716cb2b06f4b2555532fbd78118309a9b036eaa619af0d595f
source_sha256 2fc1d08ad65257e6f879ec3310b4d68052d3ba9fed7f2d015c7119aaf67cfaf8
```

`source_sha256` 在此指准备后的上游补丁源码；桌面工程源文件摘要单独记录在 runner JSON 中。

## 仍需验收

- **G4**：真实 Windows IME 组合输入；Shell 与宿主浮层同时存在时的层级、焦点、Esc、tooltip、主题切换。模拟中文文本输入不等于输入法验收。
- **G5**：完整应用主线程交互 p95、宿主总内存与原生视图资源开销。现有解释器限额、故障探针或类型检查不能代替这些测量。
- **G6**：本轮之后已补齐 ShellRoot 按运行时撤销浮层及焦点恢复，见[后续记录](p2-overlay-cleanup-2026-11-05.md)；未来插件存储写入屏障仍未实现。
- Fast / release 的 check 不生成对应成品；必须另外完成链接和实际运行。快照复制与激活核验仍是有界同步 I/O，尚未通过生产性能准入。

ShellRoot 浮层所有权和统一清理已由后续轮次接续；下一步完成真实输入法 / 焦点验证与性能测量，通过后再冻结 P2-A 后端契约并进入 P3。
