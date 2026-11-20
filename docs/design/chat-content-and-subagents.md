# 聊天正文与子代理 UI 实施方案

状态：第一至第四阶段已实现；收尾补齐子代理无障碍名称并修复 Enter 与输入区快捷键冲突，19 项相关回归、Clippy、全仓格式检查与非测试主程序构建通过；真实窗口验收待完成。设计依据为 2026-11-20 提供的三张 Claude 设计图。

后续运行时方案已独立落地为[子代理会话导航与独立交互设计](subagent-conversations.md)，当前为待实施设计稿；明确完整子会话、一键返回主会话、双方状态保留及定向发送/停止。本页的“待运行时能力”不表示该后续功能已经实现。

## 可行性与实现边界

现有 Rust / GPUI Kit 已具备异步 Markdown、可选择正文、代码复制、文件链接导航、子代理事件、时间戳和按轮次历史。保留 CLI 原文与现有解析链路，在原生组件上调整样式和交互，不引入 WebView。

| 设计内容 | 结论与落点 |
| --- | --- |
| 正文、标题、行内代码、表格 | 可实施；`app/typography.rs` 集中配置，沿用用户字体、字号、行高和主题 |
| 文件链接图标、文件与 URL 分别设置下划线 | 文件链接中等字重、静态不下划线、悬停下划线；URL 保留原默认下划线。第四阶段加入既有 FileText SVG 前置图标，装饰独立于复制文本与源位置 |
| 引用 3px、列表缩进与间距的逐项精调 | 第三阶段已扩展原生引用/列表样式，接入 3px 引用边框、随正文字号缩放的列表缩进及间距；保留原语义节点 |
| 代码块独立顶栏 | 第二阶段已统一普通/行号代码块的 32px 顶栏；语言在左、复制在右，正文沿用自动换行与独立选择，文件源码查看器保留自己的滚动策略 |
| 子代理状态、排序、计时、缺字段降级 | 可实施；应用已有九种状态，完整保留等待审批、等待输入、空闲、未知，不强行压成四种 |
| 全部成功后自动收起、手动展开、查看历史 | 可实施；以当前运行批次隔离折叠偏好，失败、停止、未知不冒充全部完成 |
| 子代理贴合输入框 | 可实施；共用外框与宽度，子代理区域设高度上限，列表与输入操作互不抢点击 |
| 展开详情、复制任务、产出 | 可实施；详情使用真实任务和结果，结果按 Markdown 渲染，已有文件链接可打开。没有产出文件元数据时不合成文件卡片 |
| 行内重试子代理 | 暂缓；当前多引擎运行时没有统一的定向重试命令。不可将主会话重新生成伪装成子代理重试 |
| 查看子代理完整对话 | 暂缓；当前保存的是任务元数据与最终结果，不是完整子会话。首批提供结果详情，不展示无效按钮 |

截图右侧的说明栏是设计注释，不加入产品界面。截图以蓝色强调，落地使用当前主题的语义色；用户选择的字体、字号、行高优先于图中的默认值。

当前聊天默认字号 15px、代码字号 13px 沿用；行高保留现有设置（默认 1.5），不强制改写已有用户偏好为图中的 1.75。子代理明细最多占 208px，其余内容内部滚动。

## 数据契约

- `Agent` 增加可选的短标题 `title` 和岗位 `role`，采用 serde 默认值兼容已有历史；`name` 继续表示原生名称，不将 Codex 昵称推断成岗位。
- Claude 的 `description` 用作短标题；`subagent_type` 用作岗位。第二阶段将主会话 `Agent` / 旧版 `Task` 工具输入按 tool-use ID 关联到任务；收到 `prompt` 后用于详情与复制，缺失时继续显示任务描述。字段沿用 8192 UTF-8 字节上限，超长输入不保证全文保留。
- Codex 的 `agentRole` 用作岗位，任务标题没有可信独立字段时取 prompt 首行并使用次级色；其余引擎按缺字段策略降级。
- 无标题依次取任务首个非空行、名称、ID；无岗位不占标签列。完整已有任务放详情并可复制。
- 状态只来自协议：`stopped` / `interrupted` 等明确取消映射为停止；断连仍是未知。显示文字与图标，不能只靠颜色。
- 排序为失败、停止、等待审批/输入、运行、空闲/未知、完成；同类按开始时间和原始顺序稳定排列。
- 运行计时使用开始时间与应用时钟，完成后固定；停止不显示耗时，缺时间不显示占位。
- 默认只看本轮，历史开关包含旧轮次；总数和状态摘要始终对应实际显示的记录。

