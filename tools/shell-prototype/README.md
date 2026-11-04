# Conduit GPUI Shell 验证原型

本工程用于验证 [Shell 设计](../../docs/plugins/phase-2-shell-runtime.md)的公开 API 接入路径，使用与 Conduit 相同的 GPUI Kit 提交 `11345ec002d9f182a266b5d5dd3a217090700a87`。它是独立 Cargo workspace / lockfile，正式应用的依赖与窗口布局不接入本原型。

原型只加载工程内的受信任 JS 夹具，使用合成工作区 A / B。不会读取聊天 Store、CLI 配置、凭据或真实会话，不接受任意插件路径。它不是完整安装器或多插件宿主。

## 运行

在仓库根目录执行：

```powershell
python tools/shell-prototype/verify.py --phase build
cargo run --locked --manifest-path tools/shell-prototype/Cargo.toml --target-dir target/shell-prototype
```

左侧 W 图标与中间内容底栏都可以首次激活脚本。左侧脚本可以打开右侧摘要、延迟更新底栏；右侧有独立计数器。顶部切换合成工作区、停用、触发 / 恢复渲染异常、切换宿主只读状态。内容区可以打开组件弹窗和 Shell 弹窗。850px 窗口下右侧覆盖内容，1200px 下独立布局；这仅是宿主布局接缝验证，不替代 Conduit 现有布局回归。

工作区切换会重建实例，示例计数不会跨工作区保留。尚未实现正式 ViewInstanceKey 缓存、导航更多菜单、图标表、状态项溢出、持久化或安装 / 更新流程。

## 重现检查

```powershell
python tools/shell-prototype/verify.py --phase check
python tools/shell-prototype/verify.py --phase test
python tools/shell-prototype/verify.py --phase fast-check
python tools/shell-prototype/verify.py --phase release-check
```

`check` 包含 rustfmt、默认后端 check 和全部目标 Clippy；`test` 包含宿主逻辑与 GPUI 测试上下文中的交互测试。测试需串行执行，因为 Shell 的默认 Policy 是线程局部宿主配置。首次构建依赖较多，runner 每条命令最多 30 分钟，日志和真实退出码保存在 `artifacts/`，该目录不提交。

死循环探针默认忽略，必须单独调用：

```powershell
python tools/shell-prototype/verify.py --phase watchdog
```

它在独立测试进程中故意执行无限 render，测量上游看门狗的中断与恢复。runner 的 120 秒外部超时用于停止本次测试及其子进程，不是插件运行时保护。堆无限分配不作为常规测试，以免在没有堆限额的宿主中造成机器级内存压力。

`release-check` 只证明 release 配置类型检查，不代表已链接 release 可执行文件或通过真实 GPU 性能测试；`fast-check` 同理。当前实测状态见[原型验证记录](../../docs/plugins/shell-prototype-validation.md)。

## 本轮实测与 Windows 冒烟

2026-11-05：默认 dev 编译 / 链接、release 类型检查、Clippy 通过；常规测试 12 项通过，独立 watchdog 测试 1 项通过。真实 Windows 窗口已启动、底栏激活后显示左右视图，并以退出码 0 关闭。

**Fast 检查当前失败**：固定上游 `gpui-base` 三处无参数 `Modifiers::secondary()` 调用与 `gpui-fast 0.1.11` 的 `secondary(&App)` 不兼容。该错误及 `PluginManager` 模块注入、约 2 秒 UI 阻塞均记录为准入缺口。

构建后可重现 Windows 窗口检查：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/shell-prototype/smoke-windows.ps1
```

脚本会短暂显示本次启动的原型窗口，向该窗口底栏发送点击并关闭它，保存启动 / 激活截图与 `native-smoke.json` 到 `artifacts/`。截图需视觉核对；脚本不操作其他应用的窗口，也不代表 GPU 性能验收。

## 公共 API 验证范围

- 一个固定的 `conduit-probe` HostModule，身份由宿主闭包绑定；UI 状态通过 generation / contextVersion 校验。
- `load_application` / `mount_application` 在同一单插件 Policy 下创建两个不同入口视图。每个已挂载入口保有自己的状态和应用代次。
- 窗口结构为组件 / Base Root → ShellRoot → Rust Probe。宿主继续绘制功能入口和底栏，脚本只填充内容。
- 只在显式点击后创建 VM、求值代码。加载前关闭 editor files，夹具不含动态 dependencies。
- 只读和停止时拒绝新调用并释放实体，测试检查 VM 弱引用与异步续体。

这里使用单个固定默认 Policy 来验证一个受信任示例；没有轮换全局权限来模拟多个插件。`PluginManager::load` 不能注入宿主模块的问题由独立负例重现。底栏执行的是宿主已知命令，不证明可定向调用任意 JS handler。需要上游扩展的部分会保留为未通过准入条件。

## 构建条件

锁定的上游 workspace 使用 `rquickjs-compat` 门面让 Shell 与 LLRT 共享 QuickJS 类型。依赖 workspace 的 patch 不会被当前根工程继承，因此本工程显式使用同一提交的 `[patch.crates-io] rquickjs`。默认关闭 Shell 的 WebView feature，GPUI Fast 开关同时转发给 Kit 和 Shell。

首次自由解析时，`rsa 0.10.0-rc.19` / `pkcs1 0.8.0` 导致固定 LLRT 的 `llrt_crypto` 出现六处 E0282 / E0284。原型锁回上游 lock 使用的 `rsa 0.10.0-rc.18` / `pkcs1 0.8.0-rc.4` 后继续验证；请使用已提交的 Cargo.lock 和 `--locked`。本工程不修改 Cargo 缓存中的任何源码。

## 文件

| 文件 | 用途 |
| --- | --- |
| `src/main.rs` | Rust 宿主、脚本加载、三处布局与 HostModule |
| `src/model.rs` | 作用域、身份、只读、状态消息校验及行为测试 |
| `src/ui_tests.rs` | GPUI 集成探针与明确预期的缺口重现 |
| `fixtures/workspace-overview/` | 双视图、权限、清理和看门狗 JS 夹具 |
| `verify.py` | 可重复的构建 / 测试执行器、超时和日志 |
| `smoke-windows.ps1` | 原生窗口启动、指定窗口点击、截图与退出检查 |
| `Cargo.lock` | 实验依赖的固定解析结果 |
