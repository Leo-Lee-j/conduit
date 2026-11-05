# 性能设计审查

审查日期：2026-11-05。修复更新：2026-11-06。原审查覆盖聊天列表、流式消费、全文搜索、正文按需加载与持久化；原始问题与基线测量保留在下方。

**修复状态：2 项 P1、2 项 P2 均已实现并通过回归。** 本轮按原报告顺序修复，未创建提交。

## 修复结果（2026-11-06）

| 项目 | 实现 | 验证结果 |
| --- | --- | --- |
| 1. 折叠组平方开销 | [展示行索引](../src/app/chat/rows.rs) 将折叠组映射为一条真实列表行；按消息身份缓存分类，映射滚动、跳转、重测与 outline；折叠分支计入渲染统计 | 400 条工具与两条普通消息形成 3 行；增加后续一轮后为 5 行，真实 GPUI 挂载不超过 5 行；展开和跳转保留锚点；Assistant Delta 只重新分类该消息 |
| 2. 重绘解析与晚截断 | [工具分析缓存](../src/app/tool_analysis.rs) 按消息身份复用，统计按工具贡献增减；大于 8 KiB 的输入/结果由单个后台 worker 解析；[预览收集](../src/app/tools.rs) 在分配前限制为 400 行、64 KiB，完整行数单独计入 | 80 个各 8,000 行的 Write 得到 640,000 行统计；20 次 Assistant Delta 不增加解析次数且复用同一统计对象；单工具变更只解析该版本；超长单行也受字节限制 |
| 3. 搜索无进展 | [搜索调度](../src/app/search.rs) 在 debounce 后一起获取快照/版本，同一查询的内容更新不取消在途任务；[结果缓存](../src/app/search_worker.rs) 使用消息身份或冷正文引用，只补搜变化会话，发布时过滤删除/替换正文 | 65 个会话初次扫描后，12 次单会话变化仅新增 12 次扫描（总计 77）；即使该会话在扫描期间继续更新，其余 64 个会话仍能发布；旧片段不发布 |
| 4. 全正文写放大 | [分块存储](../src/transcript/chunks.rs) 每页 64 条消息，长文本每块 64 KiB；复用已保存消息、页面和内容块，所有子块 fsync 后才发布根引用和 manifest；[队列](../src/persistence.rs) 保留确认屏障并限制额外版本占用 | 8 MiB 正文、12 × 32 字节追加：新增落盘 **110,712 字节（约 108 KiB）**，原基线约 **96 MiB**；兼容读取旧正文、分页缩短、同位置修改、损坏块拒读、失败 manifest 和在途读取 GC 保护均通过 |

存储写入 manifest **v4**，读取兼容 v1–v3；旧 v3 整体 JSON 正文仍可直接加载，下次修改时转换。版本升级让旧程序明确拒绝打开新格式，避免旧单文件 GC 错删分页。主 manifest 原子替换和发送前持久化确认语义保留。GC 同时追踪已提交 manifest、当前回执和在途引用，缺失/损坏根时停止删除。

工具预览缓存目标上限为 1,024 项 / 32 MiB，后台每批至多 8 个工具，待解析队列最多 128 项，并在累计输入达到 16 MiB 后暂停接收后续项；单个超大输入仍允许取得进展。当前轮只保留轻量统计贡献，不随预览淘汰丢失计数。复制完整输入/结果延迟到点击时处理。

保存队列按共享消息身份计算保留量：最大一份快照之外的额外消息版本限制为 128 MiB；单份大库仍可保存，正在写入的快照另计，确认快照不会被合并掉。受控慢 writer 回归验证共享快照不重复计费、超预算拒绝新确认请求、已有确认仍逐个完成；测试输出待保存消息字节及确认耗时。该受控耗时不是生产磁盘延迟指标。

扩展回归 **288 passed，0 failed，0 ignored**，覆盖全部 `app::` 测试以及正文存储、保存回执、事件队列和 Store。最终的小范围缓存/预算优化后再跑核心 **30 项，全通过**。默认及 `ui-tests` 两种配置的 Clippy 均通过，格式检查通过。

默认测试另通过 **367 项**（库 33、主程序 333、发布工具 1），0 失败，16 项原有 ignored；显式跳过 1 项 `models::opencode::tests::native_cache_survives_restart_expires_daily_and_manual_refresh_bypasses_it`，该已有测试在 Windows 会触发无效可执行文件对话框。本轮未运行实际 CLI/真实模型请求或 Shell 桌面烟测。