## 实施阶段

### 第一阶段：已有事件驱动的可用版本

1. 扩展兼容的数据字段，接入 Claude 短标题与岗位、Codex 岗位；保留原有生命周期和计时。
2. 重做列表：主题色状态图标、岗位标签、截断标题、格式化计时、摘要和有上限的状态分段条。
3. 成功自动收起；手动展开与本轮/历史切换；详情显示已有任务、活动、Markdown 结果，并可复制任务。
4. 将列表整合进 composer 外框，限制列表高度，保持草稿、文件引用、IME、发送与停止行为。
5. 集中调整标题比例、段间距、行内代码边框与大小、表格边框/表头/单元格，并保持代码复制、语法高亮、行号、Mermaid 与文本选择。

### 第二阶段：渲染精度与协议能力

1. **已实现**：普通/行号代码块共用自定义渲染器与 32px 顶栏，无语言时标记 `text`。行号设置仅控制 gutter；复制开关只控制按钮。复制状态按消息 scope 与原始块起始偏移隔离。Mermaid 优先匹配，不作为普通代码接管。
2. **已实现**：Claude `Agent` / `Task` 输入与任务事件按精确 tool-use ID 关联。支持输入先到、任务先到和完成后补齐；重复快照不重复发布子代理；后续 description 只更新标题，不覆盖 prompt；新 run 清空旧 prompt 标记与任务内容。后台启动结果先到也沿用同一关联路径。
3. **第三阶段接入**：逐链接文本样式、列表/引用精调、表格首列字重与颜色；前置图标在第四阶段补齐，见下方范围说明。
4. **待运行时能力**：定向重试和完整子会话导航；须先实现目标子会话身份、权限审批、事件归属及持久化隔离。当前只展示真实已有的详情与结果。

任务输入缓存随 `Protocol` 生命周期隔离，最多 256 条；每条只保留有界的 prompt、description、subagent_type，工具 ID 最长 512 字节。空输入不会抹掉已有值，淘汰缓存不抹掉已关联的任务。只有真实任务生命周期产生子代理记录，单独看到工具输入不推测启动或成功。嵌套子会话消息和 text-only 请求继续按现有规则隔离。流式 JSON 片段不作为完整输入发布，等待结构化输入块或 assistant 快照。

### 第三阶段：可复现的原生渲染扩展

依赖基线仍为 GPUI Kit `11345ec002d9f182a266b5d5dd3a217090700a87`，应用版本保持 0.3.13。仓库内 `vendor/gpui-kit` 保存该 revision 的 base/component 两个 crate，通过根 `Cargo.toml` 的 `[patch]` 引用；Kit、assets、macros 和 GPUI 本体保持原来源与版本。干净检出即可 `cargo build --locked`，不改 Cargo 缓存，不使用机器相关绝对路径。为保留完整 crate 的可构建性，纳入 555 个原始文件（约 8.9 MB），实际渲染补丁涉及五个源码文件。

来源、许可证及修改清单见 `vendor/gpui-kit/CONDUIT.md`；`upstream-files.json` 记录原始文件哈希。`scripts/check-gpui-vendor.py` 校验未声明的源码变化、固定 revision、相对 patch 路径与锁文件，已接入核心检查。升级时必须同步替换两个 crate、重放补丁并复跑专项测试；上游具备等效能力后移除此副本。

