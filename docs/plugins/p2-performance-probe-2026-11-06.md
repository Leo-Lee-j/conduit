# P2-A：G5 桌面性能测量入口

2026-11-06 后续：[release 界面回归适配](p2-release-ui-regression-2026-11-06.md)已解决下文记录的 18 项布局回归失败，dev / release 各通过 45 项常规测试。本文采样数据与当时失败记录保留历史含义；本次语义定位修复的源码摘要和验证结果另见后续记录。

日期：2026-11-06。承接 [G4 输入、焦点与主题](p2-input-focus-2026-11-05.md)。本轮增加可重复的桌面 CPU / 布局与进程内存探针；这是 G5 的自动化测量基础，尚不构成生产性能准入。

## 本轮范围

[桌面探针](../../src/app/plugin_ui/plugin_shell/tests/performance.rs)运行实际的组件 Root → ShellRoot → ConduitApp 组合，复用现有开发插件和合成数据：两个工作区，每个工作区一个会话，窗口为 1600 × 800。使用 GPUI 测试平台，计时为真实墙钟时间，窗口没有原生 GPU 呈现。

| 场景 | 状态 | VM / 缓存视图数 |
| --- | --- | --- |
| baseline | ShellRoot 中的宿主，无开发插件 | 0 / 0 |
| registered | 已注册开发包，未打开入口 | 0 / 0 |
| navigation | 打开插件导航视图 | 1 / 1 |
| dual | 同一插件的导航和摘要均打开 | 1 / 2 |
| hidden | 打开两视图后隐藏，保留缓存 | 1 / 2 |
| stopped | 打开两视图后停止，检查 VM 弱引用已释放 | 0 / 0 |

每种状态均测两项：宿主输入框全选并交替替换两段等长的中文 / emoji 文本，随后绘制；宿主通知更新，随后绘制。dual 另外测插件定向命令、结果处理和绘制。普通绘制使用缓存的脚本描述，命令更新会触发脚本视图刷新；这些是组合成本，尚未拆分 JS 描述、原生组件物化和同步 HostModule 耗时。

编辑样本包含全选、整段文本的模拟输入和测试调度，不是一次键盘事件到屏幕呈现的延迟。插件导航 / 右侧面板也会改变宿主布局，因此相对 baseline 的差值同时包含布局差异，不能视为纯脚本执行开销。

输入文本交替变化，避免将相同内容替换的短路路径当作编辑性能。每项预热 20 次，再取 200 个样本；每次动作前等待 16ms，等待时间不计入样本，Windows 实际等待可能更长。这不推进 GPUI 模拟时钟，不能证明真实窗口定时器或隐藏视图没有周期重绘。

进程内存复用宿主的 `PerformanceMonitor`，记录 fixture 前、静态注册后、激活后、待采样及采样后五个阶段的工作集、私有提交内存和进程生命周期工作集峰值。它包含 VM、Rust、测试平台及分配器缓存，不能归属为单个原生组件或 JS 堆内存；停止后 VM 释放不保证分配器立刻把内存交还系统。

## 可重现与结果校验

[测量 runner](../../tools/shell-prototype/desktop_performance.py)先编译一次，从 Cargo JSON 产物消息中定位测试二进制，并复制到本轮独立目录；所有场景执行同一份副本。每次重复使用新进程、独立数据及插件快照目录，场景顺序按轮次轮转。原始耗时数组和内存样本保留在 `artifacts/desktop-performance/<run>/`，汇总保存在 `artifacts/desktop-performance-results.json`。

```powershell
python tools/shell-prototype/verify-desktop.py --phase performance
python tools/shell-prototype/verify-desktop.py --phase performance --profile release
python -m unittest discover -s tools/shell-prototype -p test_desktop_performance.py -v
```

默认 dev、每项 200 样本、20 次预热、每种状态 3 个进程。`--samples`、`--warmup`、`--repeats` 可调整，`--target-dir` 可复用构建缓存。dev 与 release 测试平台测量均已执行，结果分别记录如下；release 测试二进制不代表原生应用成品验收。

每个进程按 nearest-rank 计算 p50 / p95 / 最大值，再报告三轮 p95 的中位数和范围，不把不同进程的样本混成一个 p95。相对 baseline 展示 `max(2ms, 基线 p95 的 10%)` 的参考比较，但 `production_gate` 固定为 `not_evaluated`；这是 [P3 发布目标](phase-3-management.md)的参照，不是该目标已通过。

空测试筛选、缺失 / 不完整报告、错误场景或 profile、NaN / 无限值、错误 VM / 实例数、失败的内存采样都会使 runner 失败。编译和测量前后校验桌面源码及 runner 摘要；若源码变化，退出失败并标记 `valid_for_source_snapshot: false`。报告同时保留上游、补丁、lockfile 和测试二进制摘要。

## 本轮验证

首次在独立工作树完成 18 个测试进程、7,800 个计时样本，runner 退出 0，`valid_for_source_snapshot: true`，`source_changed_during_checks: false`。该次测量 profile 为 dev（主工程 `opt-level = 1`），Rust 1.95.0 / x86_64-pc-windows-msvc，Windows 11 build 26200，14 个逻辑 CPU。整机后台任务负载未受控制；后续 release 数据见下节。