```powershell
cargo test --offline --locked --features ui-tests --bin conduit -- app:: transcript::tests persistence::tests event_queue::tests store:: --test-threads=1
cargo clippy --offline --locked --all-targets -- -D warnings
cargo clippy --offline --locked --all-targets --features ui-tests -- -D warnings
cargo fmt --all --check
cargo test --offline --locked -- --test-threads=1 --skip models::opencode::tests::native_cache_survives_restart_expires_daily_and_manual_refresh_bypasses_it
# 复现写入字节、受控 writer 队列占用和确认时序
cargo test --offline --locked --bin conduit -- transcript::tests::streaming_large_message persistence::tests::slow_writer_bounds --test-threads=1 --nocapture
```

仍需区分的边界：变更中的单条长文本仍需遍历并计算块哈希；根页目录及主 manifest 仍会重写，所以不是严格 O(新增字节) CPU。新查询首次仍扫描整库，持续变化会话的结果可能等待其下一次稳定快照，但不会阻止稳定会话发布。预览逐行模型有界，完整 JSON 的首次解码仍可能随输入大小分配，只是移出 UI 线程。事件应用的 3 ms 总预算、真实 FPS/输入延迟、物理冷盘与系统级磁盘写放大不在本次验收内。

以下第 1–4 节描述的是**修复前**实现及测量，不代表当前未修复问题。`target/review-performance-20261105/` 是当时抽取的基线探针；修改后的函数签名已变化，不应直接重新抽取后把结果当作同口径基线。

## 1. [P1] 折叠工具组仍逐条占用列表行，分组计算随组长度平方增长

