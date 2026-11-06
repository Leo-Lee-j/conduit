# P2-A G5：命令重复绘制与截止时间诊断

2026-11-06 后续：[独立重跑](p2-g5-rerun-2026-11-06.md)确认两种构建所有正式命令的收尾绘制及物化仍为 0，并完成 stress / breakdown 复测；历史普通路径超时未复现，原因仍未定位。本文保留原实现轮次及前后对照数据。

日期：2026-11-06。承接 [breakdown 测量](p2-breakdown-probe-2026-11-06.md)。本轮修复成功命令收尾时无条件通知宿主重绘的问题，新增 `--phase diagnostics`。上轮普通路径的截止时间故障仍未定位，G5 生产准入不变。

## 实现与计数口径

`dispatch_shell_command` 仍接收结果、校验上下文并清理任务，但仅在错误状态变化时通知宿主。成功结果目前只保存在内部 `shell.result`，没有可见结果控件；脚本视图状态变化仍由脚本的 `cx.notify()` 驱动。命令分发时挂载容器、切换视图和错误反馈所需的通知保留。

新增三项常规回归：测试根计数器覆盖自动与显式 draw；成功命令不增加收尾绘制且结果/快照已更新；命令错误通知绘制，后续成功清除错误。

诊断使用相同 `long-chat-v1`、20 个注册包和 3 个驻留 VM。每个独立进程先执行聊天行同步、激活前宿主绘制、三个视图首次打开和缓存绘制，再执行 local / metadata 命令。两个驱动按轮次交替顺序运行：

| 驱动 | 每条命令 |
| --- | --- |
| `explicit-draw` | update 分发与自动绘制 → `run_until_parked` → 显式 `window.draw` |
| `effects-only` | update 分发与自动绘制 → `run_until_parked` |

每条命令完成后，在任何额外 draw 之前确认任务为空、结果已接收、无错误、快照中的命令计数递增。`effects-only` 是新诊断入口中的有效驱动；历史 breakdown 的显式 draw 指标与报告保留原义。

测试窗口中加入不缓存的根包装实体，计数真实根 `Render::render` 入口，并用常规测试校验 notify 和显式 draw 各增加一次，纯读取不增加。这里的 root render 表示 GPUI 测试窗口绘制遍历，不是 GPU 呈现帧。每阶段同时读取现有公开 `read_metrics()`：脚本 render 次数/总耗时、物化次数/总耗时、frame script 调用次数、HostModule 调用总耗时。原生物化和脚本 render 不等同整个窗口绘制。

## 故障证据

`patches/shell-resource-diagnostics.patch` 通过可审核补丁流程增加 `resource_diagnostic()`，只捕获第一次资源故障的入口类型、墙钟耗时、预算、原生描述调用计数、桥接值/字节计数、累计用时和检查位置。嵌套调用不覆盖外层入口，随后 stop 保留首次故障。`resource_error()` 继续返回简短错误供界面展示。没有修改预算，也没有编辑 Cargo 缓存。

默认单次 render / event / module 仍为 25 / 50 / 100ms。入口类型用于区分 render、layout、event、task、module、detached 和 materialize；`native_calls` 包括原生描述 builder 调用，不是纯 HostModule 调用数。全部时间为墙钟时间，没有线程 CPU 时间或调度跟踪，不能独自区分计算、等待或抢占。

逐行 JSONL 记录命令、样本编号、阶段 begin/end、耗时和计数。写文件发生在计时区间之外，但会改变整体负载，因此本轮是诊断，不能与历史 benchmark 直接作性能准入比较。即使进程失败或超时，已刷新的记录仍保存；Python runner 保留首次错误和最后记录，失败整轮不发布成功汇总。强制终止可能留下不完整的最后一行，读取器保留此前完整记录并拒绝验收。

为在故障回收后读取数据，诊断探针持有三个 runtime 的强引用；它不在故障后尝试恢复或继续测量，不用于证明回收或内存曲线。资源回收继续使用既有 lifecycle 测试。

## 实测与验证

