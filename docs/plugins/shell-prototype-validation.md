# GPUI Shell 原型验证记录

验证日期：2026-11-05。工程：[tools/shell-prototype](../../tools/shell-prototype/README.md)；设计依据：[Shell 复用评估](gpui-shell-reuse-assessment.md)、[P2-A 候选设计](phase-2-shell-runtime.md)。

**原型已实现三处区域的联动与脚本生命周期验证，但 Shell 暂未通过生产插件后端准入。** 本文保留未打补丁的初版验证基线；后续逐插件加载和资源控制候选补丁、实测结果及新契约集中在 [G2/G5 进度](p2-host-controls-2026-11-05.md)。补丁验证与上游原始缺口须分别阅读：后续补丁已修复 Fast 原型全目标类型兼容，生产集成仍未完成。

G3 后续已补齐具体视图定向命令、有界异步返回和同 owner 多视图统一停止，见
[G3 实现与验证](p2-targeted-commands-2026-11-05.md)。下文各项仍记录初版基线，不覆盖后续补丁的结果。

清单绑定、会话快照和 P1 实例 / ticket 开发适配已完成，见 [开发加载器记录](p2-development-loader-2026-11-05.md)：61 项常规测试、6 项分别执行的故障探针通过，默认和开发窗口原生冒烟通过。后续已接入真实 ConduitApp，使用独立桌面验证二进制，构建、UI 和原生窗口结果见[桌面接入记录](p2-desktop-integration-2026-11-05.md)。正式发布依赖与生产 G4–G6 仍待验收。

## 初版原型范围（未打补丁）

后续 G6 的运行时浮层身份、停止通知、焦点恢复和桌面清理验证见[浮层清理记录](p2-overlay-cleanup-2026-11-05.md)。该记录接续上述桌面基线；下文仍保留初版结果，不代表当前候选补丁的缺口。

这是独立 Cargo workspace，以 Rust 宿主复现左侧图标及导航页、右侧摘要容器、中间内容底栏。正式 ConduitApp、主工程依赖和布局未接入该原型；测试使用合成工作区 A / B 与工程内受信任夹具，不读取真实聊天、凭据或 CLI 配置。

- 左侧 W 图标或底栏首次点击才创建 VM、加载脚本；发现清单不执行 JS。
- 一个宿主固定的 `conduit-probe` 模块提供摘要、打开右栏与提交状态。owner 由 Rust 闭包绑定，写调用校验 generation / contextVersion、可写状态与命令 ID。
- 两个公开入口 `load_application` / `mount_application` 挂载独立视图，分别保存脚本计数；切换工作区销毁后重建，不保留跨工作区计数。
- 底栏只执行已知宿主命令，脚本可以异步更新有界文本；这尚未实现任意 JS handler 的定向命令总线。
- 停用或转只读撤销实例；渲染异常保留上次有效快照，恢复后继续显示。
- 组件 / Base Root → ShellRoot → Probe 验证两类弹窗查找；1200px 与 850px 验证侧栏 / 覆盖层分支，底栏高度 34px。

只安装一个固定默认 Policy，整个窗口生命周期不轮换授权；不将它描述为多插件隔离方案。夹具关闭 storage、editor files，不包含动态 dependencies。尚未实现贡献注册表、ViewInstanceKey 缓存、图标菜单、底栏溢出、安装更新、持久化或真实 Conduit 布局回归。

## 初版环境与可重复构建

| 项目 | 本轮值 |
| --- | --- |
| 平台 | Windows x64，`x86_64-pc-windows-msvc` |
| Rust / Cargo | `rustc 1.97.1 (8bab26f4f 2026-07-14)` / `cargo 1.97.1` |
| Kit / Shell | `0.8.0`，Git rev `11345ec002d9f182a266b5d5dd3a217090700a87` |
| 默认 GPUI | `gpui-pre 0.3.9` |
| Fast GPUI | `gpui-fast 0.1.11` / macros `0.1.13`；检查失败详见下文 |
| QuickJS | `quickjs-jit 0.12.11`、`quickjs-jit-stdlib 0.12.7`，具体 Git rev 固定于实验 lockfile |
| LLRT | `0.9.0-beta`，rev `7b95c82a9b15e7ddfb2778eca4b5a63111e74f51` |
| 依赖边界 | 独立 Cargo.lock；根 Cargo.toml / Cargo.lock 未因原型改变；未修改 Cargo 缓存源码 |