位置：[chat.rs:797](../src/app/chat.rs#L797)、[chat.rs:810](../src/app/chat.rs#L810)、[tools.rs:7](../src/app/tools.rs#L7)、[tools.rs:70](../src/app/tools.rs#L70)、[shell.rs:192](../src/app/shell.rs#L192)。

列表项数量使用原始消息数量。`render_message` 对每条工具消息调用 `contiguous_tool_group`，分别向前、向后寻找整个组，再扫描组内运行状态；折叠组中的非首行直到完成这些工作后才返回零高度元素。`is_groupable_activity → is_compact_tool → is_tool_edit` 还会为非编辑工具解析 `detail` JSON，判断是否为 OpenCode 编辑。

零高度元素不能让列表按视口高度提前停止：锁定的 `gpui-pre 0.3.9` 在 `src/elements/list.rs:1064–1118` 按累计高度遍历、渲染，向上补齐视口的路径在 `1133–1140` 也逐项调用渲染函数。因此，短回复附近的一个长折叠组即使看起来只有一行，仍可能让整个组进入测量；每行又扫描整个组，形成 O(k²) 工作。滚动、切换会话以及工具组折叠后均可触发。

隔离探针使用每条 1,057 字节的非编辑工具 JSON，逐行执行实际分组、运行状态与后续回复判断逻辑，结果如下。这里未计入元素创建、布局、组标题渲染或 GPU 绘制。

| 工具数 | 单次遍历耗时中位数 | 分配次数 | 累计分配量 |
| ---: | ---: | ---: | ---: |
| 100 | 3.60 ms | 50,000 | 15.98 MiB |
| 200 | 15.39 ms | 200,000 | 63.93 MiB |
| 400 | 55.82 ms | 800,000 | 255.74 MiB |

建议：建立独立的展示行索引，让折叠工具组只贡献一条真实列表行；展开时再映射其子项。按消息身份或内容版本缓存工具分类，按结构变化维护分组边界与运行状态，避免在每行渲染中重新解析相同 JSON。保留稳定消息键与滚动锚点。

验证应统计实际列表回调次数，并覆盖数百条连续工具、空 Assistant 占位、折叠/展开和短回复。当前一万条消息虚拟化测试使用 User/Notice 交替数据，不覆盖此问题。此外，折叠分支提前返回，绕过 [chat.rs:1631](../src/app/chat.rs#L1631) 的 `record_message_render`，现有“构建消息数/耗时”也会漏记这部分开销。

## 2. [P1] 每次重绘重新解析本轮全部工具结果，展示截断发生在大部分分配之后

位置：[chat.rs:192](../src/app/chat.rs#L192)、[state.rs:207](../src/app/state.rs#L207)、[tools.rs:520](../src/app/tools.rs#L520)、[tools.rs:299](../src/app/tools.rs#L299)。

Composer 在构建时调用 `turn_stats_tags → turn_file_stats`，后者遍历本轮全部工具并调用 `edit_tool_diffs`，重新解析 JSON、分配行字符串、生成统一 Diff 文本，只为显示文件数和增删行数。即使工具卡已经折叠，或者正在更新的只是 Assistant 文字，已完成工具的统计也全部重算。

`MAX_EDIT_DIFF_LINES = 400` 目前主要限制结果：Claude Write/Edit/MultiEdit 和 Codex 的 content 回退路径先收集所有行，再由 `EditFileDiff::new` 截断。因此，长文件输入的解析和临时分配没有被 400 行上限约束。已有 `CachedDiff` 缓存的是后续 DiffState，也不能避免统计路径上的这些工作。

探针使用每次约 281 KiB、8,000 行的 Write 输入，反复调用原始 `turn_file_stats`：

| 本轮工具数 | 重算耗时中位数 | 单次累计分配量 |
| ---: | ---: | ---: |
| 10 | 9.39 ms | 21.28 MiB |
| 40 | 24.69 ms | 85.11 MiB |
| 80 | 37.11 ms | 170.21 MiB |

建议：将工具解析结果与轻量统计缓存到消息版本，单个工具更新时仅更新其统计贡献；Assistant Delta 不应使已完成工具统计失效。统计计数与展示用 patch 分开，截断应在创建逐行字符串之前执行；完整计数可扫描原始行而不分配完整行模型。大负载的首次解析放到后台，并验证消息版本后应用。

验证应断言：连续 Assistant Delta 不再调用已完成工具的解析器；更新一条工具只重算该条；超长 Write 的展示对象分配量受展示上限约束。完整统计不能因预览截断而静默少计。

## 3. [P2] 全局 revision 让流式更新反复作废整库搜索，查询可能持续无结果

位置：[search.rs:139](../src/app/search.rs#L139)、[search.rs:170](../src/app/search.rs#L170)、[runtime.rs:604](../src/app/runtime.rs#L604)、[search_worker.rs:153](../src/app/search_worker.rs#L153)、[session_controller.rs:55](../src/app/session_controller.rs#L55)。

`refresh_search` 先记录全局 revision，等待 150 ms debounce 后才复制工作区快照；返回时必须与全局 revision 完全相等。任何会话的 `session_mut` 都推进这个版本，普通持久化也会推进它。流式消息只要在 debounce 或搜索执行期间到达，本次结果就无法发布，即使匹配的其他会话根本没有变化。

轮询在版本变化且距请求至少 500 ms 时重新发起整库搜索，取消上一任务。冷正文每次都重新读取、校验 SHA-256、反序列化并全文匹配，没有按会话正文 hash 复用匹配结果。查询超过 500 ms 时，持续流式更新会不断取消尚未完成的扫描；即使扫描更快，执行期间的一次更新仍会使结果被丢弃。用户可一直看到旧结果或“没有匹配”，并伴随重复 CPU/I/O 工作。

使用真实 `collect_cancellable` 与 `transcript::read`，查询不存在的词，正文均为冷会话，每个 512 KiB；磁盘读取已预热：

| 冷会话数 | 正文合计 | 查询耗时中位数 |
| ---: | ---: | ---: |
| 32 | 16 MiB | 156.05 ms |
| 64 | 32 MiB | 301.66 ms |
| 128 | 64 MiB | 565.88 ms |

建议：debounce 之后同时捕获快照与对应版本。将“用户换查询”的 generation 与“部分会话更新”的失效分开；用 `(query, session_id, body_hash/content_revision)` 缓存结果，只重搜变化会话。让一次稳定快照扫描能够完成，再对变化部分补查；发布前重新过滤删除或替换的会话，避免为了防止旧结果而无限重启全库工作。

现有 `search_discards_results_after_transcript_changes` 验证了丢弃过期结果，但没有验证持续更新下的进展。应补充“一个会话持续产生 Delta，其他冷会话仍能在有界时间内被搜到”的回归。

## 4. [P2] 每秒保存仍重写变化会话的完整正文，内容寻址放大了小更新的写入量

位置：[stream_events.rs:536](../src/app/stream_events.rs#L536)、[runtime.rs:666](../src/app/runtime.rs#L666)、[transcript.rs:209](../src/transcript.rs#L209)、[transcript.rs:231](../src/transcript.rs#L231)、[transcript.rs:47](../src/transcript.rs#L47)。

后台保存已避免 UI 同步刷盘，也会复用未变化的正文引用。但只要活跃会话中的一个消息变化，保存就序列化整个 `session.messages`、计算整份 SHA-256，并写入新的完整 blob 后 `sync_all`。流式更新约每秒触发保存；多个运行会话会叠加同样工作。旧版本回收约每 30 秒检查一次，因此还会出现多份完整正文暂存。

探针对一个 8 MiB 正文连续保存 12 次，每次只追加 32 字节：384 字节的新内容导致写出 **12 个 JSON、约 96 MiB**。该数字直接统计生成文件大小，不含主 manifest、文件系统及同步开销；12 次在 30 秒回收周期内完成。移到后台缓解的是 UI 阻塞，没有改变写放大。慢盘时还会拖延同一 writer 后方的发送前确认回执。

建议：保留发送前的持久化确认语义，改为可恢复的追加日志、按消息/块保存，定期合并 checkpoint；让持久化成本主要随新增内容增长。将元数据保存、流式正文增量和强制确认区分调度，并设置后台写入与暂存字节预算。

验证应同时记录逻辑变更字节、物理写入字节、待保存字节数和确认延迟；覆盖长会话、并发运行及受控慢 writer。仅检查快照合并和最终内容正确不足以证明保存负载可控。

## 其他设计边界

- **事件时间预算尚未覆盖应用阶段。** [stream_events.rs:28](../src/app/stream_events.rs#L28) 的 3 ms 检查只在取队列时执行，之后 [stream_events.rs:61](../src/app/stream_events.rs#L61) 会同步应用所有已取出的事件。512 KiB 也在出队前检查累计值，单条大事件可越过该阈值。当前可以称为有界出队，不能作为 UI 时间上限。后续应让应用与失效处理共享预算，重解析异步化，同时保留事件顺序和终态可靠送达。这是已有审查记录承认的边界，本轮不重复算作新的独立缺陷。
- **性能指标还不足以给出完整验收结论。** 面板明确只统计主进程及视图构建，尚缺完整帧/输入延迟、事件应用时间、队列最老事件等待、搜索完成/取消次数、正文读写字节和保存确认延迟。尤其应修正第 1 项提前返回漏计，再制定可验证的性能目标。
- **保留有效保护。** 相邻 Delta 合并、事件容量与终态预留、后台持久化及独立发送回执、正文读取并发限制与 LRU、侧栏摘要缓存和搜索预览上限均有价值。8 个/64 MiB 正文缓存是可淘汰正文的策略，选中、运行、排队和未保存会话会被固定，不能将它描述成整个进程内存硬上限。插件实例有容量限制，Shell 上下文刷新也先检查数据相等，本轮未据此报告额外性能缺陷。

## 原始审查验证与可复现性

定向回归：**25 passed，0 failed，0 ignored**，覆盖搜索、侧栏流式缓存、正文加载/淘汰、普通聊天虚拟化、队列限额、保存回执及正文存储。

```powershell
cargo test --offline --locked --features ui-tests --bin conduit -- app::search_worker::tests app::review_tests::search_discards_results_after_transcript_changes app::sidebar::tests::streaming_text_reuses_sidebar_and_order_but_title_and_archive_refresh app::transcripts::tests transcript::tests event_queue::tests persistence::tests app::ui_tests::long_chat_virtualizes_real_rows_and_keeps_history_jump_on_append --test-threads=1
```

隔离探针保存在 `target/review-performance-20261105/`，属于本地验证产物，不进入版本控制：

- `build_probe.py`：抽取当前源文件中的分组、Diff/统计和搜索算法，并复制正文存储代码。
- `src/main.rs`：构造合成输入，执行测量；没有启动实际 CLI 或调用模型。
- `source-sha256.json`：记录所用六个源文件的 SHA-256；测量结束后核对，六个文件均未变化。

```powershell
python target/review-performance-20261105/build_probe.py
cargo run --offline --release --manifest-path target/review-performance-20261105/Cargo.toml
```

探针采用独立 release 构建及分配计数器，分组/搜索取 3 次测量中位数，统计取 5 次；计时前预热。为了不引入 GPUI，夹具使用简化的 Session/Workspace 元数据；无附件、无中断恢复，文件发布使用临时目录中的 rename。原始算法、ChatItem 共享数据结构、JSON/哈希和正文读写逻辑保持源代码行为。分配量是累计分配请求字节，**不是峰值或常驻内存**。第 4 项测量调用正文保存，不包含主 manifest 和生产调度。

这些结果验证局部成本和增长趋势，不是应用 FPS、真实用户延迟或完整发布验收。未运行全套回归，也未测实际 CLI、GPU 帧时间、磁盘冷缓存或 Shell 插件整体性能。工作区存在并行改动，本报告不把所有发现归因于当前未提交 diff。

原审查建议依次修复第 1、2、3、4 项；对应实现和回归结果现已记录在文首，失败回执、消息恢复与进程启动门闩继续保留。
