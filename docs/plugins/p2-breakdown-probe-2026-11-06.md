# P2-A G5：长会话耗时分项测量

后续：[命令绘制修复与故障诊断](p2-command-diagnostics-2026-11-06.md)记录真实测试根绘制次数和运行时计数，并修复成功命令收尾的冗余通知。本页历史数值与失败记录保留，不以新复测覆盖。

日期：2026-11-06。承接 [G5 stress 测量](p2-stress-probe-2026-11-06.md)。新增独立 `--phase breakdown`，区分宿主聊天行同步、宿主绘制、首次视图打开、缓存视图绘制，以及本地命令和 HostModule 命令的分发、更新副作用、结果收尾和显式绘制。生产准入仍未通过。

## 工作负载与计时边界

沿用 `long-chat-v1`：2 个工作区、48 个会话、5,760 条消息，当前会话 120 条，正文 9,504,000 字节。三种场景各在独立进程运行，按轮次旋转顺序：

| 场景 | 注册包 | 最终 VM / 实例 | 测量 |
| --- | ---: | ---: | --- |
| host | 0 | 0 / 0 | 聊天行同步、宿主 notify 和绘制 |
| registered | 20 | 0 / 0 | 同上，含静态插件贡献 |
| plugin | 20 | 3 / 3 | 先测未激活宿主，再依次打开三个 owner，测最后一个可见摘要的缓存绘制及命令 |

三个 VM 驻留，其中两个视图隐藏。初次打开每个 owner 只有一次，共每进程 3 个样本；首次打开的 p95 等于该进程三个值的最大值，仅用于描述观测范围，不能作为充分采样的冷启动尾延迟。

`local` 和 `metadata` 命令都递增同一计数器、通知视图、返回相同结构。后者额外调用 `getWorkspaceMetadata` 和 `getSessionSummary`，核验工作区身份及 24 个会话；声明对应的两项权限。每条命令前清空旧结果，完成后核对结果已接受、任务清空、无错误及视图计数递增，防止失败或未完成命令被算为有效样本。所有场景结束时重新检查 VM / 实例数。

这两个命令只在 `prepare_packages(..., breakdown=True)` 时加入。默认 stress 生成包 SHA-256 仍为 `cb3da44c6c9061dc1d072c43b2349181744716bb273f39d8116e251c2601e24b`，与历史报告一致。

| 指标 | 实际边界 |
| --- | --- |
| `host_chat_rows_sync_ms` | 已加载、正文不变的 `sync_chat_rows`，含测试上下文调用；不测流式文本变化或重新分类 |
| `host_draw_ms` | 宿主 notify、更新结束副作用、事件队列收尾及显式测试 draw；plugin 场景中此项发生在激活之前 |
| `plugin_first_view_materialize_ms` | 首次打开 view、VM 创建、模块加载、初始化、首次描述树/原生组件构建及宿主绘制的合计；不是纯物化函数耗时 |
| `plugin_cached_view_draw_ms` | 三个 VM 驻留时宿主 notify 及绘制，复用未改变的脚本视图状态 |
| `plugin_*_dispatch_ms` | 在 GPUI 更新回调内部计时：宿主命令准备/校验、上下文同步、目标路由、同步 JS 执行、桥接返回和任务建立；不含回调退出后的自动绘制 |
| `plugin_*_effects_ms` | 整个更新耗时减内部 dispatch；含测试平台自动绘制和通知等副作用，不是纯布局计时 |
| `plugin_*_settle_ms` | `run_until_parked`：接收命令结果、宿主刷新及其可能触发的自动绘制 |
| `plugin_*_draw_ms` | 收尾后单独调用 `window.draw` 的耗时 |

GPUI `test-support` 会在 `App::flush_effects` 中自动绘制脏窗口。第一版把整个 `VisualTestContext::update` 当作 dispatch，因此混入绘制；该版保留为诊断记录，不参与最终结论。最终报告使用 `schema_version: 2`，把内部 dispatch 与其余 effects 分开。更新副作用、settle 和 draw 可能分别绘制窗口，不能把任一单项当成一次原生帧，也不能将不同指标的 p95 相加当作总耗时 p95。

`metadata` 指标表示包含两次 HostModule 调用的完整分发路径，包含 JS 和宿主路由，不能与 local 的 p95 直接相减得到单次 HostModule 净耗时。首次打开同样是组合指标。本轮不向正式运行时添加计时开销，也不改动 `.upstream` 或 Cargo 缓存源码。