使用 Rust 1.95.0 / LLVM 22.1.2、Windows 11 build 26200、14 逻辑 CPU，未控制其他应用负载。隔离 Conduit 基线仍为 0.1.9；主工作树的版本与并行改动不在采样范围。

修复前与修复后 dev 各 6 个进程，每种驱动 3 进程，每条命令每进程 10 warmup / 60 正式样本；每组 840 次命令含 120 次预热。两组完整报告均有效，普通路径没有复现截止时间错误。下表是所有正式命令样本的实际计数，local / metadata 一致：

| dev 驱动 / 版本 | 每命令根 render | 每命令脚本 render | 每命令物化 | settle 根 render / 物化 |
| --- | ---: | ---: | ---: | ---: |
| 修复前 explicit-draw | 3 | 1 | 3 | 1 / 1 |
| 修复前 effects-only | 2 | 1 | 2 | 1 / 1 |
| 修复后 explicit-draw | 2 | 1 | 2 | 0 / 0 |
| 修复后 effects-only | 1 | 1 | 1 | 0 / 0 |

结果收尾的进程内 p95，再取三个进程的中位数（最小–最大），单位 ms：

| 命令 / 驱动 | 修复前 dev | 修复后 dev |
| --- | --- | --- |
| local / explicit-draw | 21.233（20.841–27.956） | 0.060（0.059–0.062） |
| metadata / explicit-draw | 21.907（19.306–27.203） | 0.069（0.060–0.069） |
| local / effects-only | 20.576（20.283–34.062） | 0.064（0.058–0.067） |
| metadata / effects-only | 24.191（20.061–25.400） | 0.066（0.064–0.067） |

一次成功收尾的冗余通知已经消除；历史显式 draw 仍产生额外根绘制和物化，这是测试驱动成本。脚本 render 每条命令只有一次，不能把三次宿主绘制解释为脚本执行三次。以上为测试平台和诊断包装下的观测，前后并非同一二进制，诊断错误详情也由字符串扩展改为独立接口；时间不作为生产收益承诺，修复依据是实际计数与回归。不能相加分项 p95 得到总 p95。

修复前原始目录：`desktop-diagnostics/20261106T110206Z-uybpckfa/`；修复后 dev：`desktop-diagnostics/20261106T111744Z-2j0m29we/`。修复前源码摘要 `edc768737a93ec3037c73139452b46b35f1a903e0f3b5e5bcff2b2d74bfde611`；修复后为 `909d3f6bdf60f48ef0ece45662787968c96db6719cb439c1eb0aba86cdd2d118`。生成包均为 `b613ec29b754283e235b1c0f7bcd1632fb7f55c7250c68d2aaad441b1724e5f3`，沿用 breakdown 夹具。

独立的真实故障注入将生成包 `stress.plugin-02` 的 local 命令改为死循环，不改预算。第一个 warmup 命令在 `dispatch-effects` 阶段失败，exit 101；JSONL 保留了首次故障：`entry=event elapsed_ms=50.009 limit_ms=50.000 native_calls=0 bridge_values=1`。剩余两个 runtime 仍在，失败 runtime 的诊断可读取；validator 拒绝该记录，`valid_as_measurement: false`。其独立记录为 `g5-diagnostics-20261106/intentional-event-deadline/expected-failure-results.json`，含二进制及修改后包文件摘要。它验证失败证据链，不是上轮偶发故障的复现。

本轮三项隔离资源测试均通过；其中有界原生调用阻塞 30ms、render 限额 10ms 的用例观测总边界约 32ms，后续 HostModule 调用被拒绝，诊断标识 render，stop 后首次详情不变。它证明返回后检查仍有效，不代表可以抢占原生调用。

修复后 release 同样完成 6 个进程、840 次命令（含 120 次预热），所有正式样本的根 render / 脚本 render / 物化次数与修复后 dev 完全一致，收尾均为 0 次绘制及物化。local / metadata 的 settle p95 中位数分别为 0.056 / 0.060ms（explicit-draw）及 0.071 / 0.065ms（effects-only）。原始目录为 `desktop-diagnostics/20261106T111933Z-qnndbtci/`，报告有效，普通路径无截止时间故障；这不撤销上轮失败。

