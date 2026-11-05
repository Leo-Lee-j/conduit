# Conduit GPUI Shell 验证原型

2026-11-05 G4 后续：开发夹具的摘要、dialog、sheet 新增原生输入框，显示输入值和当前主题。桌面回归覆盖组合输入接口、粘贴、主题、Tab / Esc、设置页返回与窄窗状态保留；真实 Windows IME 尚未验收，详见 [G4 进度](../../docs/plugins/p2-input-focus-2026-11-05.md)。

2026-11-05 G6 后续：增加运行时浮层所有权补丁，停止时撤销 dialog / sheet / toast / HUD / tooltip，保留其他 owner，修复焦点恢复。桌面停用、关闭与更新准备支持同步清理，详见 [G6 实现及验证](../../docs/plugins/p2-overlay-cleanup-2026-11-05.md)。真实 IME、完整浮层交互、生产性能和未来插件存储写入屏障继续验收。

2026-11-05 桌面接入：新增 `conduit-shell-desktop` 验证入口，直接编译真实 ConduitApp，
将开发插件接到现有导航、右侧和底栏，读取受权限限制的真实工作区摘要。构建和验收边界见
[真实桌面开发接入](../../docs/plugins/p2-desktop-integration-2026-11-05.md)。正式发布仍不启用 Shell。

2026-11-05 G3 后续：已增加定向 `on_command`、弱视图句柄、有界异步结果和统一停止回归。
清单驱动的开发加载器也已接入 P1 Registry、ViewInstanceKey 和 RequestTicket；当前完整测试结果见
[P2-A 开发加载适配器进度](../../docs/plugins/p2-development-loader-2026-11-05.md)。
开发加载器当轮常规测试 61 项通过，6 项有意故障探针分别执行通过；最新结果见 G6 记录，以下初版和 G2/G5 数字保留历史含义。

2026-11-05 后续：原型已增加按插件 Policy 加载和有限额解释器的候选补丁，13 项常规测试与
4 项独立故障测试通过；默认 dev 链接、release 类型检查、Fast 全目标类型检查和 Windows
窗口冒烟验证通过。详细结果及生产验收边界见
[G2 / G5 记录](../../docs/plugins/p2-host-controls-2026-11-05.md)，可重建补丁见
[patches](patches/README.md)。下方“本轮实测”数字为初版上游基线，后续验证单独记录。

本工程用于验证 [Shell 设计](../../docs/plugins/phase-2-shell-runtime.md)的公开 API 接入路径，使用与 Conduit 相同的 GPUI Kit 提交 `11345ec002d9f182a266b5d5dd3a217090700a87`。它使用独立 Cargo workspace / lockfile；默认 member 运行原型，`desktop` member 编译真实桌面源码，正式应用 manifest 不引入 Shell 候选依赖。

默认无参数模式只加载工程内的 G3 JS 夹具，使用合成工作区 A / B。开发模式可以接收用户明确指定的本地插件目录，先建立会话内快照并要求逐项批准业务权限；不会读取聊天 Store、CLI 配置、凭据或真实会话，也不会持久化信任。它不是完整安装器或生产多插件宿主。

## 运行

在仓库根目录执行：

```powershell
python tools/shell-prototype/verify.py --phase build
cargo run --locked --manifest-path tools/shell-prototype/Cargo.toml --target-dir target/shell-prototype
```

检查清单而不执行脚本：

```powershell
cargo run --locked --manifest-path tools/shell-prototype/Cargo.toml --target-dir target/shell-prototype -- `
  --inspect-plugin tools/shell-prototype/fixtures/development-overview
```

打开清单驱动的开发窗口：

```powershell
cargo run --locked --manifest-path tools/shell-prototype/Cargo.toml --target-dir target/shell-prototype -- `
  --dev-plugin tools/shell-prototype/fixtures/development-overview `
  --allow-permission workspace.metadata.read `
  --allow-permission session.summary.read
```

开发窗口初始不创建 VM；点击声明的 view 或底部 status command 才激活对应 owner。快照、路径和权限边界见
[开发加载适配器进度](../../docs/plugins/p2-development-loader-2026-11-05.md)。

左侧 W 图标与中间内容底栏都可以首次激活脚本。左侧脚本可以打开右侧摘要、延迟更新底栏；右侧有独立计数器。顶部切换合成工作区、停用、触发 / 恢复渲染异常、切换宿主只读状态。内容区可以打开组件弹窗和 Shell 弹窗。850px 窗口下右侧覆盖内容，1200px 下独立布局；这仅是宿主布局接缝验证，不替代 Conduit 现有布局回归。