| 接口 | 本批落地 | 保留边界 |
| --- | --- | --- |
| 原生链接 run 的样式解析 | 普通内联、混合行内代码、引用式链接共用 URL 样式；字号/字重参与表格文本测量；保持 URL、源区间、选择及点击 | 第三阶段仅提供文字样式；第四阶段另补不进入复制文本的前置图标，保留标签部分选择 |
| blockquote/list/list-item refinement | Base 可选 builder 与 Component 透传；应用配置引用边框、列表缩进和项间距；嵌套列表、编号、任务复选标记继续用原节点 | 默认未配置时保持上游行为；显式缩进对 marker、嵌套和续段统一生效 |
| 表格首列 | 按 body 列设置 TextStyleRefinement；首列 semibold 与正文色，表头保持独立样式；测量、缓存失效与绘制采用相同设置 | 本批是列级文字样式，不声称已提供通用逐行逐列 padding/背景回调 |

专项验证发现并修复了上游引用式链接定义未写回渲染上下文的问题。含引用定义的文档在流式追加时按完整原文重解析，确保后到定义和后到引用都可导航，普通文档继续使用尾块增量解析。超长且大量使用引用定义的流式文档可能增加解析工作量，后续可单独优化。

文件链接的外观分类与实际导航共享语法解析器，分类不依赖工作区、不访问文件系统；真正打开时仍使用当前工作区解析。用户字体与行高设置继续有效；正文、子代理结果和文件 Markdown 预览共享这些样式。

本批已验证：部分选择与源区间、拖选不触发跳转、点击保持原 URL、引用式链接、跨块复制、搜索源位置映射、表格流式追加、首列字体变化后的宽度缓存失效。

### 第四阶段：文件图标与视觉验收

1. **已接入**：文件链接复用应用现有 FileText SVG 与导航分类。图标为 0.9em、右侧间距 0.25em，使用链接色；外部 URL 不加图标。
2. **已接入**：原生 InlineFlow 为装饰预留宽度，将其与标签首个字素一同换行；长标签仍可逐字选择，后续行不重复图标。图标不增加原文、复制文本、搜索索引或编辑器位置中的字节；点击图标沿用原 URL，拖选不导航。
3. **已实现并通过专项**：表格内在宽度与绘制共用图标测量，布局缓存比较图标与引用目标。覆盖普通/引用式/行内代码/加粗链接、字号缩放、表格缓存、流式补齐定义、复制和中文拖选。从图标开始拖至空白也注册为正文选择起点，图标仅提供命中几何，不增加复制 run。
4. **待验收**：使用下方矩阵检查真实窗口，记录明暗主题、窄窗口和高 DPI 下的正文及子代理状态。本批再次调用 Computer Use 返回 `native pipe unavailable / os error 2`，尚未执行真实窗口操作。

第四阶段新增 `inline_flow.rs` 补丁，当前共有六个明确修改的依赖源码文件；来源校验和维护清单同步更新。应用版本仍为 0.3.13。

定向重试与完整子会话导航仍需独立的运行时能力，不纳入上述视觉收尾范围。

## 验证与验收

自动回归覆盖新增字段的旧数据兼容、序列化、协议字段来源、停止/未知的区别以及跨轮次状态。布局和颜色采用实际窗口检查，不锁定所有像素。

标准入口为 `scripts/check-core.ps1`：翻译检查、格式、Clippy、包含 ui-tests 的核心测试、生产主程序与助手构建。本次为避免共享目录中测试可执行文件锁冲突，拆开执行检查，并将主程序测试快照复制到 `target/chat-ui-phase2-tests.exe` 运行全量测试。使用 `target/` 下隔离数据，不启动真实模型任务。

人工验收：明暗主题；850×500、1100×700、1200×800、1600×1000；100%–200% DPI；长中文标题、无岗位/无标题/无时间、混合状态、全部完成后重新展开、历史切换、详情/复制、长结果内部滚动、窄窗口下输入可用、键盘焦点与屏幕阅读器，以及 Markdown 文件链接、代码复制/行号、Mermaid、流式输出和滚动锚点。未实际操作的项目保留待验收状态。

### 收尾操作清单

使用隔离数据目录启动实际待验收构建，按 [测试与验收](../testing.md) 记录可执行文件身份、窗口逻辑尺寸、系统 DPI、主题和用户字号。现有原生窗口控制通道仍返回 `native pipe unavailable / os error 2`，以下真实窗口项目均保持待验收。

