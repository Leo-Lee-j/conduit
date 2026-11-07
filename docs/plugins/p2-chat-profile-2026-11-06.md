# P2-A G5：输入粒度与聊天 Markdown 分项

后续：[聊天缓存与 Shell 宿主边界](p2-chat-cache-2026-11-07.md)已完成输入重复绘制优化及跨构建模式验证；本文保留优化前的原始分项与 G5 结果。

日期：2026-11-06；正式矩阵约 22:27–23:12（Asia/Shanghai）。承接 [独立重跑](p2-g5-rerun-2026-11-06.md)，本轮继续定位长会话成本。G5 生产准入保持未通过，普通路径历史超时保持待定位。

## 构建恢复

本次隔离工作树基于 `4a46c64`（Conduit 0.2.1），未包含主工作树并行 Git 功能改动。环境为 Rust 1.97.1 / LLVM 22.1.6，不能直接和上轮 Rust 1.95.0 的耗时作优化对比。

首次构建发现两项可复现阻塞：

- Windows 检出把补丁转成 CRLF，`git -c core.autocrlf=false apply` 本身不转换补丁输入。`prepare_upstream.py` 现在将 LF 规范化后的补丁通过 stdin 交给 Git，原始补丁文件及其 SHA-256 保持不变。真实 Git 回归覆盖 LF/CRLF 生成相同源码、原补丁字节未变、重复准备通过以及拒绝覆盖已修改的生成源码。
- 原型 `Cargo.lock` 中本地 `conduit` 仍是 0.2.0，`--locked` 拒绝构建；只同步为 0.2.1，未更新第三方依赖。

本轮 Windows 原补丁集合哈希和历史 LF 检出的哈希不同；生成源码仍为 `a366593fbca9ee2b08abd4c81dbda79403344def2a1b554ea8bba990c57763ff`，和上轮相同。

## 设置返回焦点回归

常规 G5 前置回归在 `desktop_settings_and_narrow_dock_preserve_plugin_input` 失败：设置由弹窗改为工作区标签页后，关闭时调用 `select_document`，无条件把焦点移回聊天主输入框。本轮以弱焦点句柄保存打开设置前的控件，关闭时恢复；已关闭/销毁的控件退回文档默认输入框。重新打开已选中的设置页不改写原焦点；主动切换或关闭文档时清除旧记录。

原回归补充重复打开设置的场景；新增插件停用时关闭设置的回归，验证 VM 释放和输入焦点回退。

## 测量边界

新增独立 `--phase chat-profile`。历史 performance、stress、breakdown 和 diagnostics 口径保留。

固定 `long-chat-v1`、1600×800 测试窗口，分别测 baseline / registered（20 包）/ active（3 VM）。active 打开右侧插件栏，会改变聊天可用宽度；各场景差值同时包含宿主布局变化，不能单独归因为插件运行时。依次记录聊天行同步、Ctrl+A、`G5 中文😀 input A/B` 的 14 个 Unicode 标量输入、显式 draw、notify + draw。每轮验证完整输入、焦点和 VM / 实例数量；活动场景检查视图内容。

旧 `simulate_input` 已逐字符派发输入。因此旧 `host_edit_draw_ms` 是 Ctrl+A、14 字符输入与显式绘制的合计，不能解释为单键延迟。新探针逐字符调用 `simulate_input`，每次会运行测试执行器直到停驻；JSONL 写入在各计时区间外但会改变调度，因此不能把新分项相加当作旧探针的严格等价总耗时。

每个阶段附带实际根 render 次数，以及聊天行构造、Markdown request_layout / prepaint / paint 的调用次数及累计时间。统计仅在 Shell 桌面 UI 测试启用，正式应用路径不变；未启用的旧探针不包装 Markdown 元素。

Markdown 计时包括该元素的后代，不能把它和外层阶段耗时相加。request_layout 包含组件准备与布局请求，**不等于独立 Markdown 解析时间，也不包括其后的完整 Taffy 求解**；后台解析、测试调度及其余窗口内容仍包含在外层耗时中。prepaint / paint 是测试平台 CPU 工作，不是 GPU 呈现。

每次运行创建独立 `artifacts/desktop-chat-profile/<时间戳-随机后缀>/`，保存二进制、生成包、逐阶段 JSONL、日志和带源码身份的 `run-results.json`。缺失、乱序、错误计数、输入错误、VM 丢失或空绘制不能发布成功汇总；失败进程保留开始记录和日志。

## 重现

