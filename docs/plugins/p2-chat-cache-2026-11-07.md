# P2-A G5：聊天缓存与 Shell 宿主边界

开始：2026-11-06，最终版本的正式矩阵跨至 2026-11-07（Asia/Shanghai）。承接 [输入与 Markdown 分项](p2-chat-profile-2026-11-06.md)，目标是减少输入时未变化聊天内容的重复排版与绘制。

## 实现与失效边界

- `ChatTimeline` 使用 GPUI 缓存。每次父视图渲染比较消息版本、会话元数据、模型目录、字体/颜色、复制与高亮、展开状态、权限/运行状态、滚动位置；未变化时复用已绘制内容。
- `Rows` 在已有消息身份比较中维护版本号。内容替换即使行数、类型和 key 相同也会失效；工具分析完成的重新测量也会失效。缓存不保存消息文本副本。
- 正在运行的会话保留刷新行为；尺寸和继承文字样式由 GPUI 自身的缓存条件处理。用户文本拖选显式通知列表，已有三种拖选测试继续验证实际选择高亮。
- 底部跟随在列表真正渲染时执行，避免父视图重置 ListState 的末尾哨兵后又复用旧绘制。性能面板保留缓存帧的挂载行数，实际绘制或隐藏列表时才清空。
- `shell-host-content-cache.patch` 增加 opt-in `ShellRoot::with_uncached_content()`。Conduit 的 Shell 入口和夹具都启用它。GPUI 在重建缓存的父视图时会刷新后代；原先 ShellRoot 对整个 ConduitApp 的缓存使内层聊天缓存失效。默认脚本根的行为保持原样。代价是 Shell 浮层单独刷新时也会重新执行宿主根布局；下层聊天/导航等继续独立缓存，本轮未对浮层动画单独做帧预算验收。

## 测量有效性

分项探针升级 schema 2：计时外强制一次 Markdown 绘制并验证所有分项计数非零；随后允许缓存帧计数为零，每轮验证末条聊天与视口相交、输入值/焦点及 VM/实例数量。几何观测使用公开 `test_support` 的透明包装，编译范围仅限 Shell 桌面 UI 测试，普通应用不增加此状态。旧 stress 的驱动、数据、统计和预算未修改。

既有 `debug_selector` / `debug_bounds` 在 release 中不记录几何信息。因此完整 100 项几何/交互回归在 dev 运行；release 使用不依赖调试选择器的插件、更新及性能监控测试，其中新增 ShellRoot 缓存回归也验证真实输入与消息边界。误将全部几何测试送入 release 的 24 项查询失败原始日志独立归档，不作为应用绘制故障或通过结果。

固定 `long-chat-v1`（2 工作区 ×24 会话 ×120 消息；5,760 条、9,504,000 文本字节）、1600×800、60 正式样本、10 预热、3 独立进程。每进程字符 p95 池化 840 次正式字符输入，再报告 3 个 p95 的中位数。不能将分项 p95 相加当作旧 stress 的完整编辑耗时。

前后都使用 Rust 1.97.1 / LLVM 22.1.6。比较基准来自上一轮保留的二进制与原始轨迹；不是随机交错 A/B，后台负载未固定。active 打开右侧栏，场景差值包含聊天宽度和宿主布局变化。平台为 GPUI test，无真实 GPU 呈现；runner 的 `production_gate` 仍是 `not_evaluated`。

## 结果

下表为每进程 p95 的三进程中位数；单位 ms。前值来自 2026-11-06 上一轮，后值来自本轮最终源码。

### 单字符输入

| 构建 | 场景 | 优化前 | 优化后 | 观察到的下降 |
| --- | --- | ---: | ---: | ---: |
| dev | baseline | 17.323 | 3.327 | 80.8% |
| dev | registered | 23.112 | 4.750 | 79.4% |
| dev | active | 18.743 | 6.367 | 66.0% |
| release | baseline | 15.809 | 2.844 | 82.0% |
| release | registered | 16.886 | 3.123 | 81.5% |
| release | active | 19.011 | 4.484 | 76.4% |

正式字符样本共 **15,120** 个，其中聊天行或 Markdown 阶段计数非零的样本 **0** 个。缓存帧跳过这些元素阶段，不代表 CPU/GPU 整帧成本为零。所有进程在计时外通过了非零绘制校准。

### 原 stress 驱动

| 构建 | 场景 | 编辑+draw 前→后 | notify+draw 前→后 |
| --- | --- | ---: | ---: |
| dev | baseline | 241.975 → 43.452 | 32.245 → 7.108 |
| dev | registered | 344.586 → 51.100 | 38.439 → 8.440 |
| dev | active | 417.528 → 73.417 | 56.056 → 12.988 |
| dev | mixed | 361.307 → 79.573 | 50.541 → 12.112 |
| dev | lifecycle | 326.588 → 55.693 | 40.212 → 9.075 |
| dev | idle | 330.080 → 56.762 | 42.295 → 8.336 |
| release | baseline | 204.077 → 42.704 | 31.562 → 7.460 |
| release | registered | 230.351 → 46.172 | 30.934 → 7.499 |
| release | active | 268.165 → 69.592 | 32.212 → 9.829 |
| release | mixed | 259.543 → 64.442 | 31.351 → 9.986 |
| release | lifecycle | 215.233 → 49.773 | 28.820 → 7.341 |
| release | idle | 210.729 → 49.891 | 40.111 → 7.622 |