## 重现与输出

在 `target/g5-validation-20261106` 隔离工作树执行，release 使用独立缓存：

```powershell
$env:CARGO_HOME = 'D:/enviroments/cargo'
$env:RUSTUP_HOME = 'D:/enviroments/rustup'
$env:CARGO_NET_GIT_FETCH_WITH_CLI = 'true'
python -m unittest discover -s tools/shell-prototype -p 'test_desktop*.py' -v
python tools/shell-prototype/verify-desktop.py --phase check --target-dir D:/m/conduit/target/shell-prototype
python tools/shell-prototype/verify-desktop.py --phase breakdown --samples 60 --warmup 10 --repeats 3 --profile dev --target-dir D:/m/conduit/target/shell-prototype
python tools/shell-prototype/verify-desktop.py --phase breakdown --samples 60 --warmup 10 --repeats 3 --profile release --target-dir D:/m/conduit/target/shell-prototype-g5-release
```

每个动作前等待 16ms，不计入耗时。报告为 `desktop-breakdown-results.json`、`desktop-breakdown-{profile}-results.json`，原始数据位于唯一的 `artifacts/desktop-breakdown/<时间戳-随机后缀>/`。旧 performance/stress 报告和原始目录不被覆盖。保留源码、测试二进制、生成包及上游摘要；测量期间源码变化会使结果无效。

Python 校验覆盖数据集、注册数、注册阶段和最终 VM / 实例数、首次打开数量、全部计时数组长度、HostModule 指标、有限非负值及进程内存。汇总先计算每个进程的 p95，再给出进程 p95 中位数与范围，并保留原始观测范围；profile 对照不作为准入门槛。

## 实测与下一步

最终对照使用相同 Rust 1.95.0 / LLVM 22.1.2、相同生成包和源码快照。dev 为失败后的完整复测，release 为完整首次测量；各 9 个独立进程，分别产生 2,700 个稳态分项样本及 9 次首次打开样本。两份完整报告均为 `valid_for_source_snapshot: true`。这不撤销下面保留的 dev 截止时间失败。

- 源码：`c3a5500e8362273c4b034dc0e0bdae13f28c8d0a808b65b09d715d6b0126d1ef`。
- dev 二进制：`204be5b8efbb504d2092b14925b23f1330ac4d4e6a28abdb979a43348462fe0f`。
- release 二进制：`caa4845852333c0cfd1f46924bf2df655f1ff5d662ad8107ed730d95e7ef625a`。
- 生成包：`b613ec29b754283e235b1c0f7bcd1632fb7f55c7250c68d2aaad441b1724e5f3`。
- 环境：Windows 11 build 26200，14 个逻辑 CPU；未控制其他应用负载。源码基线仍是隔离工作树中的 Conduit 0.1.9，主工作树后续 0.2.0 与并行功能变更不在测量范围内。

下表均为三个进程各自 p95 的中位数（最小–最大），单位 ms。

| 场景 / 指标 | dev | release |
| --- | ---: | ---: |
| host / 聊天行同步 | 0.043（0.042–0.046） | 0.028（0.025–0.032） |
| host / 宿主 notify/绘制（激活前） | 37.015（31.724–46.633） | 27.272（19.675–27.817） |
| registered / 聊天行同步 | 0.044（0.042–0.047） | 0.028（0.027–0.031） |
| registered / 宿主 notify/绘制（激活前） | 43.337（35.116–56.326） | 30.736（19.954–32.224） |
| plugin / 聊天行同步 | 0.041（0.039–0.042） | 0.028（0.028–0.029） |
| plugin / 宿主 notify/绘制（激活前） | 49.167（36.652–222.458） | 31.369（22.958–32.184） |

| plugin 场景指标 | dev | release |
| --- | ---: | ---: |
| 首次打开合计（每进程仅 3 点） | 53.652（53.123–58.894） | 48.341（32.363–50.384） |
| 缓存视图 notify/绘制 | 42.118（40.761–44.518） | 35.823（27.117–43.134） |
| 本地命令：同步分发 | 0.297（0.278–0.312） | 0.265（0.233–0.295） |
| 本地命令：更新副作用 | 24.584（22.065–25.773） | 19.262（14.498–20.256） |
| 本地命令：结果收尾 | 23.500（21.380–23.902） | 17.624（15.885–19.519） |
| 本地命令：显式 draw | 21.788（20.920–24.368） | 16.955（15.738–20.294） |
| HostModule 命令：同步分发 | 0.328（0.327–0.382） | 0.263（0.244–0.314） |
| HostModule 命令：更新副作用 | 23.771（21.699–30.569） | 18.648（12.689–19.097） |
| HostModule 命令：结果收尾 | 24.860（19.776–31.280） | 17.320（12.025–18.354） |
| HostModule 命令：显式 draw | 23.306（20.710–49.253） | 16.188（11.419–16.914） |