```powershell
python -m unittest discover -s tools/shell-prototype -p 'test_*.py' -v
python tools/shell-prototype/verify-desktop.py --phase chat-profile --samples 60 --warmup 10 --repeats 3
python tools/shell-prototype/verify-desktop.py --phase chat-profile --profile release --samples 60 --warmup 10 --repeats 3
```

## 本轮结果

固定源码 dev 检查已通过：rustfmt、all-targets + ui-tests check、Clippy `-D warnings`；Python 27 项、插件 46 项（5 项按需探针 ignored）、更新 7 项、聊天 7 项、设置 12 项通过。

dev 分项采样为每场景 60 样本、10 预热、3 轮独立进程，共 9 进程和 7,560 个正式字符样本。下表是每进程 p95 再取三轮中位数，括号是三轮 p95 的最小—最大值；单位 ms。字符指标在每进程中聚合 14 个位置共 840 个正式样本，不是 14 个 p95 的平均。

| 场景 | 字符输入 p95 | 聊天行同步 p95 | notify + draw p95 | Markdown 三阶段累计占字符总耗时 |
| --- | ---: | ---: | ---: | ---: |

| baseline | 17.323（14.663–18.021） | 0.019 | 32.728 | 44.1% |
| registered | 23.112（15.643–26.249） | 0.022 | 47.309 | 41.6% |
| active | 18.743（17.261–27.139） | 0.018 | 37.711 | 37.6% |

所有正式字符样本均为 1 次根 render、1 次聊天行构造和 1 次 Markdown 各阶段调用；notify + draw 为 2 次根 render。行构造 p95 三轮中位数约 0.054–0.066ms；Markdown request_layout 为 4.27–5.20ms、prepaint 为 1.53–2.27ms、paint 为 2.19–2.94ms。这些分项 p95 不能直接相加。统计占比用累计时间相除，分别在三轮进程内计算后取中位数。

本轮 registered 明显高于 active，且轮间存在波动。没有控制其他应用负载，没有统计置信区间，不能把场景差值当作插件可归属开销。数据说明重复 Markdown 元素工作值得优先优化，但尚未独立区分后台解析、完整布局求解和其他宿主绘制；不据此宣称已经找到唯一瓶颈或获得性能收益。

## 原 stress / diagnostics 对照

dev stress 六场景 × 三轮共 18 进程全部有效；每项仍为 60 正式样本、10 预热，启停与空闲分别每进程 20 轮。下表保持原驱动的整段输入口径（单位 ms）：

| 场景 | 整段编辑 + draw p95 | notify + draw p95 |
| --- | ---: | ---: |
| baseline | 241.975 | 32.245 |
| registered | 344.586 | 38.439 |
| active | 417.528 | 56.056 |
| mixed | 361.307 | 50.541 |
| lifecycle | 326.588 | 40.212 |
| idle | 330.080 | 42.295 |

active 相对本轮 baseline 的整段编辑增量 175.554ms（预算 24.197ms），notify 增量 23.812ms（预算 3.224ms），仍超出参考预算。编译不与测量并行，但没有控制其他应用负载；不能与旧版本/旧工具链作优化回归结论。

三轮启停合计 60 次，逐次确认 3 个 VM 释放、runtime / instance 清零。lifecycle 私有提交末轮减首轮中位数 8.922MiB，idle 为 14.168MiB；窗口仍只有 20 轮，`leak_verdict` 保持 `not_evaluated`。

dev / release diagnostics 各两驱动 × 三轮、各 6 进程全部有效，每 profile 840 命令含 120 预热，普通路径未复现超时。所有正式命令的 settle 阶段仍为 0 次根 render、0 次物化，成功命令收尾无冗余通知的修复持续成立。未复现不关闭历史故障。

主工作区在集成后通过 `cargo check --locked --all-targets --features ui-tests --jobs 2`，源码摘要前后相同。性能及 Shell 回归结果仍只对应隔离快照，主树并行 Git 功能没有纳入这些性能结果。

## Release 分项

release 插件 46 项、更新 7 项通过。分项同样完成 9 进程、7,560 个正式字符样本；下表统计方式与 dev 相同，单位 ms。

| 场景 | 字符输入 p95 | 聊天行同步 p95 | notify + draw p95 | Markdown 三阶段累计占字符总耗时 |
| --- | ---: | ---: | ---: | ---: |
| baseline | 15.809（14.580–16.413） | 0.016 | 30.497 | 43.9% |
| registered | 16.886（15.556–18.448） | 0.014 | 32.501 | 41.2% |
| active | 19.011（18.897–19.052） | 0.015 | 35.825 | 37.8% |