### release 参考预算

参考增量预算仍为 `max(2ms, baseline p95 × 10%)`，未调整。

| 场景 | 编辑增量 / 预算 | notify 增量 / 预算 |
| --- | ---: | ---: |
| registered | 3.468 / 4.270（内） | 0.039 / 2.000（内） |
| active | 26.887 / 4.270（超出） | 2.370 / 2.000（超出） |
| mixed | 21.737 / 4.270（超出） | 2.527 / 2.000（超出） |

**G5 生产准入仍未通过。** 上述 CPU 测试参考预算不能替代真实 GPU、Windows IME、滚动与成品验收；普通路径历史超时仍未归因。本轮不改变 P2 冻结或 P3 状态。

### 生命周期内存

下表是每个进程第 20 个 settled 点减第 1 个点的三进程中位数（MiB），不是峰值，也不单独证明泄漏或无泄漏。

| 构建 | 场景 | working set 增量 | private commit 增量 |
| --- | --- | ---: | ---: |
| dev | lifecycle | 2.027 | 2.777 |
| dev | idle | 0.398 | 0.398 |
| release | lifecycle | 2.730 | 3.625 |
| release | idle | 0.328 | 0.328 |

两种构建共 120 次启停循环和 120 次 idle 对照；启停每轮验证 3 个 VM 释放。完整 240 点曲线保存在 `lifecycle-memory.csv`；仍需更长窗口及 native 分配归属分析。

## 验证与证据

- dev 桌面回归：100 通过，5 个 opt-in 测量探针未在普通测试组执行。
- release 桌面回归：58 通过，5 个 opt-in 测量探针未在普通测试组执行。
- 默认 Shell 原型测试：67 通过；三项资源探针分别执行 1 个测试并通过。
- Python：28 通过；格式、全目标/ui-tests 类型检查及 Clippy `-D warnings` 通过。
- dev/release 各 9 个 chat-profile +18 个 stress 进程，合计 **54 个正式测量进程**通过且源码稳定。diagnostics 不在本轮矩阵内。
- 主工作区集成后 `cargo check --locked --all-targets --features ui-tests --jobs 2` 通过；其中包含原有并行 Git 功能修改。

正式矩阵：2026-11-07T00:18:18+08:00 至 2026-11-07T00:29:46+08:00。

| 身份 | SHA-256 |
| --- | --- |
| 最终隔离源码 | `e8af652e7df0dcb46f089619dd428e0cccedff44a378b9246031905ef322e5ef` |
| chat-dev 二进制 | `a23329636673b3afab7d985d5c469057b6b2561c6fdc4fb51c9f6b6a7c5365e8` |
| chat-release 二进制 | `53743f01a681a60a4006f2146b5138acb33456e2731ae518c2d4920d45493b15` |
| stress-dev 二进制 | `a23329636673b3afab7d985d5c469057b6b2561c6fdc4fb51c9f6b6a7c5365e8` |
| stress-release 二进制 | `53743f01a681a60a4006f2146b5138acb33456e2731ae518c2d4920d45493b15` |
| 准备后的上游源码 | `9e01a893f91b4ed846a6cca10b3b377921e80bf685b89ce8af0c4daabf0cc433` |

归档：`tools/shell-prototype/artifacts/g5-timeline-cache-20261106T233624/`。包含报告、原始轨迹/包/日志、`comparison.json`、`chat-summary.json`、内存曲线、源码快照、增量 `optimization.patch` 和校验清单。主工作区保存非二进制副本；二进制保留在隔离工作树的原始测量目录。归档脚本校验本工作区原有 789 个非二进制证据文件未变化；不外推到其他机器的历史文件。

复现：

```powershell
python tools/shell-prototype/verify-desktop.py --phase check
python tools/shell-prototype/verify-desktop.py --phase chat-profile --profile release --samples 60 --warmup 10 --repeats 3
python tools/shell-prototype/verify-desktop.py --phase stress --profile release --samples 60 --warmup 10 --repeats 3 --cycles 20
```

完整阶段、过滤器及参数见归档 `run.py` 与 `extra-checks.py`。已有 `.upstream/` 若属于旧补丁系列，先保留旧目录再重新生成，不能覆盖手工编辑。

中间版本分别保存只读测试夹具失败和 ShellRoot 外层缓存导致无收益的源码/日志。正式结果只引用最终稳定版本。

## 后续

下一步继续分离 active/mixed 场景的宿主根布局与插件可见视图成本，并追踪生命周期 native 内存归属；普通路径历史超时仍需复现归因。