上游 workspace 的 `rquickjs` patch 不会被依赖方自动继承，本工程显式 patch 到同一提交的兼容门面。首次自由解析得到 `rsa 0.10.0-rc.19` / `pkcs1 0.8.0`，使固定 LLRT 的 `llrt_crypto` 出现六处 E0282 / E0284。仅将实验锁文件恢复到上游使用的 `rsa 0.10.0-rc.18` / `pkcs1 0.8.0-rc.4` 后，默认构建通过。因此所有重现命令使用 `--locked`，不以刷新上游版本消除问题。

Shell 关闭默认 features，只启用 `quickjs` / `components`；实验 `gpui-fast` 同时转发给 Kit 和 Shell。独立工作区未复用主应用整个锁文件，不能将其检查成功等同于将依赖合入主应用也必然成功。

在仓库根目录执行：

```powershell
python tools/shell-prototype/verify.py --phase check
python tools/shell-prototype/verify.py --phase test
python tools/shell-prototype/verify.py --phase watchdog
python tools/shell-prototype/verify.py --phase fast-check
python tools/shell-prototype/verify.py --phase release-check
python tools/shell-prototype/verify.py --phase build
powershell -NoProfile -ExecutionPolicy Bypass -File tools/shell-prototype/smoke-windows.ps1
cargo run --locked --manifest-path tools/shell-prototype/Cargo.toml --target-dir target/shell-prototype
```

`verify.py` 保存命令、退出码、耗时及锁文件 SHA-256，日志位于原型 `artifacts/`，不纳入版本库。常规检查每条命令最长 30 分钟；死循环测试单独进程、最长 120 秒，超时只终止该次测试进程树。GPUI 测试串行执行。下列结果记录初版上游基线；当前 runner 会应用宿主控制与 Fast 兼容补丁，最新结果以 [本轮进度](p2-host-controls-2026-11-05.md) 及其补丁哈希为准。

## 初版实测结果（未打补丁）

| 检查 | 结果与边界 |
| --- | --- |
| rustfmt、默认 cargo check | 通过 |
| Clippy，all-targets + ui-tests，`-D warnings` | 通过 |
| 常规测试 | **12 passed，0 failed，1 ignored**；其中 5 项模型测试、7 项 GPUI 集成探针 |
| 独立死循环探针 | **1 passed**；无限 render 在 2002 ms 后中断，停止并重新激活后正常渲染 |
| 默认 dev 可执行程序 | 编译、链接通过 |
| Fast 后端 all-targets 类型检查 | **失败**，上游 gpui-base 三处 E0061，详情如下 |
| 默认 release 配置类型检查 | 通过；未链接 release 成品或做 release 性能验收 |
| 实际 Windows 窗口 | 创建成功；截图核对底栏激活后的左右脚本视图；WM_CLOSE 后进程正常退出，退出码 0 |

Fast 失败位置均在固定提交的上游 `crates/base/src/`：`input/editor/lsp/definitions.rs:146`、`input/editor/lsp/hover.rs:99`、`questionnaire/keyboard.rs:36`。它们调用无参数 `Modifiers::secondary()`，而解析出的 `gpui-fast 0.1.11` 要求 `secondary(&App)`。日志为 `artifacts/check-fast.log`；没有修改上游缓存或静默升级依赖来绕过。该失败使 G1 的双后端条件仍未满足。

普通测试覆盖以下行为：

| 证据 | 断言内容 |
| --- | --- |
| 模型测试 | 激活状态、owner / 命令校验、上下文与代次过期、只读状态、状态文本边界 |
| 三处交互 | 底栏懒激活，左页打开右栏，两页计数独立，A → B 内容与实例更新，两档宽度和底栏高度 |
| 快照与故障 | 干净帧不重复运行 JS render；异常保留有效快照；恢复重新渲染；只读停止并拒绝激活；VM 弱引用释放 |
| 根节点 | 组件弹窗与 Shell 弹窗均能从组合根结构找到宿主并关闭 |
| 异步结果 | 正常异步回调更新底栏实际按钮文本；切换工作区后旧状态不能污染新工作区；停用后待完成任务不保留 VM |
| 权限拒绝 | 实际 JS 的文件读取、Shell localStorage 访问被拒绝 |
| 应用清理 | 用公开 API 挂载的 root 销毁后，`owner: null` 异步任务不再回调；保持 root 存活的正对照能回调 |
| manager 缺口 | 发现不加载、拒绝授权不加载；批准后仍无法导入宿主模块，且 loaded 列表为空 |

