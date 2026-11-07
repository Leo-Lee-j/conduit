# P2-A G5：启停内存归因

日期：2026-11-07；Windows，release，GPUI 无 GPU 测试平台。本轮延续插件缓存后的内存风险排查。

**缓存版更高的 20 轮内存增量未在交替对照中复现；已确认原探针的逐轮 JSON 记录会贡献增长。固定记录后追踪到的每轮4块小分配来自测试未投递的下一帧回调，独立内存探针已补齐帧处理。G5生产准入仍未通过。**

## 保留二进制的 200 轮交替对照

沿用插件缓存前/后的原始 release 二进制，核对 SHA-256；固定 20 个包、3 个活动 VM、48 个会话、5,760 条消息、9,504,000 字节文本。每进程先执行 10 次预热、60 次正式编辑和通知；前后版本交替，lifecycle/idle 顺序轮换，各 3 个独立进程。

下表先在每进程计算 settled 差值，再取三进程中位数，MiB。idle 工作量不同，不能直接相减视作泄漏量。

| 场景 | 私有内存20−1 | 私有内存200−1 | 私有内存200−100 | 工作集200−1 |
| --- | ---: | ---: | ---: | ---: |
| 缓存前 lifecycle | 3.473 | 5.598 | 1.469 | 4.863 |
| 缓存后 lifecycle | 2.430 | 3.617 | 0.793 | 2.859 |
| 缓存前 idle | 0.332 | 1.008 | 0.672 | 1.008 |
| 缓存后 idle | 0.336 | 0.977 | 0.430 | 0.977 |

三组 lifecycle 的缓存后增量均低于缓存前。该结果不能证明缓存降低长期泄漏，也不能把上轮 9.266 MiB 改写为无效数据；它说明单次短窗口结果不足以归因。

## 分配与记录开销

新增 `allocation-probe` 显式测试特性，只有 `cfg(test)` 才安装 System 包装计数器；不改变分配后端。计数覆盖经过 Rust 全局分配器的请求字节/块数，不覆盖直接使用原生分配器的库、分配器内部元数据或 GPU。两次原子读取不是跨线程事务快照；原子操作也会影响计时，因此探针的耗时不用于性能预算验收。

新探针预分配所有数值记录，支持 `fixed` / `json` 对照。`json` 额外保留与旧 stress 相同的每轮字段；结束时单独释放记录，测量实际回收。不是事后从旧曲线扣除一个估计值。

200 轮三进程中位数，首尾 settled 差值：

| 场景/记录 | 私有内存MiB | Rust存活MiB | Rust存活块数 |
| --- | ---: | ---: | ---: |
| lifecycle/fixed | 2.883 | 1.048 | 797 |
| lifecycle/json | 2.930 | 1.691 | 6,444 |
| idle/fixed | 0.730 | 0 | 0 |
| idle/json | 1.234 | 0.592 | 5,573 |

idle/json 的三进程都增加 **620,407 字节/5,573 块**，idle/fixed 都为零。释放完整 200 条 JSON 记录时，每个 json 进程回收 **623,432 字节**；settled 最后一个点尚未包含第200条记录，所以与首尾差值不同。私有内存仍受分配器驻留和其他原生状态影响，不能与存活请求字节等同。

## 虚拟时钟和 1,000 轮窗口

原 stress 用真实 `thread::sleep` 节流，但 GPUI 测试调度器使用虚拟时间。追加诊断分别保持时钟静止、每轮推进32ms，并在末尾另采推进5秒前后状态；原始曲线不被末尾样本替换。

200轮静止时钟：推进5秒后，lifecycle减少461,190字节/928块，idle减少460,734字节/927块。两组相近，因此不能把启停专有增长归因于这些到期任务。

1,000轮推进时钟的三进程中位数（每进程首尾差值）：

| 场景 | 私有内存MiB | Rust存活MiB | Rust存活块数 |
| --- | ---: | ---: | ---: |
| lifecycle | 7.445 | 4.964 | 5,577 |
| idle | 1.371 | 1.021 | 96 |

推进时钟会触发宿主周期任务，工作负载与静止时钟不同，不能混算为同一A/B。曲线包含容量阶跃和周期波动；启停中常见每轮128字节/4块增长，归因见下一节。

## 下一帧回调归因与探针修正

对第101–150轮申请、该窗口结束仍存活的32字节块抓取栈，最大的两组各100块，调用链分别经过 `ConduitApp::document_tab_bar` 和 `ConduitApp::render_status_contributions` 的 `on_prepaint`。二者都在宽度变化时用 `window.on_next_frame` 排队；打开、关闭侧栏各触发一次，所以每轮4块。

GPUI的生产帧回调会取走并执行 `next_frame_callbacks`；测试的 `window.draw()` 本身不投递这些回调。原测试辅助函数也没有使用返回的 `ArenaClearNeeded` 清理绘制临时区。前者解释持续增长的回调对象，后者需要按生产帧契约清理；本轮未分别量化所有容量阶跃的来源。

同一最终二进制的手动帧对照：200轮后积压 **804个回调**。推进虚拟时钟5秒并不能投递它们。随后正常投递下一帧并完成绘制清理，Rust存活分配从49,834,664降到49,312,452字节（减少522,212字节），块数从68,442降到66,628。这个差值包含回调执行、界面状态和临时区清理，不能全部归到回调盒子的32字节。