| 检查 | 结果 |
| --- | --- |
| Rust 1.95.0 格式、桌面 all-targets + ui-tests check、Clippy `-D warnings` | 通过 |
| 原型 all-targets + ui-tests Clippy `-D warnings` | 通过 |
| dev / release 常规回归 | 各插件 45 passed、4 按需探针 ignored；更新 3 passed |
| 隔离资源故障探针 | 3 passed（有界资源/恢复、累计预算、原生阻塞后检查） |
| Python runner / 夹具 / 校验 | 23 passed |
| 普通诊断采样 | 修复前 dev、修复后 dev、修复后 release 各 6 进程完整有效 |
| 人为 event 截止时间故障 | 预期 exit 101，首故障证据捕获，测量校验拒绝 |
| 历史证据 | 1,076 个非二进制文件 SHA-256 未变 |

修复后 dev 二进制 SHA-256：`eecb3c4bfcb51733e3c39799508be152ffb989b3fa43aa7a22b3ccbd3635ee77`；release：`1eab81ab43444d0bcf7c11dd3d7ef51d88f7cb2a69966c9b558b7b92726b5ff7`。检查、dev/release 测量与资源测试的源码摘要相同，期间未发生源码变化。候选补丁集合摘要为 `2adb18e3cedf1c50dc10be20262683f18207fd05724f756d0e4fdf3e34371f2b`，生成上游源码摘要为 `a366593fbca9ee2b08abd4c81dbda79403344def2a1b554ea8bba990c57763ff`。

隔离实现过程中的编译错误和同步抛错夹具失败也留有日志；最终错误回归使用 Promise 拒绝以覆盖此次修改的异步收尾路径。只向主工作树集成此任务 9 个源码/工具文件与 5 份文档，未提交或推送。主工作树的并行功能未纳入全量编译验证。

## 重现与证据目录

在隔离工作树 `target/g5-validation-20261106` 中：

```powershell
$env:CARGO_HOME = 'D:/enviroments/cargo'
$env:RUSTUP_HOME = 'D:/enviroments/rustup'
$env:CARGO_NET_GIT_FETCH_WITH_CLI = 'true'
python -m unittest discover -s tools/shell-prototype -p 'test_desktop*.py' -v
python tools/shell-prototype/verify-desktop.py --phase check --target-dir D:/m/conduit/target/shell-prototype
python tools/shell-prototype/verify-desktop.py --phase test --target-dir D:/m/conduit/target/shell-prototype
python tools/shell-prototype/verify-desktop.py --phase diagnostics --samples 60 --warmup 10 --repeats 3 --target-dir D:/m/conduit/target/shell-prototype
python tools/shell-prototype/verify-desktop.py --phase diagnostics --profile release --samples 60 --warmup 10 --repeats 3 --target-dir D:/m/conduit/target/shell-prototype-g5-release
```

每轮有唯一 `artifacts/desktop-diagnostics/<时间戳-随机后缀>/`，内含 JSONL、进程日志、生成包和带源码/二进制/上游摘要的 `run-results.json`。顶层 `desktop-diagnostics-{profile}-results.json` 只是最近结果；独立目录不会被覆盖。本轮汇总归档位于 `artifacts/g5-diagnostics-20261106/`，历史 performance / stress / breakdown 另按哈希校验。

补丁更新后，已有 `.upstream/.prepared.json` 会与新补丁摘要不一致。必须先通过 `prepare_upstream.py --destination <全新目录>` 重建并校验，再保留旧目录并切换到新生成副本；不要修改生成源码或手工更新摘要。本轮验证在隔离工作树进行，主工作树并行功能与版本变化不属于验证快照。

## 后续

继续使用新故障记录定位正常负载下的截止时间问题；未复现不能宣称修复。接着细分长会话 Markdown、布局、缓存和宿主绘制成本，结合原生 GPU / Windows IME / Fast 成品验收再评估 G5。当前不冻结 P2 生产契约，也不进入 P3 安装管理。