| 操作 | 预期结果 | 当前证据 |
| --- | --- | --- |
| 打开 [Markdown 样例](fixtures/chat-content.md)，分别切换明暗主题、四档窗口尺寸与 100%/125%/150%/200% DPI | 标题、引用、列表、表格、代码与图标无重叠或裁切，输入框仍可用；系统 DPI 与用户字号分别记录 | 排版专项已覆盖；真实窗口待验收 |
| 点击文件文字和前置图标；从图标开始跨行拖选并复制；搜索表格中的中文 | 原文件和行号正确；拖选不跳转，复制不多图标或空格，搜索仍定位原文 | 自动回归已覆盖；真实鼠标待验收 |
| 开关代码行号后分别复制有语言/无语言代码块，再查看 Mermaid | 仅 gutter 改变；复制不包含顶栏、行号和相邻块；图表仍走 Mermaid | 自动回归已覆盖；主题下显示待验收 |
| 在含当前/历史子任务的会话中，用 Tab/Shift+Tab 到达总开关、历史、任务行和复制任务，使用 Enter/Space 操作 | 焦点可见，名称包含动作和目标；任务行包含状态及已有岗位；操作不会发送或清空草稿 | 新增键盘专项；Narrator 与真实 Tab 顺序待验收 |
| 混合失败、停止、等待与运行状态；展开长任务和结果；切换历史后回到当前 | 状态文字与排序正确，摘要对应显示范围，列表内部滚动，输入操作可用 | 事件与状态回归已覆盖；真实长列表待验收 |
| 最后一个任务成功后重新展开结果，再更新画面/切换会话；随后开始另一轮任务 | 全成功自动收起一次，手动重新展开被保留，新一轮恢复默认展开 | 状态回归已覆盖；真实焦点、滚动锚点待验收 |

子代理总开关与历史按钮已补充可访问名称；总开关和任务行的名称随展开状态提示“展开/收起”。现有 Button 提供键盘交互和焦点环。本次使用动作名称表达折叠操作，未将其声称为原生 ExpandCollapse 无障碍状态，也未代替 Narrator 实机验收。

键盘专项发现 composer 外框的 `ConduitComposer` 上下文会将子代理按钮的 Enter 匹配为发送快捷键。现已将该上下文移到 Textarea 容器，使发送、换行、命令补全和文件引用只在文本输入区匹配；子代理与输入框操作按钮沿用自身的 Enter/Space 激活。

子代理键盘回归入口为 `cargo test --locked --features ui-tests --bin conduit app::subagents:: -- --test-threads=1`，覆盖键盘打开/收起、历史切换、任务复制、全成功后重新展开，以及草稿和主会话未被发送。该测试使用离线夹具，不调用真实模型。此项与命令、文件引用、队列、快捷键和已有子代理回归合计 19 项通过，日志在 `target/chat-ui-acceptance-regressions.log`。收尾时全仓格式检查已通过，第四阶段末轮格式差异的历史记录保留。

完成上述真实窗口验收并处理发现的问题后，才进入发布检查。定向重试和完整子会话导航另立运行时方案，不作为这轮 UI 验收的前置条件。

包含本轮键盘修复的非测试程序为 `target/chat-ui-acceptance-production.exe`（dev profile）；构建日志为 `target/chat-ui-acceptance-build.log`，可执行文件和本轮源码哈希记录在 `target/chat-ui-acceptance-evidence.json`。旧的 phase4 可执行文件不包含本轮修复。

第三阶段的可打开样例位于 `docs/design/fixtures/chat-content.md`，涵盖普通/引用式/行内代码文件链接、长中文标签、嵌套引用与任务列表、窄表格和代码/图表分流。

真实窗口的视觉与鼠标操作验收仍待完成：本阶段复查 Computer Use 仍返回 `native pipe unavailable / os error 2`，不能用模拟窗口绘制测试替代人工验收。