在当前固定摘要命令中，同步分发约 0.3ms，而更新副作用、结果收尾和额外显式 draw 各自达到十几至几十毫秒。稳态聊天行同步约 0.03–0.04ms；现有证据优先指向绘制及测试驱动重复帧路径，不支持把长会话总延迟主要归因于这两个简单脚本命令或 HostModule 数据访问。此结论不适用于任意复杂插件。

dev 的激活前宿主绘制仍有明显离群值（plugin 场景某进程 p95 达 222.458ms）；在该指标采样时尚未创建 VM，不能归因于 JS 执行。各轮差异也说明不能用 profile 间或场景间差值直接估计可归属开销。

下一步先给普通路径的截止时间故障补充命令/渲染阶段和原生调用证据，确认正常负载下是否可复现；同时记录单次事件触发的实际绘制次数，区分测试自动绘制、结果通知和显式 draw，再细分长会话 Markdown/布局成本。不得通过放宽 watchdog 预算把该故障变成通过。

归档目录为 `tools/shell-prototype/artifacts/g5-breakdown-20261106/`，包含两种 profile 的完整报告、失败/诊断记录、`profile-comparison.json` 和逐指标 `metrics.csv`。最终原始目录分别为 `desktop-breakdown/20261106T095625Z-n8zegvvt/`（dev）和 `desktop-breakdown/20261106T094712Z-qh15ubtm/`（release）。原始二进制留在隔离工作树；主工作树归档 JSON、日志及生成包。旧 performance/stress 证据另外按 452 个文件的 SHA-256 校验保留。

## 验证

| 检查 | 结果 |
| --- | --- |
| Rust 1.95.0 格式、all-targets + ui-tests check、Clippy `-D warnings` | 通过，源码摘要与正式采样相同 |
| dev 常规回归 | 插件 42 passed、3 个按需探针 ignored；更新 3 passed |
| release 常规回归 | 插件 42 passed、3 个按需探针 ignored；更新 3 passed |
| Python 报告/夹具/runner 测试 | 18 passed；隔离工作树与主工作树均通过 |
| 主工作树集成 | 只带入本任务 8 个源码/工具文件；触及 Rust 文件格式检查通过 |

源码、二进制和生成包一致性只在对应报告的范围内成立；主工作树并行功能的全量构建与回归不包含在本表中。

## 保留的诊断与失败

- `20261106T092859Z-iy8yzws0`：schema 1、20 样本试跑，三个场景通过，仅用于开发验证。
- `20261106T094012Z-b867teq7`：schema 1 的九进程测量；dispatch 包含 `update` 自动绘制，已被 schema 2 口径替代，不用于纯分发结论。
- `20261106T094315Z-c19ag3ll`：schema 2 dev 的九进程测量全部完成，但使用本机默认 Rust 1.99.1；release 缓存使用 Rust 1.95.0，因此该组不纳入最终 profile 对照。
- `20261106T095232Z-yca_5ki5`：切换到 Rust 1.95.0 后，第二轮 plugin 进程以 101 退出，报 `stress.plugin-02: script runtime disabled: execution deadline exceeded`。整个测量 `valid_for_source_snapshot: false`，已保存为 `failed-dev-deadline-results.json`，未将部分样本拼入成功汇总。

失败后采集到的系统总 CPU 瞬时值约 17%，当时未看到 cargo/rustc 进程；这不能还原故障瞬间的线程调度，不能据此归因于背景负载，也不能证明预算误报。运行时限额保持原值。这次普通命令/绘制路径的截止时间故障仍未定位，即使后续相同参数复测成功，也不能将其视为已经修复。

结果只适用于报告内的隔离源码快照。GPUI 测试平台没有原生 GPU 呈现，仍需 Windows IME、原生窗口/滚动、release/Fast 成品及生产资源验收。完整准入前不冻结 P2 生产契约，也不转入 P3 安装管理。
