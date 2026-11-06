# P2-A G5：长会话、多插件与生命周期 stress 测量

日期：2026-11-06。承接 [release 界面回归](p2-release-ui-regression-2026-11-06.md) 与 [六场景性能探针](p2-performance-probe-2026-11-06.md)。本次新增独立 `--phase stress`，沿用源码稳定性、二进制摘要、独立进程和原始样本校验；原有 performance 六场景及历史报告保持独立。

## 固定负载与口径

`long-chat-v1` 数据集包含 2 个工作区、每个 24 个会话、每会话 120 条消息，共 5,760 条，正文共 9,504,000 字节，最长单条 10,560 字节。正文交替使用 User / Assistant，包含中文、emoji、列表和 Rust 代码块，每十条消息中一条使用更长正文。内容在所有场景中固定，输出总 UTF-8 字节数、最长消息字节数和正文 SHA-256；Python 校验器独立重建这些字段。探针确认选中会话已加载 120 条消息，沿用真实 ConduitApp 的布局和聊天渲染。

runner 从 `development-overview` 复制 20 个包，分别使用 `stress.plugin-00` 至 `stress.plugin-19`，同时改写 owner、view、command、activity 和 status ID，快照准备仍走原有清单与授权校验。最后一个包的 increment 命令被替换为同步死循环，只在 mixed 场景显式执行，由现有运行时执行截止时间中断。生成包摘要记录在报告中，runner 对测试进程另设超时。

| 场景 | 注册包 | 稳态 VM / 实例 | 行为 |
| --- | ---: | ---: | --- |
| baseline | 0 | 0 / 0 | 长会话宿主基线 |
| registered | 20 | 0 / 0 | 只注册贡献，不创建 VM |
| active | 20 | 3 / 3 | 三个 owner 分别打开摘要，轮流执行命令 |
| mixed | 20 | 3 / 3 | 正常 VM 驻留时另激活一个异常 owner；超时后检查故障 VM 释放、正常 VM 身份不变及命令可继续执行 |
| lifecycle | 20 | 0 / 0 | 稳态宿主采样后反复执行三个 owner 的激活 → 命令/绘制 → 停止 → settled 采样 |
| idle | 20 | 0 / 0 | 不激活 VM，执行相同轮数的空闲绘制/停止/settled 采样，作为宿主背景增长参考 |

三个活动 VM 同时驻留，当前场景只有最后选中的一个右侧视图可见，另两个视图缓存隐藏。此处不宣称三个插件同时可见或并行执行。生命周期循环保留同一批包快照，不在每轮重复生成或安装包。

每种场景测量整段输入替换及绘制、宿主 notify 及绘制；active / mixed 另外测量依次分发三个插件命令、切换右侧容器并绘制的总耗时。命令后检查三个缓存视图的计数，检查位于该项计时循环之外。输入长度固定，并检查最终文本与焦点。每个动作前的 16ms sleep 不计入耗时。指标仍包含 GPUI 测试调度和 CPU 布局开销，不含原生 GPU 呈现，也不是逐键延迟、滚动 p95 或单次脚本/HostModule 耗时。

生命周期每轮保存 ready、after_stop、settled 三个进程内存点、激活/停止耗时、VM/实例计数和释放的弱引用数。settled 指停止后固定等待 16ms 并执行两次测试 draw，不声称执行了 GC 或操作系统已回收页面。汇总保留逐轮数据，计算末轮减首轮、相对首轮的最大增长及最小二乘斜率；不把单次增长或正斜率判定为泄漏，`leak_verdict` 和 `production_gate` 均为 `not_evaluated`。idle 的工作量比启停循环低，只作背景参考，不能直接相减得出插件可归属内存。

## 重现

```powershell
$env:CARGO_HOME = 'D:/enviroments/cargo'
$env:RUSTUP_HOME = 'D:/enviroments/rustup'
$env:CARGO_NET_GIT_FETCH_WITH_CLI = 'true'
python tools/shell-prototype/verify-desktop.py --phase stress --profile dev --samples 60 --warmup 10 --repeats 3 --cycles 20 --target-dir D:/m/conduit/target/shell-prototype
python tools/shell-prototype/verify-desktop.py --phase stress --profile release --samples 60 --warmup 10 --repeats 3 --cycles 20 --target-dir D:/m/conduit/target/shell-prototype-g5-release
python -m unittest discover -s tools/shell-prototype -p 'test_desktop*.py' -v
```

本轮命令在 `target/g5-validation-20261106` 隔离工作树运行。共享主工作树存在并行修改，验证只对应报告中的源码摘要。`--cycles` 范围 3–200，默认 20；samples / warmup / repeats 范围沿用已有 runner。stress 输出 `desktop-stress-results.json`、`desktop-stress-{profile}-results.json` 和唯一的 `artifacts/desktop-stress/<时间戳-随机后缀>/` 原始目录，避免覆盖 performance 数据。归档目录为 `artifacts/g5-stress-20261106/`。

## 实测记录

最初 20 samples / 1 warmup / 1 repeat / 3 cycles 的六场景试跑均成功执行，但期间补充了 Python 测试文件，源码稳定性校验拒绝该轮结果。它仅作为开发排错记录保留于 `smoke-source-changed-results.json`，不作为性能结论。

### dev

固定源码 `b34b760aca6a077785ca135c4c8fbde39890061976c325dbe40a8d39e7997784`；二进制 `e39ed9b06bf69ee12acef3597a02eb07f425a0440f0030e19e03e23610a52fe3`。18 个独立进程均退出 0，`valid_for_source_snapshot: true`。每项 60 samples、10 warmup，三轮合计 2,520 个计时样本，另有 60 次启停和 60 次空闲循环。