全部正式字符样本为每字符 1 次根 render。行构造 p95 中位数 0.047–0.050ms，Markdown request_layout 为 4.12–4.33ms、prepaint 为 1.56–1.65ms、paint 为 2.08–2.19ms。上述分项不包括完整 Taffy 求解，不能称为独立解析耗时。

## Release stress 与结论

release stress 完成 18/18 个进程，参数与 dev 相同，原驱动数据如下（每进程 p95 的三轮中位数，ms）：

| 场景 | 整段编辑 + draw p95 | notify + draw p95 |
| --- | ---: | ---: |
| baseline | 204.077 | 31.562 |
| registered | 230.351 | 30.934 |
| active | 268.165 | 32.212 |
| mixed | 259.543 | 31.351 |
| lifecycle | 215.233 | 28.820 |
| idle | 210.729 | 40.111 |

active 整段编辑增量 **64.088ms**，超过 **20.408ms** 参考预算；notify 增量 **0.650ms**，在 **3.156ms** 参考预算内。registered / mixed 的编辑增量也超过参考预算。**G5 生产准入仍未通过。**

release lifecycle 私有提交末轮减首轮中位数 9.734MiB，idle 为 14.852MiB。dev/release 合计 120 次启停、120 次空闲对照，240 行曲线保留在 `lifecycle-memory.csv`。启停每次 3 个 VM 弱引用均释放；这不能替代长时间进程内存泄漏分析。

两 profile 合计 18 个 chat-profile、12 个 diagnostics、36 个 stress 测量进程全部有效，新增字符样本共 15,120 个。没有调整运行时限额，没有复现普通路径截止时间故障，也没有关闭历史故障。当前不冻结 P2 契约，不进入 P3 安装管理。

## 后续定位

`src/app/shell.rs` 的宿主 render 每次执行 `self.timeline.update(... cx.notify())`，聊天行在输入字符时重复构造。下一步用明确失效条件验证是否可复用未变化的聊天视图，同时覆盖流式文本、主题/字体、消息操作、选择及滚动；另为后台 Markdown 解析和 Taffy 求解增加独立证据。当前元素阶段计时仅缩小范围，不能替代这些分解，也不能据现有数据直接合入缓存优化。

## 证据身份与范围

本轮归档：`tools/shell-prototype/artifacts/g5-chat-profile-20261106T222528/`。主工作区保存非二进制副本，二进制保留在隔离工作树 `C:/Users/Administrator/.codex/worktrees/4799/conduit` 的原始测量目录。

| 身份 | SHA-256 |
| --- | --- |
| 固定隔离源码 | `40953bd3ca4e3c1ef58066e60ceb7238d1aeda44ed699ac31de13d33581505a1` |
| dev chat-profile 二进制 | `b4b9f08f13880c526939ef879b42c7b20c4ddf7240ccb6dc40a98b0cf28b956d` |
| dev diagnostics / stress 二进制 | `cb0bf227d78c3129749637d118714a419332b7bc63997e6b7aba6de08cfde687` |
| release 三阶段二进制 | `036b859cd3bc80760d1450aeb4d0be63ac14e13b9cde183428b9f1b0b3c0b309` |
| 生成上游源码 | `a366593fbca9ee2b08abd4c81dbda79403344def2a1b554ea8bba990c57763ff` |

dev chat-profile 后构建日志记录一次重新链接，后续 diagnostics / stress 二进制哈希不同；源码摘要前后一致，分别保留二进制身份，不将其称为同一二进制对照。

三项资源保护探针均通过且各实际执行 1 项：终止/重建恢复、连续短调用累计预算、原生工作返回后检查。原生调用不可抢占的边界不变。

归档的 `results.json` 记录每步实际命令和退出码；`raw/` 保存生成包、JSON/JSONL、进程日志和独立报告；`chat-summary.json` 与 `lifecycle-memory.csv` 为派生汇总。`snapshot.patch`、`snapshot-new-files/` 保存本轮可重建实现，`identities.json` / `export-manifest.json` 记录报告与导出文件身份。`earlier-attempt/` 保留初期焦点失败和源码变化时被拒绝的检查，不能计入最终成功。

历史证据校验仅针对当前机器主工作区实际存在的 19 个非二进制文件，前后 SHA-256 相同；不是重跑上轮文档所记 3,216 文件的哈希审计。当前工具链、应用版本及背景负载与上轮不同，本轮没有宣称跨轮性能收益。没有运行原生 GPU / Windows IME 或 Fast 成品验收。