第二阶段验证记录（2026-11-20）：

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| Claude task 协议 | 11 通过 | `target/chat-ui-phase2-protocol.log` |
| 相关协议、子代理、引用定位、滚动回归 | 119 通过，1 原有集成用例忽略 | `target/chat-ui-phase2-regressions.log` |
| 代码块 UI 专项 | 2 通过：两种行号模式的独立复制、中文跨行拖选、键盘复制；流式内容更新与 Mermaid 分流 | `target/chat-ui-phase2-code-final.log` |
| 格式 / 翻译 | 通过；两种语言各 3194 条 | `cargo fmt --all --check`、`python scripts/check-i18n.py` |
| Clippy | all-targets + ui-tests，零 warning 通过 | `target/chat-ui-phase2-clippy-final.log` |
| 生产构建 | conduit / conduit-updater / conduit-launcher 通过 | `target/chat-ui-phase2-build.log` |
| 库测试 / 助手测试入口 | 库测试 61 通过、1 忽略；两个助手测试入口通过（各 0 测试） | `target/chat-ui-phase2-lib.log`、`target/chat-ui-phase2-helpers.log` |
| 主程序全量测试快照 | 786 通过、1 失败、23 忽略；其后新增的流式代码测试另已通过 | `target/chat-ui-phase2-full-tests.log` |

主程序全量测试发现 `app::settings::plugins::encourager_tests::desktop_pet_selects_existing_provider_and_model` 找不到 `encourager-provider-option-pet-test`；单独复跑也失败（`target/chat-ui-phase2-unrelated-rerun.log`）。该文件及 provider_picker 正在被并行修改，本任务未编辑它们。第一阶段曾超时的 `git_stages_whole_files_after_idle_and_requires_manual_additions` 本次已通过。

第三阶段验证记录（2026-11-20）：

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| 应用专项 | 10 通过；覆盖链接、代码、引用定位、嵌套内容、搜索与流式追加 | `target/chat-ui-phase3-app-final.log` |
| 渲染库文本回归 | 399 通过；包含新增的列宽缓存、链接装饰和引用定义追加/替换测试 | `target/chat-ui-phase3-base-regressions.log` |
| Clippy | all-targets + ui-tests，`-D warnings` 通过 | `target/chat-ui-phase3-clippy-final.log` |
| 格式 | 全仓检查及五个依赖补丁文件的 rustfmt 检查通过 | `target/chat-ui-phase3-fmt-final.log` |
| 翻译 | 两种语言各 3194 条，通过 | `python scripts/check-i18n.py` |
| 可复现依赖 | 555 个原始文件与五个明确补丁的来源/锁文件检查通过 | `python scripts/check-gpui-vendor.py` |
| 生产构建 | 不含测试特性的主程序独立输出构建通过；updater / launcher 构建通过（dev profile） | `target/chat-ui-phase3-build-isolated.log`、`target/chat-ui-phase3-helpers-build-final.log` |

应用专项使用 `cargo rustc --locked --features ui-tests --bin conduit --profile test -- -o target/chat-ui-phase3-verified.exe` 生成独立测试文件，再运行相应用例，避免另一轮测试占用默认 exe。`-o` 引起的两条输出路径提示不是源码 warning；标准 Clippy 单独以零 warning 通过。本批未重复运行耗时的应用全量套件；第二阶段的全量结果保留在上表。

标准构建曾因另一进程正在运行 `target/debug/conduit.exe`，无法替换该文件而失败（Windows os error 5，记录见 `target/chat-ui-phase3-build-final.log`）。保留该进程，改用 `cargo rustc --locked --bin conduit -- -o target/chat-ui-phase3-production.exe`，已生成独立的非测试主程序 `target/chat-ui-phase3-production.exe`；随后 `cargo build --locked --bin conduit-updater --bin conduit-launcher` 通过。独立主程序构建同样仅有上述两条输出路径提示；本次未进行 release profile 构建或真实窗口启动验收。

共享工作区新增的 `src/app/sidebar/tests.rs` 曾使所有 UI 测试无法编译：`debug_bounds` 要求 `&'static str`，该测试传入 `&str`。本批仅将闭包参数改为 `&'static str`，调用方均为字符串字面量；未修改侧栏产品逻辑。真实窗口验收仍未完成，不将模拟窗口测试当成截图验收。

