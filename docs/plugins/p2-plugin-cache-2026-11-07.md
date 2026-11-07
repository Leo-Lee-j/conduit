# P2-A G5：插件视图缓存与容器布局

日期：2026-11-07（Asia/Shanghai）。承接 [聊天缓存与 Shell 宿主边界](p2-chat-cache-2026-11-07.md)。本轮消除了静态右侧插件随宿主输入/重绘重复构造原生控件的问题，并完成输入、主题、布局及生命周期验证。**G5 生产准入仍未通过**；下文区分确定的工作量变化与墙钟时间结果。

## 实现与回归

`render_shell_view` 通过 GPUI `.cached(StyleRefinement::default().size_full())` 挂载 ScriptView。脚本命令、retained 控件更新、元数据刷新仍走实体通知；尺寸、裁剪、继承文本样式及窗口 refresh 沿用 GPUI 缓存失效。此处测量的是右侧插件；左侧导航还有其父级 Sidebar 的刷新规则，未宣称两侧开销均已消除。

直接加缓存会暴露根布局问题：GPUI 将缓存子树作为独立根布局，只有 available space 不会撑开脚本的 intrinsic 根，输入组实测缩到 22px，中文编辑等 3 项既有回归失败。本轮通过 `shell-view-container-layout.patch` 增加 `ScriptView::set_fill_container`：默认关闭，由 Conduit 嵌入视图启用一个满尺寸纵向 flex 根。独立脚本应用和浮层保持默认布局。补丁由 `prepare_upstream.py` 重建，没有修改 Cargo 缓存或直接编辑生成源码。

新增 3 项 dev/release 均运行的桌面回归：宿主中文/emoji 输入不重建插件且命令更新一次；元数据、明暗主题及窗口尺寸正确失效；输入组保持可用宽度并支持中文替换。旧实现先复现失败：14 字符输入加选择/通知/draw 多出 17 次物化，脚本 render 未增加。初版缓存的布局失败与诊断日志也全部保留。

## 固定负载与测量口径

保持 `long-chat-v1`：2 工作区 ×24 会话 ×120 消息（5,760 条、9,504,000 文本字节），20 个注册包、3 个驻留 VM、1 个可见右侧视图，1600×800。每进程 10 次预热、60 次正式样本，每场景 3 个独立进程；生命周期每进程 20 轮。Rust 1.97.1 / LLVM 22.1.6。平台为 GPUI test，无 GPU 呈现。

未修改 stress / chat-profile / diagnostics 探针、数据集、计数口径或参考预算。预算仍为 `max(2ms, baseline p95 × 10%)`。active 打开右侧栏，场景增量还包含宿主布局变化，不能当作纯插件 VM 开销。

完整矩阵之外，保留上一轮 release 二进制与本轮 release 二进制，验证哈希后交替执行 baseline / registered / active / mixed，每组 3 对独立进程（24 进程）。每轮旋转场景顺序，每个相邻对交替先后顺序；使用同一批包、同一负载和验证器。首次 A/B 与检查任务重叠后被中止，原始记录保留在 `ab-release-interrupted-overlap/`，不计入有效样本。最终 A/B 运行时没有并行编译或我们启动的其他测量，但没有控制整机后台负载。它改善历史前后分时比较的偏差，仍不是置信区间或生产准入。

## 可确定的重复工作变化

优化前 dev 6 进程与优化后 dev/release 各 6 进程的 diagnostics 一致，表中为每正式样本计数：

| 阶段 | 根 render | 修改前物化 | 修改后物化 | 脚本 render |
| --- | ---: | ---: | ---: | ---: |
| 静态插件 notify + draw | 2 | 2 | 0 | 0 |
| 命令分发与自动绘制 | 1 | 1 | 1 | 1 |
| 成功结果收尾 | 0 | 0 | 0 | 0 |
| 命令后额外显式 draw | 1 | 1 | 0 | 0 |

local / metadata 两类命令均通过结果和快照验证。优化后正式静态样本 720 个及命令后额外显式 draw 720 个，物化计数全部为 0；1,440 条正式命令各保留 1 次脚本 render / 物化。缓存跳过的是子树构造/布局/绘制回调，根遍历和帧数据复用仍有成本。

## release 交替 A/B

下表为各版本每进程 p95 的三进程中位数，单位 ms。配对变化是相邻前后进程 p95 差值的中位数，与两个中位数相减不一定相等。

| 场景 | 编辑+draw 前 | 编辑+draw 后 | 配对变化 | notify+draw 前 | notify+draw 后 | 配对变化 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline | 42.817 | 42.031 | -0.039 | 6.743 | 7.003 | 0.464 |
| registered | 52.104 | 50.221 | 0.643 | 8.235 | 8.126 | -0.110 |
| active | 71.204 | 61.924 | -6.431 | 10.961 | 11.231 | -0.094 |
| mixed | 73.728 | 62.109 | -11.619 | 9.756 | 10.263 | -0.601 |

| 修改后 A/B 场景 | 编辑增量 / 预算 | notify 增量 / 预算 |
| --- | ---: | ---: |
| registered | 8.190 / 4.203（超出） | 1.123 / 2.000（内） |
| active | 19.893 / 4.203（超出） | 4.228 / 2.000（超出） |
| mixed | 20.078 / 4.203（超出） | 3.259 / 2.000（超出） |

active/mixed 的三组配对编辑 p95 均下降，配对变化中位数分别为 -6.431ms / -11.619ms；两版本 p95 中位数分别下降约 13.0% / 15.8%。notify+draw 的配对有升有降，两个场景的 p95 中位数反而上升，不能宣称稳定改善。registered 的编辑增量也仍超预算。

