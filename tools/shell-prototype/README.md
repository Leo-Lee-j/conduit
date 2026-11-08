# Conduit GPUI Shell 验证原型

本工程用于验证原生贡献与受控 Shell 开发接入，不启用正式应用的第三方插件安装。架构和当前状态见[插件文档](../../docs/plugins/README.md)，清单/授权见[开发运行时](../../docs/plugins/runtime.md)，准入和未完成项见[验证清单](../../docs/plugins/validation.md)。本页集中维护可运行命令。

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
[开发运行时](../../docs/plugins/runtime.md)。

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

`release-check` 只证明 release 配置类型检查，不代表已链接 release 可执行文件或通过真实 GPU 性能测试；`fast-check` 同理。准入状态见[验证清单](../../docs/plugins/validation.md)。

## Windows 窗口冒烟

构建后可重现 Windows 窗口检查：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/shell-prototype/smoke-windows.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File tools/shell-prototype/smoke-windows.ps1 -Development
```

脚本会短暂显示本次启动的原型窗口，向该窗口底栏发送激活和定向命令点击并关闭它，保存启动 / 激活 / 命令结果截图与 `native-smoke.json` 到 `artifacts/`。截图需视觉核对；脚本不操作其他应用的窗口，也不代表 GPU 性能验收。

`-Development` 使用 `development-overview` 包，检查清单加载、左侧 view、底栏 command 和停止，输出 `development-native-*` 截图和结果。

## 性能与诊断

以下阶段显式运行，默认 dev，可通过 `--profile release` 切换；不属于普通回归。

```powershell
python tools/shell-prototype/verify-desktop.py --phase performance --profile release --samples 200 --warmup 20 --repeats 3
python tools/shell-prototype/verify-desktop.py --phase stress --profile release --cycles 20
python tools/shell-prototype/verify-desktop.py --phase breakdown --profile release
python tools/shell-prototype/verify-desktop.py --phase diagnostics --profile release
python tools/shell-prototype/verify-desktop.py --phase chat-profile --profile release
```

`--target-dir` 可隔离构建输出。每次报告核对源码和可执行文件身份，原始数据与日志保留在新 artifacts 目录；这些 CPU/测试帧结果不能代替真实 GPU、IME 或滚动验收。

原生内存和宿主分项另有显式忽略的测试探针：[native_memory.rs](../../src/app/plugin_ui/plugin_shell/tests/native_memory.rs)、[host_profile.rs](../../src/app/plugin_ui/plugin_shell/tests/host_profile.rs)。它们需要生成的插件包、隔离数据和输出路径，不是 `verify-desktop.py --phase` 选项。内存归因构建启用 `allocation-probe`；测试过滤器和环境参数以相应源码为准，不复制旧 artifacts 中的机器绝对路径。

[windows_cpu_trace.py](windows_cpu_trace.py)提供 runner 可调用的 Windows CPU 采样，保留实际间隔、系统/探针/观察进程累计值及覆盖率；采样关闭对照用于校准开销。它只清理本次直接子进程，需要完整进程树清理的测试仍使用原 runner。统计规则和未完成项见[验证与准入](../../docs/plugins/validation.md)。

## 公共 API 验证范围

- 一个显式传入 Policy 的 `conduit-probe` HostModule，身份由宿主闭包绑定；UI 状态通过 generation / contextVersion 校验。
- 补丁公开的 `load_view_with_policy` 创建两个不同入口视图，每个入口保有自己的状态和应用代次；VM 由该插件独占。
- 窗口结构为组件 / Base Root → ShellRoot → Rust Probe。宿主继续绘制功能入口和底栏，脚本只填充内容。
- 只在显式点击后创建 VM、求值代码。加载前关闭 editor files，夹具不含动态 dependencies。
- 只读和停止时拒绝新调用并释放实体，测试检查 VM 弱引用与异步续体。

原型已移除默认全局 Policy 设置。旧 `PluginManager::load` 不注入宿主模块的负例保留；新的 `load_with_policy` 用双插件授权、异步续体和卸载/重载测试验证。开发加载器通过两份清单的 entry / binding 接到 P1 实例与 ticket；正式桌面接入仍待验收。

## 构建条件

锁定的上游 workspace 使用 `rquickjs-compat` 门面让 Shell 与 LLRT 共享 QuickJS 类型。依赖 workspace 的 patch 不会被当前根工程继承，因此本工程显式使用同一提交的 `[patch.crates-io] rquickjs`。默认关闭 Shell 的 WebView feature，GPUI Fast 开关同时转发给 Kit 和 Shell。

使用已提交的 Cargo.lock 和 `--locked`，其中包含与固定 LLRT 兼容的依赖解析；不要自由升级预发布依赖或修改 Cargo 缓存源码。

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