工作区切换会停止旧 VM；默认夹具自动重建，开发模式等待下次显式打开或命令。开发适配器已复用 P1 的 ViewInstanceKey 缓存，同一上下文的数据刷新保留实例状态。下述独立桌面入口进一步接入现有三处容器和溢出规则；原型窗口仍不提供持久化或安装 / 更新流程。

点击内容区 `Focus host content` 后，再点击底栏 `Increment summary`，右侧计数递增、
左侧计数保持，内容区显示返回结果，宿主焦点保持不变。停止或切换工作区后旧命令与结果失效。

## 真实桌面验证入口

```powershell
python tools/shell-prototype/verify-desktop.py --phase build
$env:CONDUIT_DATA_DIR = Join-Path (Get-Location).Path 'target/shell-desktop-dev-data'
target/shell-prototype/debug/conduit-shell-desktop.exe `
  --dev-plugin tools/shell-prototype/fixtures/development-overview `
  --allow-permission workspace.metadata.read `
  --allow-permission session.summary.read
```

在验证窗口添加工作区后可看到 workspace 作用域的插件入口。`--settings` / `--git` 可与开发加载参数合用，`--history` 仍走原有命令行入口。无开发参数时不加载 Shell 包。宿主停止按钮会撤销本次开发会话中的贡献；重新加载需重新启动验证程序。

```powershell
python tools/shell-prototype/verify-desktop.py --phase check
python tools/shell-prototype/verify-desktop.py --phase test
python tools/shell-prototype/verify-desktop.py --phase fast-check
python tools/shell-prototype/verify-desktop.py --phase release-check
pwsh -NoProfile -File tools/shell-prototype/smoke-windows.ps1 -Desktop
```

桌面 runner 核对依赖声明同步，保存 `desktop-*-results.json` 和日志。测试与冒烟使用独立数据目录；冒烟自动建立空的合成工作区，只操作本次 PID 的窗口。需要检查截图内容才能认定交互通过。桌面模式运行完整 ConduitApp，适配器向脚本提供的仅是经过过滤的元数据；前面的“不读取聊天 Store / CLI 配置”只适用于独立原型窗口。

默认开发夹具右侧提供 `Open plugin overlays` 按钮；`smoke-windows.ps1 -Desktop -Overlays` 捕获脚本弹窗、Esc 后的 sheet，再在浮层保持打开时通过 WM_CLOSE 退出，用于 G6 原生关闭验证。

## 重现原型检查

```powershell
python tools/shell-prototype/verify.py --phase check
python tools/shell-prototype/verify.py --phase test
python tools/shell-prototype/verify.py --phase fast-check
python tools/shell-prototype/verify.py --phase release-check
```

`check` 包含 rustfmt、默认后端 check 和全部目标 Clippy；`test` 包含宿主逻辑与 GPUI 测试上下文中的交互测试。runner 自动从精确提交加补丁准备上游副本，校验源码 SHA；测试串行执行。首次构建依赖较多，每条编译命令最多 30 分钟，日志和真实退出码保存在 `artifacts/`，该目录不提交。

死循环探针默认忽略，必须单独调用：

```powershell
python tools/shell-prototype/verify.py --phase watchdog
python tools/shell-prototype/verify.py --phase resources
python tools/shell-prototype/verify.py --phase commands
python tools/shell-prototype/verify.py --phase development-fault
```

它们先完成测试编译，再在独立测试进程中执行无限 render 和有上限的资源故障探针，测量中断、停止与恢复。runner 的 120 秒外部超时用于停止本次测试及其子进程，不是插件运行时保护。堆探针使用 32 MiB VM 限额和最多 128 MiB 的脚本申请，故障测试默认忽略。

`release-check` 只证明 release 配置类型检查，不代表已链接 release 可执行文件或通过真实 GPU 性能测试；`fast-check` 同理。当前实测状态见[原型验证记录](../../docs/plugins/shell-prototype-validation.md)。

## 初版实测与 Windows 冒烟

2026-11-05：默认 dev 编译 / 链接、release 类型检查、Clippy 通过；常规测试 12 项通过，独立 watchdog 测试 1 项通过。真实 Windows 窗口已启动、底栏激活后显示左右视图，并以退出码 0 关闭。

**未打补丁的初版 Fast 检查失败**：固定上游 `gpui-base` 三处无参数 `Modifiers::secondary()` 调用与 `gpui-fast 0.1.11` 的 `secondary(&App)` 不兼容。这些初版缺口已由后续候选补丁推进：Fast 全目标类型检查通过，Policy 注入与有限额解释器的验证结果见 [本轮进度](../../docs/plugins/p2-host-controls-2026-11-05.md)。

构建后可重现 Windows 窗口检查：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/shell-prototype/smoke-windows.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File tools/shell-prototype/smoke-windows.ps1 -Development
```