下表耗时单位为 ms，均为三轮 p95 的中位数，括号为三轮 p95 的最小 / 最大值；内存为采样前 ready 阶段的三轮中位数，单位 MiB。

| 场景 | 整段替换及绘制 p95 | 通知及绘制 p95 | 私有提交 | 工作集 |
| --- | --- | --- | --- | --- |
| baseline | 29.904（28.175–33.943） | 3.497（3.322–3.605） | 13.812 | 36.180 |
| registered | 38.328（29.803–65.737） | 3.849（3.351–4.831） | 13.797 | 36.738 |
| navigation | 36.316（30.008–111.506） | 8.338（3.174–13.040） | 16.477 | 43.301 |
| dual | 48.154（44.636–52.660） | 3.925（3.654–4.542） | 16.879 | 44.738 |
| hidden | 96.604（30.143–145.342） | 4.445（3.590–19.981） | 18.363 | 45.305 |
| stopped | 34.959（32.796–45.848） | 3.387（3.146–4.257） | 18.719 | 46.707 |

dual 的插件命令及绘制 p95 中位数为 11.508ms，三轮范围 6.441–23.138ms。五种非 baseline 状态的整段编辑增量均超过参考预算；navigation 的通知及绘制增量也超过参考预算。轮次差异明显，尤其是隐藏场景，尚不能确定脚本、宿主布局、测试调度和整机负载各自的贡献。**本轮结果不支持宣布性能达标**，也不把超出部分全部归因于插件运行时。

停止场景已通过 VM 弱引用释放断言，但 ready 私有内存仍高于从未激活的基线。这只能说明本轮进程保留了更多分配，不能单次判定泄漏；后续应补多轮激活 / 停止后的增长曲线。

可重现摘要：

```text
base_commit          a121358
source_sha256        a921680fbe4b065b9863a51d415051b94fdb96195c40fc1948f1d93fad1c3f89
lock_sha256          8f772a19257b558d23e9f5b6de01d9042003f1834b8ce2cc2b31bf0cd2fc3034
executable_sha256    d111a566b9216c9003616f6d0990596cd830ecaffa3a3849a13c89c71588569d
upstream_sha256      e8d19bbb0faaa950cdb8f3c7dde5938bb3c06e9b2f3c43576d6b731b05e72171
patch_sha256         87b1301f6f2387fddd818df8af39c33d0f8a7012e7de53a57a6bde9baba35181
```

原始结果位于独立工作树的 `tools/shell-prototype/artifacts/desktop-performance/20261105T170425Z-4erray5i/`，汇总为同级 `desktop-performance-results.json`。本轮没有升级依赖；独立 Windows checkout 的 lockfile 换行与主工作树不同，故字节摘要不同。上游补丁已按主工作树原始字节复制并通过固定摘要校验，未修改生成源码。

dev 汇总另存为 `artifacts/g5-dev-performance-results.json`，后续 release 运行可以更新默认结果文件而不会覆盖此记录。实验 lockfile 内容与基线提交一致，换行导致上述字节摘要差异；主工作树根 manifest / lockfile 随其他任务继续变化，因此本轮独立验证结果只覆盖固定基线及本轮改动。

| 定向验证 | 结果 |
| --- | --- |
| 报告校验单元测试 | 4 项通过，覆盖分位数、无效样本、缺失结果、构建产物、profile / 实例数与参考比较 |
| 桌面插件回归 | 42 项通过；性能探针默认 ignored，另由测量 runner 显式执行 |
| 桌面更新回归 | 3 项通过；两组桌面常规回归合计 45 项 |
| 桌面相关文件格式 | 通过，包含新增性能探针 |
| 桌面 all-targets + ui-tests check | 通过 |
| 桌面 all-targets + ui-tests Clippy | `-D warnings` 通过 |

首次 check / test / dev performance 三份报告的源码摘要均为 `a921680fbe4b065b9863a51d415051b94fdb96195c40fc1948f1d93fad1c3f89`，各自检查前后一致。上述结果不包括共享工作树中其他任务的未完成修改。后续仅加强探针状态断言，新的源码摘要与 release 结果另行记录。

本轮起始在干净的 0.1.9 工作树复核，桌面格式、all-targets + ui-tests check 和 Clippy `-D warnings` 均通过，昨天记录的 7 项 Clippy 阻塞已消除。其后其他任务继续修改 CLI / 模型管理源码，所以新增探针改用 `target/g5-validation-20261106` 的独立工作树验证：基于提交 `a121358`，只带入本轮测试与 runner 改动，不合入其他任务的未完成修改。

两次早期试跑不计入有效性能结果：小样本六场景均执行成功，但源码变化被校验拒绝；第二次在旧共享临时快照目录遇到访问错误，并同时检测到源码变化。随后插件测试快照改为数据目录下按进程分开，测量本身仍使用每轮独立数据目录。

## release 测试边界与探针修复