逐进程 p95、配对差值和原始 60 个样本见 `ab-release/report.json` 及同目录 JSON。不能把重复构造清零等同于整帧耗时清零；是否出现稳定时间收益，应同时看对照场景和配对结果。

## 完整矩阵与历史对照

以下“前”来自上一轮聊天缓存报告、“后”来自本轮矩阵；两轮不交替，包含时间与环境差异。每进程 p95 的三进程中位数，ms：

| 构建 / 场景 | 编辑+draw 前→后 | notify+draw 前→后 |
| --- | ---: | ---: |
| dev baseline | 43.452 → 54.412 | 7.108 → 8.636 |
| dev registered | 51.100 → 64.158 | 8.440 → 9.657 |
| dev active | 73.417 → 84.225 | 12.988 → 11.924 |
| dev mixed | 79.573 → 82.499 | 12.112 → 11.973 |
| dev lifecycle | 55.693 → 65.065 | 9.075 → 9.781 |
| dev idle | 56.762 → 61.139 | 8.336 → 9.941 |
| release baseline | 42.704 → 41.353 | 7.460 → 7.240 |
| release registered | 46.172 → 46.355 | 7.499 → 7.662 |
| release active | 69.592 → 62.937 | 9.829 → 9.371 |
| release mixed | 64.442 → 63.377 | 9.986 → 10.056 |
| release lifecycle | 49.773 → 46.071 | 7.341 → 7.615 |
| release idle | 49.891 → 47.042 | 7.622 → 7.853 |

单字符 p95 在每进程池化 840 次正式字符输入后取三进程中位数，ms：

| 构建 / 场景 | 上轮 | 本轮 |
| --- | ---: | ---: |
| dev baseline | 3.327 | 4.295 |
| dev registered | 4.750 | 4.999 |
| dev active | 6.367 | 6.228 |
| release baseline | 2.844 | 2.996 |
| release registered | 3.123 | 3.556 |
| release active | 4.484 | 4.447 |

18 个 chat-profile 进程中的 15,120 次正式字符输入仍全部保持聊天行与 Markdown 阶段计数为 0，每字符 1 次根 render；计时外均通过非零绘制校准、末条聊天可见性及输入焦点验证。分项 p95 不能相加代替完整编辑 p95。

## 生命周期与验证

每个进程第20个 settled 点减第1个点，再取3进程中位数，MiB；不是峰值或泄漏判定：

| 构建 / 场景 | working set | private commit |
| --- | ---: | ---: |
| dev lifecycle | 3.441 | 4.199 |
| dev idle | 0.387 | 0.387 |
| release lifecycle | 8.445 | 9.266 |
| release idle | 0.367 | 0.367 |

本轮release lifecycle的settled private增量为9.266MiB，高于上一轮3.625MiB（idle为0.367MiB）。缓存保留更多native帧状态可能影响驻留分配，但本轮没有分配归属证据，不能将增长归因于缓存或宣称无泄漏；这是后续需优先排查的风险。

- dev 桌面回归103通过；release61通过，各5个 opt-in 测量探针由独立 runner 执行。
- Python28通过；格式、all-targets/ui-tests 类型检查和 Clippy `-D warnings` 通过。
- 默认 Shell 原型67通过；3项资源探针分别实际运行1项测试通过。
- 最终源码矩阵66进程（18 chat-profile、36 stress、12 diagnostics）通过；另有修改前6个dev diagnostics与24个release A/B进程，共96个有效测量进程。
- 启停120轮、idle对照120轮；每次停止验证3个VM释放，240个内存曲线点见 `lifecycle-memory.csv`。
- 主工作区 `cargo check --locked --all-targets --features ui-tests --jobs 2` 通过（包含原有Git功能改动）。
- 主工作区6个实现/测试/准备文件与隔离工作树一致，增量补丁在全新目录可重建；主工作区上游从头生成哈希一致。原有暂存区保持不变。

历史普通路径 watchdog 超时本轮未复现，仍未归因；mixed 的有意故障被单独验证隔离。G5 仍需真实GPU/滚动/Windows IME及成品验收，剩余宿主开销需继续归因，内存需要更长窗口和native分配归属分析。本轮不改变P2冻结或P3状态。

## 证据与复现

证据目录：`tools/shell-prototype/artifacts/g5-plugin-cache-20261107T013500/`。原始轨迹、包、报告、失败尝试与日志均有保留；主工作区导出非二进制证据，二进制及上游备份保留在隔离目录。上一轮证据逐文件哈希复核见 `historical-evidence-check.json`。

- Conduit 隔离基线：`4a46c64385cb8ac4330b0c9ed1b67a7f0e0a6c9a`
- 最终源码：`31d3b0527c253fb4474f08a3287fbed92c96e6d14c4523362a3d61acb3dfd0a9`
- 上游 revision：`11345ec002d9f182a266b5d5dd3a217090700a87`
- 补丁系列：`7fe64c989d4246c9484b778ea33d6a7d8edeb177a8715db5363e42c0ae7141b8`
- 生成源码：`b14b2616b4948e757bc4953a4ab76f067b866872cefc71f99a8e0d0e11b112c0`
- dev 最终测量二进制：`0b0364f88dfde08408b678b7f1c009b9e84c714265a97df24edb168c1df74b7a`
- release 最终测量二进制：`ae7e9666d3cd2d33ab447d2e41c5e3b1bbae6113f36cc33fd944ccaa3be0c629`

`run.py`、`extra-checks.py` 记录矩阵，`ab-release.py` 记录交替运行，`optimization.patch` 是本轮增量，`snapshot.patch` 和 `snapshot-new-files/` 用于重建完整隔离快照。报告 JSON 中的原始相对路径以各自运行目录为准；主工作区镜像存于 `raw/`。`production_gate` 保持 `not_evaluated`。