GPUI 测试上下文执行真实脚本与组件实体，但不等价于真实 GPU 帧率或完整应用回归。日志中的 snapshot render 耗时仅是小夹具诊断样本，不用于推算 FPS、P95 或聊天流式输出延迟。当前弹窗检查没有证明生产设置页、tooltip、焦点切换和 Esc 的完整兼容。

Windows 冒烟脚本仅操作本次启动的进程和窗口，不发送全局鼠标键盘事件。它保存 `native-smoke.json`、初始及激活后截图到 `artifacts/`；本轮已实际查看两张截图，确认内容正确。截图文件创建本身并非图像断言；该脚本不替代上表的交互测试，也不证明安装 / 更新退出屏障。

## 初版准入结果（未打宿主控制补丁）

| 条件 | 本轮结论 | 已验证 / 剩余缺口 |
| --- | --- | --- |
| G1 构建与公开 API | **部分通过，Fast 阻塞** | 默认 dev 编译、链接和原生窗口可用，外部 crate 可使用公开加载 API；Fast 存在三处上游 API 不兼容；默认 release 类型检查通过，正式应用合并依赖与 release 链接仍未验证 |
| G2 授权与加载 | **阻塞** | 单个固定 Policy 能工作；`PluginManager::load` 批准后仍无法导入宿主模块，无逐插件 Policy / HostModule / 数据代次加载路径 |
| G3 多视图与命令 | 部分通过 | 两个入口视图独立、三处使用当前工作区；尚无同一逻辑插件统一应用生命周期、正式实例缓存与任意 JS 命令定向派发 |
| G4 根节点与浮层 | 部分通过 | 原型的组合根、两类弹窗、宽窄布局可用；正式 Conduit 设置、主题、焦点、tooltip、Esc 与窗口回归未完成 |
| G5 资源与响应 | **阻塞** | 渲染异常恢复和快照复用可用；无限 render 实际约 2 秒才中断，仍占用 UI 线程；锁定源码无默认 JS 堆上限、未找到公开宿主预算配置入口 |
| G6 发现与关闭 | 部分通过 | 静态发现、懒激活、VM 释放、root 任务取消、原型原生窗口退出可验证；卸载 / 更新事务与持久化写入屏障未实现 |

G2 的测试保留原始异常，核心为 `cannot resolve module`，目标模块为 `conduit-probe`。它验证了 manager 没有继承宿主配置模块，不能将此负例通过写成逐插件注入通过。生产实现需要显式扩展加载接口，不能临时切换默认 Policy 绕过。

G5 的无限循环在独立测试进程执行，不对无限堆分配做常规压力试验。源码中无默认堆上限是静态证据，不能替代真实资源耗尽后的可恢复性验证；外部测试超时也不是运行时对插件的保护。release / Fast 的 check 命令不生成可运行成品，不代表相应配置已链接或完成性能验收。

## 初版提出的实现边界

下列第 2、3 项已由后续宿主控制与 Fast 补丁推进，G3 定向命令与多视图停止也已完成原型验证；清单绑定、开发适配及生产验收顺序见 [G3 进度](p2-targeted-commands-2026-11-05.md)。

1. P1 可继续建设与后端无关的贡献注册表、命令校验与三处容器；保留原型作为 Shell 接缝测试。
2. P2-A 先形成公开加载 API 补丁：逐插件提供已裁剪 Policy、HostModule、稳定身份及数据代次；挂载与卸载时返回可追踪的实例句柄；增加多实例定向调用能力及互相隔离的测试。
3. 为 Shell 增加可配置堆限额、执行预算与可恢复中断，并在真实聊天交互中验收。只把 2 秒 watchdog 缩短，并不能自动证明所有原生调用都可中断。同时修复 Fast 后端兼容问题并重跑构建矩阵。
4. 上述缺口解决后再接入 ConduitApp，验证生产布局、退出、安装更新和存储屏障；若补丁成本不接受，再评审 P2-B。原型不提供普通第三方脚本安装，也不据此下调原阶段预算。