后续以相同固定基线、独立 target `target/shell-prototype-g5-release` 编译 release 测试二进制，首次编译链接成功，耗时 972.586 秒，完成六种状态、三轮共 7,800 个样本。原始记录保留为 `artifacts/g5-release-initial-performance-results.json`；它是加强隐藏状态断言之前的结果，最终数据以随后重测为准。

对该二进制执行常规回归，插件组为 24 passed / 18 failed / 1 ignored，更新组为 3 passed / 0 failed，报告见 `artifacts/g5-release-regression-results.json`。18 项失败发生在测试查找调试布局标记时；锁定的 `gpui-pre 0.3.9` 中，`InteractiveElement::debug_selector` 在无 `debug_assertions` 的依赖构建中为空操作，`VisualTestContext::debug_bounds` 明确返回 `None`。这些 dev 布局测试尚未适配 release，不能将 release 常规回归写为全通过，也不能据此断言组件实际消失。

因此修复了本轮性能探针中的同类问题：隐藏场景原先只断言 `debug_bounds("detail-panel").is_none()`，在 release 中没有验证效力；现在直接检查宿主导航选择与右侧面板打开状态，并核对六种场景与预期可见状态对应。VM / 缓存视图数、命令计数、输入与焦点、停止弱引用校验继续保留，未开启 debug assertions，也未更改性能构建配置或生产代码。

修复后再次完成 release 编译链接（178.837 秒）和全部 18 个进程、7,800 个样本，runner 退出 0，源码前后一致且 `valid_for_source_snapshot: true`。采样参数、平台、数据集和硬件环境与 dev 记录相同；依然没有原生窗口或 GPU 呈现。下表使用相同的 p95 中位数 / 范围与 ready 内存口径：

| 场景 | 整段替换及绘制 p95（ms） | 通知及绘制 p95（ms） | 私有提交（MiB） | 工作集（MiB） |
| --- | --- | --- | --- | --- |
| baseline | 44.493（39.667–46.636） | 4.346（2.731–7.459） | 13.758 | 34.051 |
| registered | 44.319（38.349–50.601） | 4.417（3.759–4.679） | 13.867 | 34.738 |
| navigation | 36.421（30.615–142.626） | 3.647（2.768–6.393） | 16.531 | 40.973 |
| dual | 37.005（35.300–52.056） | 4.691（3.558–4.713） | 16.750 | 42.133 |
| hidden | 41.556（32.775–45.399） | 4.099（3.794–5.193） | 18.438 | 42.887 |
| stopped | 42.814（31.723–52.196） | 3.926（2.738–4.885） | 18.633 | 44.145 |

插件命令及绘制 p95 中位数为 **7.576ms**，三轮范围 7.372–7.660ms。按中位数计算的场景增量都在参考预算内，但 navigation 单轮编辑 p95 达到 142.626ms，基线在 dev / release 间也明显变化。当前负载与测试调度尚未充分控制，不能把负增量解释成“插件让宿主更快”，也不能把参考比较为 true 等同生产性能通过；`production_gate` 仍为 `not_evaluated`。

最终 release 源码 SHA-256 为 `5154e8560d209be36bbb1c232f38c562cf3e1f6ba757c5767f324ea7e978a6d5`，二进制 SHA-256 为 `2724985cbb169d28a464182f9c0a34cdd50d2cc4b9cd42fe49fb6feffa7aec50`。lockfile 和上游摘要未变，最终格式、all-targets + ui-tests check 与 Clippy `-D warnings` 也在该源码摘要下通过。

最终 release 原始目录为 `artifacts/desktop-performance/20261106T021016Z-w_qojq6_/`，归档报告为 `artifacts/g5-release-performance-results.json`。dev / 最终 release 汇总、原始 JSON / 日志及 release 常规回归失败记录均复制到主工作树的 `tools/shell-prototype/artifacts/` 便于查看；原始二进制与完整验证工作树继续保留在 `target/g5-validation-20261106`。最终报告不覆盖后来共享源码的变更。

## 未完成与下一步

2026-11-06 重试 Windows Computer Use，仍返回 `Computer Use native pipe is unavailable`，没有执行真实输入法操作或取得新原生截图。G4 的 Windows IME 候选框、组合输入事件和原生视觉验收仍未完成。

本轮不覆盖 release / Fast 原生应用成品运行、真实 OS 输入到 GPU 呈现的交互及滚动 p95、代表性长会话、20 个未激活包、3 个活动插件、异常与正常插件并存、脚本 / 物化 / HostModule 分项计时和可归属资源。release 布局回归已在后续改用稳定的语义定位并通过；下一步扩展测量数据与负载、分解耗时和重复激活 / 停止后的内存增长，在原生连接可用时完成 G4 及 release 原生性能验收。只有完整准入通过后才冻结 P2 生产契约并进入 P3 安装管理。

## 后续 stress 扩展

2026-11-06 新增独立 `--phase stress`，固定长会话、20 个静态包、3 个活动 VM、故障隔离与重复启停/空闲内存采样见 [G5 stress 记录](p2-stress-probe-2026-11-06.md)。上文六场景数据及当时的未完成范围保留历史含义；当前剩余项以新记录为准。