第四阶段验证记录（2026-11-20）：

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| 应用专项 | 11 通过；含图标与文字点击、部分选择、跨行拖选、剪贴板、流式引用、代码和搜索源位置 | `target/chat-ui-phase4-app-final.log` |
| 渲染库文本回归 | 最终 402 通过；包含三项新增图标排版、缓存和表格宽度回归 | `target/chat-ui-phase4-base-regressions-final.log` |
| Clippy | all-targets + ui-tests，`-D warnings` 通过 | `target/chat-ui-phase4-clippy-final.log` |
| 格式 | 本批应用文件及六个依赖补丁文件检查通过；首轮全仓通过，末轮发现并行侧栏改动的格式差异 | `target/chat-ui-phase4-fmt-scoped.log`、`target/chat-ui-phase4-fmt.log`、`target/chat-ui-phase4-fmt-final.log` |
| 翻译 / 来源 | 两种语言各 3196 条，555 个原始文件与六个明确补丁通过 | `scripts/check-i18n.py`、`scripts/check-gpui-vendor.py` |
| 非测试构建 | 独立主程序、updater、launcher 均通过（dev profile） | `target/chat-ui-phase4-build-final.log`、`target/chat-ui-phase4-helpers-build-final.log` |
| 真实窗口 | 未验收；两次 Computer Use 调用均报 native pipe unavailable / os error 2 | 不以模拟窗口测试替代明暗主题、DPI 和真实鼠标验收 |

应用测试沿用独立输出路径 `target/chat-ui-phase4-verified*.exe`，未重复运行应用全量套件，未发布或修改版本号。

非测试主程序由 `cargo rustc --locked --bin conduit -- -o target/chat-ui-phase4-production.exe` 生成，路径为 `target/chat-ui-phase4-production.exe`。构建仅有独立输出 `-o` 导致的两条路径提示，标准 Clippy 单独以 `-D warnings` 通过。未运行 release profile 构建或真实窗口启动验收。

最终全仓格式检查发现并行修改的 `src/app/sidebar/cache.rs` 与 `src/app/sidebar/projects.rs` 存在 rustfmt 换行差异。本批保留这些并行修改，未将末轮全仓格式检查标为通过；本批涉及的应用文件与六个依赖补丁文件已单独通过 rustfmt 检查。

### Review 问题修复（2026-11-20）

已修复两处已复现的问题：

- 子代理详情拖选任务或活动文本后，将键盘焦点转移到面板，使 Ctrl+C 复制选中的正文。面板保留独立焦点，拖选时请求重绘；返回输入框后仍可正常选择、复制草稿。
- Markdown 流式追加引用定义时，解析结果携带实际的全文重解析标记。高亮与待定位范围重新比较所有段落，移除已失效的范围并保留未变化段落的高亮；同时使旧高度缓存失效。追加新块仍保留逻辑滚动位置和选择兼容性，普通文档继续按尾块增量解析。

新增应用测试覆盖“已展开详情、输入框持有焦点、部分拖选、Ctrl+C、返回输入框复制草稿”；渲染库测试覆盖先前段落高亮错位、待定位范围失效、未变化段落高亮保留、屏幕外段落高度更新及追加新块后的滚动位置。

本轮验证：

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| 应用专项 | 32 通过，含两个新增复制用例，以及键盘、草稿、队列、文件链接、代码块与流式更新回归 | `target/chat-ui-review-fixes-app.log` |
| 渲染库文本回归 | 405 通过，含三个新增引用定义、高亮、布局与滚动用例 | `target/chat-ui-review-fixes-base.log` |
| Clippy | all-targets + ui-tests + remote，`-D warnings` 通过 | `target/chat-ui-review-fixes-clippy.log` |
| 格式 / 翻译 / 来源 | 全仓格式及依赖补丁 rustfmt 通过；两种语言各 3207 条；555 个原始文件与六个明确补丁校验通过 | `target/chat-ui-review-fixes-fmt.log`、`scripts/check-i18n.py`、`scripts/check-gpui-vendor.py` |

应用测试使用 `cargo rustc --locked --features ui-tests,remote --bin conduit --profile test -- -o target/chat-ui-review-fixes-tests.exe` 独立输出，构建日志为 `target/chat-ui-review-fixes-build.log`；两条 `-o` 输出路径提示不属于源码 warning。未重复运行应用全量套件，未启动或替换实际应用。真实窗口视觉验收仍待完成。