新探针的 `complete` 帧模式在绘制前投递上一帧回调，绘制后执行 `window.draw(cx).clear(cx)`；`manual`保留原路径作对照。回归测试验证：手动draw不执行帧回调；完整帧执行已有回调；回调中新排的工作到下一帧才执行。没有强制清空业务队列或修改生产界面代码。

旧 stress/计时探针及其历史报告保持原含义。长时间内存判断应使用这里的固定记录和完整帧模式，不能继续把旧曲线直接当泄漏证据。

## 完整帧的1,000轮复测

每场景3个独立进程，固定记录、每轮推进32ms虚拟时间；最终二进制一致。首尾settled差值的三进程中位数：

| 场景 | 私有内存MiB | Rust存活MiB | Rust存活块数 | 最后100轮Rust增量（字节/块，逐进程） |
| --- | ---: | ---: | ---: | --- |
| lifecycle-fixed | 1.895 | 0.763 | 481 | 0 / 0；0 / 0；7493 / 5 |
| idle-fixed | 0.516 | 0.021 | 95 | 0 / 0；0 / 0；0 / 0 |

6个进程末尾待处理帧回调均为0，额外补投递一帧没有改变Rust存活字节/块数。推进5秒时钟的独立变化逐进程记录在原始JSON；第3个lifecycle进程减少7,493字节/5块，因此不能笼统宣称所有延迟任务状态不变。此前每轮128字节/4块的队列累积已不再出现。仍可见阶段性容量/状态变化，本轮没有对所有分配做全量归属，因此结论限定为修正并验证这项探针偏差，不宣称成品绝对无泄漏。

## 边界

所有生命周期点验证3个VM弱引用失效、运行时/实例计数归零。这排除了对应VM强引用残留，不证明所有原生分配释放。未调用强制GC、工作集裁剪或改变资源预算。原生GPU、Windows IME、滚动及成品性能验收仍待完成；P2冻结、P3状态不变。

## 验证与复现

最终分配器自检、帧顺序回归、格式检查、release/all-targets Clippy（`-D warnings`）通过。桌面回归共58项通过、0项失败，6个opt-in探针在常规回归中跳过，本轮stress和native-memory由独立runner执行；Python28项通过。主工作区集成类型检查见 `main-check.json`，原暂存区未改变。

42个正式矩阵进程，共18,000个生命周期/空闲采样点：9,000次启停（逐次验证3个VM释放）及9,000次idle对照。另有1个32字节分配调用栈实验、1个手动帧积压诊断，均单列；其额外开销不进入性能结论。

最初构建成功后，归档脚本误将Path传给要求日志字符串的解析函数而失败；日志保留为 `allocation-build-initial.log`，修正脚本后复用构建产物通过。测量矩阵没有失败进程被剔除后仅挑选样本发布。

证据目录：`tools/shell-prototype/artifacts/g5-native-memory-20261107T024328/`。`report.json`为原二进制交替对照；`allocation-*`为逐版本分配诊断矩阵；`summary.json`与`memory-curves.csv`为派生汇总；`trace-symbols.json`含地址及解析栈；`manual-callback-drain.json`含804个回调的诊断。`identities.json`映射各轮构建到保留二进制，不要混用版本。

[完整曲线PNG](../../tools/shell-prototype/artifacts/g5-native-memory-20261107T024328/memory-curves.png) · [SVG](../../tools/shell-prototype/artifacts/g5-native-memory-20261107T024328/memory-curves.svg)

最终测量快照SHA-256：`6646adb9b6c19b79656161a197178f7d237889159d3bf3b8563579077d4f3717`；release探针二进制：`cd484a92d4424111fd90ba2b87d03ddec3c05cee57b94867523bf34c167ca37a`。上游及补丁沿用插件缓存轮，没有修改生成源码或Cargo缓存。

复现：在隔离工作树用 `cargo test --locked --manifest-path tools/shell-prototype/Cargo.toml -p conduit-shell-desktop --release --features allocation-probe --no-run` 构建。取生成的测试exe，设置 `CONDUIT_PERF_SCENARIO=lifecycle|idle`、`CONDUIT_MEMORY_RECORDING=fixed`、`CONDUIT_MEMORY_CLOCK=advance`、`CONDUIT_MEMORY_FRAMES=complete`、`CONDUIT_STRESS_CYCLES=1000`、`CONDUIT_STRESS_PACKAGES`（归档packages目录）、`CONDUIT_DATA_DIR`（独立临时目录）、`CONDUIT_PERF_OUTPUT`（独立JSON路径），运行 `app::plugin_ui::shell::tests::native_memory::desktop_native_memory_probe --exact --ignored --test-threads=1 --nocapture`。

`manual`和`json`用于观察旧驱动/记录方式的偏差。归档 `build.py`、`allocation-run.py 1000 advance complete` 记录复测过程；runner拒绝覆盖已有矩阵目录。复跑时应放入新的归档目录并准备相同包。二进制、PDB、绘图依赖只保留在隔离目录，主工作区导出其余证据与哈希清单。

集成收尾仅为采样访问器补上 `#[cfg(conduit_shell_desktop)]`，消除普通根测试的未使用警告；桌面探针编译分支与测量时一致。测量源码另存于 `measured-source/`，集成源码在 `final-source/`，这处差异及最终检查记录在 `post-measurement-cfg.json`。未将旧二进制冒充为修改后重新构建的产物。
