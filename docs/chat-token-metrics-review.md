**Chat token 统计审查与缓存显示调研**

审查日期：2026-10-26。审查当前工作区中的实现；未修改业务代码。范围包括 Codex app-server、Claude stream-json、Usage 状态传递、输入区统计展示和模型上下文兜底。

结论：速度、用量和上下文三个指标都存在实际计算问题。当前还没有统一“单次模型请求 / 用户的一轮任务 / 整个会话”的统计范围，不能直接在现有字段上增加缓存百分比。

**发现，按优先级排序**

1. **P1：tok/s 的分子包含输入 token，显示的不是输出生成速度。**

   位置：[sessions.rs:436](../src/app/sessions.rs#L436)、[state.rs:209](../src/app/state.rs#L209)、[runtime.rs:530](../src/app/runtime.rs#L530)。

   公式是 `(usage.total_tokens - turn_tokens_at_start) / turn_started.elapsed()`。Codex 的输入包含反复发送的上下文和缓存，因此一次请求的 100,000 输入、1,000 输出，耗时 10 秒，会显示 10,100 tok/s，而输出除以同一耗时只有 100 tok/s。导入会话的 Usage 初始为零，第一次续聊还会把已有会话累计量纳入分子。

   耗时包含启动、工具和审批等待。所谓暂停实际上是“距离最后一次 usage 超过 10 秒就隐藏”，不是扣除等待；恢复更新后又把等待时间计入。总量下降时重新设置基线也无法解决统计范围混用。

   建议：有可靠输出计数和生成时间时才显示生成速度；否则显示明确标注的“本轮平均输出速度”，或显示 `—`。输入、缓存、历史累计量都不能进入分子。不要从文本字符数估计 token。Codex 若只在请求完成时报告 usage，不能把它包装成连续实时采样。

2. **P1：Codex 输入 token 把缓存重复计算了一次。**

   位置：[protocol.rs:520](../src/protocol.rs#L520)、[sessions.rs:593](../src/app/sessions.rs#L593)。

   Codex 原始 `inputTokens` 已经是全部输入，`cachedInputTokens` 是其中的缓存部分，但展示层再次计算 `input_tokens + cached_input_tokens`。现有夹具的输入 5,389、缓存 5,120，最终会显示 10,509，正确输入应为 5,389。缓存是输入的拆分，不能再次加到总量。

3. **P1：Codex 上下文占用使用会话累计总量，会随请求次数持续虚高。**

   位置：[protocol.rs:517](../src/protocol.rs#L517)、[sessions.rs:500](../src/app/sessions.rs#L500)、[sessions.rs:613](../src/app/sessions.rs#L613)。

   协议同时提供 `tokenUsage.total` 和 `tokenUsage.last`，实现丢弃 `last`，再以 `total.totalTokens / modelContextWindow` 画环。累计调用消耗不是当前上下文。比如累计 300K、最近请求 40K、窗口 200K，当前会显示 300K / 200K 且满环，最近一次请求规模只相当于约 20%。

   建议：单独保存最近请求的 `last` 和累计 `total`。上下文显示使用最近请求的上下文规模，并明确它是最近报告值；它不能精确预测下一次请求新增工具输出后的占用。压缩后允许上下文下降，累计量继续独立统计。不要用累计量推导压缩是否发生。

4. **P1：Claude 用量遗漏缓存、覆盖多次请求，且没有读取最终输出统计。**

   位置：[protocol.rs:712](../src/protocol.rs#L712)、[protocol.rs:816](../src/protocol.rs#L816)、[runtime.rs:536](../src/app/runtime.rs#L536)。

   Claude 的总输入是 `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` [3]。当前 `total_tokens` 漏掉 cache read；界面输入又漏掉 cache creation。现有测试的 34,441 输入、256 缓存读取、153 输出，总量应为 34,850，测试却期待 34,594。

   每个 assistant 消息都直接覆盖同一个 Usage，因此一轮中的工具调用前后多个模型请求没有累计。与此同时，assistant 消息上的输出计数可能只是 message_start 时的占位值；实现忽略 `message_delta.usage` 和 `result.usage`，最终输出也不会校正 [4]。

   建议：按模型响应 ID 去重，合并同一响应的流式 usage 更新；累积不同响应的输入和缓存。响应结束时取最终计数，一轮结束用 `result.usage` 校正主代理本轮量。若使用 `result.modelUsage` 展示整个会话/代理树，必须另设范围：它可能包含恢复的历史和子代理，不能逐次相加，也不能与主代理本轮 usage 混在一起 [4][5]。

5. **P2：模型 ID 的上下文后缀解析对承诺支持的格式全部失效。**

   位置：[models.rs:134](../src/models.rs#L134)。

   `rsplit_once('[')` 对 `model[200K]` 得到 `200K]`，接下来的 `strip_suffix('K')` 必然失败；`[1M]` 也没有 M 的处理。窗口未由 CLI/目录报告时，即使模型名有后缀，统计环仍不会显示。

   建议：先解析完整的方括号后缀，再分别支持 K/M，并覆盖 `[200K]`、`[1M]`、非法格式和溢出。

6. **P2：CCS 上下文值没有进入统计环的兜底路径。**

   位置：[models.rs:58](../src/models.rs#L58)、[sessions.rs:491](../src/app/sessions.rs#L491)。

   CCS 读取时保留了 `context_window`，但 `with_synced` 使用 `push_model` 丢弃该值；更直接的问题是统计环只查询 CLI 的 `self.catalogs`，根本没有查询当前 CCS Provider。选择一个只在 CCS 中提供上下文长度的模型，CLI 又未报告窗口时，兜底会失败。

   建议：模型选择器和统计展示共享同一个合并目录；合并时传递 `context_window`。窗口估计必须对应产生当前统计的模型，而不是用户刚为下一轮选中的模型。

**缓存命中显示：可行，建议先显示最近一次主代理请求**

这里应定义为“缓存读取 token 占全部输入 token 的比例”。它不是请求命中次数占比，也不是费用节省比例。缓存写入属于输入，但不计入命中分子。

| 引擎 | 分子 | 分母 | 推荐数据范围 |
| --- | --- | --- | --- |
| Codex | `last.cachedInputTokens` | `last.inputTokens` | 最近一次主代理模型请求 |
| Claude | `cache_read_input_tokens` | `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` | 最近一个主代理响应 ID |

Codex 本机 CLI 0.163.0 生成的 schema 已确认同时包含 `last`、`total`、`cachedInputTokens`、`cacheWriteInputTokens` 和 `modelContextWindow`，无需额外模型请求即可接入 [1]。OpenAI 的缓存字段是全部输入中的拆分 [2]；Claude 的三个输入字段是互斥部分，需先求和 [3]。

建议紧凑显示 `缓存 95.0%`，悬浮显示“最近一次主代理请求；缓存读取 5,120 / 输入 5,389 token”，并展示可获得的缓存写入量。没有 usage、缺少缓存字段、分母为零时显示 `—`；只有字段明确报告 0 且分母大于零时才显示 `0%`。现在协议的缺失字段会被 `unwrap_or(0)` 吞掉，接入时需要保留“缺失”和“真实零”的区别。

本轮和会话级指标可以后续加入，但必须标范围：

- 本轮比例使用 `sum(缓存读取) / sum(全部输入)`，不能对各次百分比求平均。
- Codex 会话比例可用 `total.cachedInputTokens / total.inputTokens`；本轮比例需要可靠的轮前基线，导入会话缺少基线时不要伪造。
- Claude 主代理本轮量可以采用 `result.usage`；含子代理的会话量可采用 `modelUsage`，其缓存字段也要加入分母，并处理恢复历史的累计口径 [4][5]。
- 上下文中命中缓存的 token 仍占上下文窗口，不能从上下文长度中扣除。相同 token 重复请求所产生的累计消耗，也不代表当前上下文在增长。

当前 composer 把引擎、权限、所有统计、模型和发送控制放在一行。添加缓存项前应检查窄窗布局，必要时只保留紧凑百分比并把详细拆分放进悬浮说明。

**建议统一的数据语义**

- `input_tokens`：始终指全部输入，缓存读取/写入是其子集；Claude 适配层负责归一化。
- 最近请求、本轮任务、会话累计分别保存；上下文用最近请求快照，累计用量用于消耗展示。
- 缓存读取、缓存写入、窗口、输出速度保留可用性信息；不把未知值当 0。
- 输出速度保存独立的输出计数和计时范围，不复用上下文或累计总 token。
- 明确主代理和代理树范围；不将不同来源的累计快照重复相加。

**验证与后续回归点**

本次运行 `cargo test --locked`：114 通过，11 忽略，0 失败。未运行真实模型请求。现有两项 usage 测试仅验证字段转换：Codex 把 last/total 设为相同值，Claude 没有覆盖缓存写入、重复 ID、多模型请求或最终 result 校正，因此不能证明显示统计正确。

修复时建议覆盖：缓存占比接近 100%；Codex last 与 total 不同；首次续聊导入会话；上下文压缩；Claude 同 ID 多条 assistant 和流式更新去重；result 校正输出；缓存写入非零；混合子代理；缺失字段和真实零；不同窗口的模型切换；后缀和 CCS 兜底；等待审批后继续生成。

**一手资料（2026-10-26 查阅）**

1. OpenAI app-server 文档：https://developers.openai.com/codex/app-server 。此外运行本机 `codex --version` 与 `codex app-server generate-ts --out <临时目录>`，检查 0.163.0 版本 `v2/ThreadTokenUsage.ts` 和 `v2/TokenUsageBreakdown.ts`。未启动模型会话。
2. OpenAI Prompt caching：https://developers.openai.com/api/docs/guides/prompt-caching 。核对输入总量与 cached/cache write 拆分。
3. Anthropic Prompt caching：https://platform.claude.com/docs/en/build-with-claude/prompt-caching 。核对总输入公式。
4. Anthropic Agent SDK cost tracking：https://platform.claude.com/docs/en/agent-sdk/cost-tracking 。核对响应 ID 去重、assistant 输出占位值、message_delta 和 result 的范围。
5. Anthropic Agent SDK TypeScript：https://platform.claude.com/docs/en/agent-sdk/typescript 。核对 SDKResultMessage、ModelUsage、contextWindow 和恢复会话的累计范围。
6. Anthropic Streaming：https://platform.claude.com/docs/en/build-with-claude/streaming 。核对 message_delta.usage 是同一响应的累计值，不应逐事件直接累加。