脚本会短暂显示本次启动的原型窗口，向该窗口底栏发送激活和定向命令点击并关闭它，保存启动 / 激活 / 命令结果截图与 `native-smoke.json` 到 `artifacts/`。截图需视觉核对；脚本不操作其他应用的窗口，也不代表 GPU 性能验收。

`-Development` 使用 `development-overview` 包，检查清单加载、左侧 view、底栏 command 和停止，输出 `development-native-*` 截图和结果。2026-11-05 已视觉核对：左侧摘要显示 3 个会话，右侧命令计数为 1，停止后两侧清空，关闭退出码为 0。CLI 静态检查成功，缺少批准权限时在窗口创建前退出。

## 公共 API 验证范围

- 一个显式传入 Policy 的 `conduit-probe` HostModule，身份由宿主闭包绑定；UI 状态通过 generation / contextVersion 校验。
- 补丁公开的 `load_view_with_policy` 创建两个不同入口视图，每个入口保有自己的状态和应用代次；VM 由该插件独占。
- 窗口结构为组件 / Base Root → ShellRoot → Rust Probe。宿主继续绘制功能入口和底栏，脚本只填充内容。
- 只在显式点击后创建 VM、求值代码。加载前关闭 editor files，夹具不含动态 dependencies。
- 只读和停止时拒绝新调用并释放实体，测试检查 VM 弱引用与异步续体。

原型已移除默认全局 Policy 设置。旧 `PluginManager::load` 不注入宿主模块的负例保留；新的 `load_with_policy` 用双插件授权、异步续体和卸载/重载测试验证。开发加载器通过两份清单的 entry / binding 接到 P1 实例与 ticket；正式桌面接入仍待验收。

## 构建条件

锁定的上游 workspace 使用 `rquickjs-compat` 门面让 Shell 与 LLRT 共享 QuickJS 类型。依赖 workspace 的 patch 不会被当前根工程继承，因此本工程显式使用同一提交的 `[patch.crates-io] rquickjs`。默认关闭 Shell 的 WebView feature，GPUI Fast 开关同时转发给 Kit 和 Shell。

首次自由解析时，`rsa 0.10.0-rc.19` / `pkcs1 0.8.0` 导致固定 LLRT 的 `llrt_crypto` 出现六处 E0282 / E0284。原型锁回上游 lock 使用的 `rsa 0.10.0-rc.18` / `pkcs1 0.8.0-rc.4` 后继续验证；请使用已提交的 Cargo.lock 和 `--locked`。本工程不修改 Cargo 缓存中的任何源码。

## 文件

| 文件 | 用途 |
| --- | --- |
| `src/main.rs` | Rust 宿主、脚本加载、三处布局与 HostModule |
| `src/manifest.rs` / `src/package.rs` | 两份清单解析、P1 静态注册映射、路径 / 摘要 / 会话快照 |
| `src/adapter.rs` | 开发 Host、按 owner Policy / VM、ViewInstanceKey、ticket 和停止边界 |
| `src/development.rs` / `src/development_tests.rs` | 清单驱动开发窗口，以及加载、权限、上下文、快照和故障集成测试 |
| `src/model.rs` | 作用域、身份、只读、状态消息校验及行为测试 |
| `src/ui_tests.rs` | GPUI 集成探针与明确预期的缺口重现 |
| `src/controls_tests.rs` | 按插件授权、卸载重载、资源限制与恢复 |
| `src/commands.rs` / `src/commands_tests.rs` | 定向路由与宿主上下文票据、同步/异步结果及停止边界验证 |
| `prepare_upstream.py` / `patches/` | 精确上游源码加可审核补丁、哈希校验与可重复重建 |
| `fixtures/workspace-overview/` | 双视图、权限、清理和看门狗 JS 夹具 |
| `fixtures/development-overview/` | 可供 --inspect-plugin / --dev-plugin 加载的完整清单示例 |
| `verify.py` | 可重复的构建 / 测试执行器、超时和日志 |
| `smoke-windows.ps1` | 原生窗口启动、指定窗口点击、截图与退出检查 |
| `Cargo.lock` | 实验依赖的固定解析结果 |