| 场景 | 编辑及绘制 p95 中位数（范围，ms） | notify 及绘制 p95 中位数（范围，ms） | ready 私有提交（MiB） |
| --- | ---: | ---: | ---: |
| baseline | 567.221（286.458–987.955） | 50.836（38.328–88.347） | 60.320 |
| registered | 1062.968（305.489–1085.024） | 40.139（38.877–41.243） | 64.090 |
| active | 327.683（309.687–348.737） | 38.622（37.256–45.591） | 78.707 |
| mixed | 397.260（359.229–406.770） | 43.312（41.989–49.825） | 81.621 |
| lifecycle | 328.768（316.004–418.084） | 37.659（36.055–38.847） | 63.938 |
| idle | 282.580（254.126–330.607） | 34.813（31.637–36.482） | 63.691 |

三个插件命令及绘制的 p95 中位数：active 为 191.520（162.691–204.700）ms，mixed 为 207.648（122.862–676.583）ms。

宿主输入 p95 在同一场景各轮之间波动很大，甚至未激活插件的 registered 场景也出现高值。当前采样含测试调度与机器背景负载，不能把场景差值当作可归属插件开销，更不能把负增量解释为插件加速宿主。

格式、all-targets + ui-tests check、Clippy `-D warnings` 通过；同一 dev 二进制的常规回归为 42 项插件 + 3 项更新测试通过，两个按需性能探针默认忽略。Python 校验测试共 10 项通过。

### release

与 dev 相同源码 `b34b760aca6a077785ca135c4c8fbde39890061976c325dbe40a8d39e7997784`；release 二进制 `4ece6e4feb5b6e8a438caef8c1dd8d3edb45348bdde2b2ebf85ceb0f50e304c3`，未开启 debug assertions。18 个进程退出 0，`valid_for_source_snapshot: true`，采样参数和样本数与 dev 相同。

| 场景 | 编辑及绘制 p95 中位数（范围，ms） | notify 及绘制 p95 中位数（范围，ms） | ready 私有提交（MiB） |
| --- | ---: | ---: | ---: |
| baseline | 197.850（197.013–233.213） | 26.602（25.974–27.368） | 60.473 |
| registered | 254.664（239.862–270.859） | 30.798（28.458–31.210） | 63.582 |
| active | 243.374（236.337–249.547） | 31.470（31.254–36.264） | 78.059 |
| mixed | 253.235（241.738–256.344） | 32.524（32.444–40.917） | 81.406 |
| lifecycle | 226.095（223.566–297.914） | 30.086（29.877–31.504） | 63.887 |
| idle | 219.908（219.521–240.155） | 31.174（30.196–31.890） | 64.152 |

三个插件命令及绘制 p95 中位数：active 为 137.238（136.059–137.626）ms，mixed 为 136.710（134.904–150.401）ms。三轮故障命令中断及随后绘制合计 70.196–72.414ms；每轮均确认故障 VM 释放、三个正常 VM 身份不变且命令继续成功。

同一 release 二进制常规回归为 42 项插件 + 3 项更新测试通过、0 失败，两个性能探针默认忽略。原 performance 六场景另以 dev 小样本验证兼容性全部通过，数据单独归档，未替换原性能基线。

### 内存趋势与验收结论

| Profile / 场景 | 私有提交末轮−首轮中位数（MiB） | 私有提交斜率中位数（MiB/轮） |
| --- | ---: | ---: |
| dev / lifecycle | 7.250 | 0.203 |
| dev / idle | 13.746 | 0.150 |
| release / lifecycle | 7.574 | 0.180 |
| release / idle | 14.102 | 0.202 |

两种 profile 的全部 120 次启停循环都验证了每次三个 VM 弱引用释放、runtime/instance 回到 0；它证明 VM 生命周期收尾符合预期，不证明进程总内存立即回到注册值。启停与空闲对照都存在台阶和回落，因此无法从这 20 轮窗口判定持续泄漏或排除泄漏。逐轮 CSV 和原始 JSON 同时保留。

本次 release 长会话输入/绘制和 notify p95 均明显高于旧小数据集；相对本轮 baseline 的参考预算也有超出，不能标记 G5 生产性能通过。下一步优先分解宿主聊天布局、脚本执行、视图物化及 HostModule 的耗时，并控制机器背景负载；原生窗口/滚动性能需另行验收。

归档 `artifacts/g5-stress-20261106/` 包含 dev / release 汇总、静态检查、常规回归、原六场景兼容验证和两份逐轮 CSV。dev 原始目录为 `artifacts/desktop-stress/20261106T042207Z-l90id4ij/`，release 为 `artifacts/desktop-stress/20261106T043833Z-pxl6u9_f/`。原始二进制保留在隔离工作树；主工作树复制 JSON、日志和生成包。第一次 release 构建误用另一缓存目录，在采样前主动停止，记录为 `aborted-release-build.json`；后续完整采样使用已有 release 缓存，未将中断构建计为通过。


## 后续准入

本次补齐合成长会话、多包注册、多个活动 VM、异常与正常插件共存、重复启停内存曲线的测量入口。后续依据实测曲线定位内存波动及可能的持续增长来源，拆分脚本执行、视图物化和 HostModule 耗时，并在原生连接恢复后完成 Windows IME、release / Fast 成品和输入到 GPU 呈现验收。完整准入通过前不冻结 P2 生产契约，也不据此转入 P3 安装管理。
