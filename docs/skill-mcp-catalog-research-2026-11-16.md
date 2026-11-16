**Skill / MCP 三方目录与安装调研 · 2026-11-16**

状态：首版已实施；本文保留实施前的来源调研与分阶段建议。实际功能和限制见[设置与 AI 连接](settings.md#在线发现与安装)。本次实现采用 GitHub/skills.sh 及直连 MCP Registry 加本地缓存；下文建议的生产目录快照或聚合层、更新与卸载等后续能力尚未实现。

调研基线为本地 Conduit 0.3.6、HEAD `96c890a` 及当时工作区代码。通过 CodeGraph 检查现有链路，核对供应方文档和源码，并执行匿名只读 HTTP 请求。调研阶段没有安装三方 Skill/MCP、执行远程包或修改 CLI 配置。

建议在现有 Skills、MCP 两个设置页分别增加“已安装 / 发现”。共用目录浏览和安装任务基础设施，分别处理 Skill 文件包和 MCP 配置。首版优先让用户完成“发现 → 查看详情 → 选择引擎与范围 → 安装或添加 → 查看结果”。

| 对象 | 建议来源 | 首版交付方式 |
| --- | --- | --- |
| Skill | GitHub 仓库作为内容来源；skills.sh 作为可降级的搜索来源 | 内置少量精选仓库，支持输入仓库与子目录；skills.sh 搜索失败仍可从 GitHub 安装 |
| MCP | 官方 MCP Registry 的 `server.json` 模型 | 开发验证可直连；发布版通过目录快照或聚合层分发，提供分页、缓存和筛选 |
| 扩展来源 | 用户指定的标准目录；以后再考虑 Glama | 统一来源接口，保留各平台身份、署名和能力差异 |

这是一项中等规模功能。读取列表相对简单，主要工作在来源身份、目录到安装路径的解析、MCP 参数表单、原生配置转换，以及配置保存与进程启动的边界。

**现有基础与缺口**

| 已有实现 | 当前行为 | 可复用 / 需要补齐 |
| --- | --- | --- |
| [设置页状态与扫描](../src/app/settings/extensions.rs)，`ExtensionsUi`、`ensure_extensions` | 按引擎、工作区扫描本地目录；内存缓存 60 秒；搜索、过滤、虚拟列表 | 保留“已安装”；新增独立的远程目录状态，不把网络搜索结果塞进本地 `Catalog` |
| [Skill 导入](../src/extensions/edit.rs)，`destination`、`import_skill` | 选择本地文件夹，检查 `SKILL.md`，拒绝重名、链接和重解析点；限制 2,000 个文件/目录、20 MiB；暂存后重命名 | 可复用最终落盘阶段；新增下载、路径解析、来源记录与安装互斥 |
| [Skill 启停](../src/extensions/management.rs)，`toggle_skill` | 重命名 `SKILL.md`，使用锁并检查外部修改 | 在线安装的普通文件副本可沿用；首版不依赖符号链接安装 |
| [MCP 表单](../src/app/settings/config_editor.rs)，`McpForm`、`open_mcp_form` | 名称、命令/URL、逐行参数；先生成预览再保存 | 新增环境变量、请求头、必填项、包版本、认证与运行依赖展示 |
| [MCP 配置转换](../src/extensions/mcp.rs)，`add_preview` | 用 `remote: bool` 区分本地与远程，映射三种引擎格式；保留其他配置和注释；拒绝同名 | 扩展为结构化安装描述，不能把任意 `server.json` 直接写成原生配置 |
| [配置写入](../src/extensions/edit.rs)，`save` | 校验、文件锁、外部修改检查、备份和替换 | 复用，增加安装回执与写入结果一致性处理 |
| [GitHub 插件目录](../src/plugin_packages/catalog.rs)及[页面任务](../src/app/plugin_ui/management/catalog.rs) | HTTPS、超时、大小限制、缓存、分页、commit 固定、来源摘要；后台任务 generation 检查 | 提取通用 HTTP/GitHub 获取能力和交互模式；插件专用清单与 SVG 校验不能直接套用 |

目前 Qoder 的创建/导入入口明确禁用，首版继续显示“由原生 CLI 管理”，不因已有目标路径计算就开放写入。现有路径表可作为实现起点，尚未在本次调研中逐一启动 CLI 验证加载行为。

| 引擎 | Skill：项目 / 全局 | MCP：项目 / 全局 |
| --- | --- | --- |
| Claude | `.claude/skills` / 配置定位器给出的 Claude 目录下 `skills` | `.mcp.json` / 用户目录 `.claude.json` |
| Codex | `.agents/skills` / 用户目录 `.agents/skills` | `.codex/config.toml` / 配置定位器给出的 Codex 目录下 `config.toml` |
| OpenCode | `.opencode/skills` / 配置定位器给出的 OpenCode 目录下 `skills` | 项目目录 / OpenCode 配置目录内 `opencode.jsonc` 或 `opencode.json`，优先已有 JSONC |

**三方来源选择**

| 来源 | 已核实的接口与约束 | 结论 |
| --- | --- | --- |
| skills.sh 正式 API | `/api/v1/skills/search`、`/api/v1/skills/{source}/{skill}`；文档要求 Vercel OIDC，使用短期令牌，不能作为普通桌面应用的匿名 API。[S1] | 若需要正式 API 契约，增加部署在 Vercel 的代理或与平台确认其他接入方式；桌面端不内置共享令牌 |
| skills.sh CLI 使用的搜索接口 | 官方 `skills` CLI 的 `src/find.ts` 仍请求 `/api/search`；本次带 `q`、`limit` 匿名实测成功，返回 `source`、`skillId`、`name`、`installs` 等。[S2] | 可作为首版搜索适配器；它与正式 API 是不同契约，单独封装并允许关闭/降级，不把它描述为已承诺稳定的第三方 API |
| GitHub | 可以将 ref 解析为 commit，并从固定 commit 获取文件；匿名 REST API 有限流。[S3] | 首版主要 Skill 内容来源；支持精选仓库和直接地址，缓存目录与 ref 解析结果 |
| 官方 MCP Registry | `GET /v0.1/servers`，支持 `search`、`version=latest`、`limit`、`cursor`，返回 `servers[].server` 和 `metadata.nextCursor`；`search` 是服务器名称子串搜索。[S4] | 用作 MCP 上游标准。名称搜索不等于全字段全文搜索或热门榜，UI 不承诺这些能力 |
| 官方 MCP 聚合模式 | 官方文档建议下游通过自己的后端消费，并本地缓存或定期同步；可用 `updated_since` 增量同步。[S4] | 发布版使用目录快照/聚合服务，避免每个桌面实例遍历上游全量目录 |
| Glama | 目录、搜索、详情 API 需要 API Key；API 文档要求显示来源署名以及每条记录对应的 Glama 链接。[S5] | 适合作为后续增强来源，需单独解决 Key、展示要求和分发安排，不作为默认免配置来源 |
| Smithery | 官方公告：目录继续可浏览；旧 CLI、API 和托管服务于 **2026-10-31** 停止服务，改为直接连接远程服务器或使用开发者托管方式。[S6] | 不依赖旧 Registry API 或 `@smithery/cli` 自动安装；可保留网页跳转入口 |
| 其他目录站 | 本次未逐一核实其公开 API、安装元数据与访问安排 | 暂不接入，不能仅凭网站有列表就假定存在稳定 API |

正式 skills.sh API 可能提供更丰富的详情，但当前 CLI 搜索响应不能直接当作完整安装清单。`skillId`、仓库内路径和本地目录名必须分开保存：实测 `vercel-react-best-practices` 位于 `skills/react-best-practices/SKILL.md`，其 frontmatter 的 `name` 才是 `vercel-react-best-practices`。[S7]

目录可能包含 GitHub 之外的来源；正式 API 已区分 `github` 与 `well-known`。[S1] 首版安装器只接受明确解析成功的 GitHub 来源，其余显示来源入口，不能把域名强行当作 `owner/repo`。

skills CLI 本身支持来源解析、多引擎、多个安装模式与更新命令，可用作兼容性参考。[S8] 建议 Conduit 自己完成所选目录的下载和导入，以便安装范围、冲突提示和来源回执保持一致。直接包装 `npx skills add` 会引入 Node、CLI 版本、交互输出及安装策略的额外耦合。

**匿名接口实测**

以下结果来自 2026-11-16 本机只读请求，仅代表当时访问；没有提供账户令牌。外部服务响应不能当作长期可用性承诺。

| 请求 | 实测结果 | 对实现的影响 |
| --- | --- | --- |
| `GET https://skills.sh/api/v1/skills/search?q=react&limit=2` | 401，`authentication_required`，要求 Vercel OIDC | 正式 API 不能直接按匿名桌面请求接入 |
| `GET https://skills.sh/api/search?q=react&limit=2` | 200，返回两条 Skill 和仓库来源 | 可验证搜索适配器；响应未提供可靠的仓库子目录 |
| `GET https://registry.modelcontextprotocol.io/v0.1/servers?limit=2&version=latest` | 200，包含服务定义、状态元数据和翻页游标 | 可直接构造协议解析样例 |
| 同一 MCP 接口加 `search=playwright` | 200；结果包含请求头定义及带 `{region}`、`{workspaceId}` 的 URL | “获取 URL 后直接保存”不够，安装前需要变量表单 |
| `GET https://glama.ai/api/mcp/v1/servers?first=2` | 401，提示 API Key 与署名要求 | 不适合作为匿名默认来源 |
| `GET https://registry.smithery.ai/servers?q=github&pageSize=2` | 500 | 单次 500 不能单独证明停止服务；停用结论以官方公告为依据 |
| GitHub：解析 `vercel-labs/agent-skills` 的 `main` | 200，得到完整 commit | 可固定内容版本；该次结果不代表后续请求也有额度 |
| GitHub：随后读取该 commit 的递归 tree | 遇到 API rate limit，未获得目录树 | 需要缓存和限流退避；本次没有验证完整目录下载 |
| 从同一 commit 的 raw 地址读取 `skills/react-best-practices/SKILL.md` | 200，核对 frontmatter 名称与物理目录不同 | 证明名称和路径不能通过字符串拼接互相推断 |

**建议交互**

Skills、MCP 各自保留当前引擎/范围选择器，在内容区加入“已安装 / 发现”。发现页使用搜索框、来源筛选、分页列表；默认展示精选或已有缓存，避免打开设置时全量扫描网络。

列表显示名称、简述、发布者/仓库、来源、可用版本和安装状态。热度只标注对应平台的指标，不能用不同平台的数值进行统一排名，也不把安装量显示成安全认证。

打开详情后完成两种不同操作：

1. **Skill：安装。** 预览 `SKILL.md` 和文件清单，选择具体 Skill、引擎、项目/全局范围，显示目标路径与固定 commit；点击安装后下载并导入完整目录。
2. **MCP：添加服务。** 选择远程连接或本地包，填写必需变量/凭据，查看将使用的命令或 URL，再预览配置并保存。明确显示“保存后由 CLI 首次启动时下载/运行”的情况。

本地 npm MCP 的配置落盘不代表已下载包，更不代表服务可用。状态至少区分“已安装 Skill / 已添加配置 / 待认证 / 未检测 / 检测成功 / 检测失败”。检测结果只说明连接和工具发现，不表示工具业务操作已通过验收。

**Skill 安装流程**

1. 将来源解析成 `repository + ref + skill_id + path`；列表元数据缺少 path 时，扫描受限的仓库目录并读取 `SKILL.md` 匹配。仓库有多个候选时让用户选择，不能安装所有候选。直接输入分开的 ref/path 可以避免含斜杠分支名与子目录的歧义。
2. 将 ref 固定为 commit，后续清单、正文和依赖文件全部使用同一 commit；保存来源页和远端子目录。
3. 获取所选 Skill 子树。首版可用 Git Trees/Contents 加固定 commit 的 raw 文件下载，避免依赖 Git 可执行文件及新增压缩包解压器。目录树被截断时必须继续按子树取数或明确失败，不能安装不完整结果。
4. 下载整个 Skill 目录，包含 `scripts/`、`references/`、`assets/` 等支持文件；保留原始正文。标准规定这些支持目录可存在。[S9] 仓库根部共享资源、外链依赖和非标准包结构需要明确提示，不能凭猜测补全依赖。
5. 下载期间限制字节数、文件数、目录深度和总时限；拒绝绝对路径、`..`、Windows 保留名称、链接、重解析点、子模块，以及大小写折叠后冲突的路径。不能先无限下载再仅靠最终导入检查。
6. 以解析并校验后的安装名创建暂存目录，再调用现有导入能力。现有 `skill_metadata` 是简化 frontmatter 解析，需补齐标准字段与真实多行 YAML 样例；不应宣称它已经完整实现 Agent Skills 规范。
7. 为目标目录增加安装互斥和不覆盖发布策略，记录内容摘要与来源。遇到同名 Skill 提示目标冲突，首版不静默覆盖、自动改正文名称或合并两个目录。
8. 成功后刷新对应引擎/范围的本地列表；清楚提示后续新会话加载。取消或失败清理本任务暂存内容，不删除已有安装。

来源记录建议放在 Conduit 数据目录下独立的安装账本，按规范化目标路径索引，保存 `source_id`、仓库、子目录、commit、内容摘要、安装时间和目标上下文。不同引擎共享同一个物理目录时以真实路径去重，避免重复安装或误判为不同副本。

**MCP 解析与添加流程**

Registry 的元数据包含 `packages`、`remotes`、传输类型、包参数、运行参数、环境变量、请求头和变量声明；它是分发描述，不是某个客户端的配置文件。[S10]

| 元数据类型 | 首版处理 | 暂不承诺的范围 |
| --- | --- | --- |
| Streamable HTTP remote | URL、模板变量、静态请求头/令牌表单 → 原生配置 | OAuth 登录、令牌刷新由原生 CLI 完成；不能将需要认证的服务标为可直接使用 |
| npm + stdio | 使用显式支持的 `npx` 配方，固定包版本，预填结构化 args/env；检查运行环境 | 不执行目录提供的任意安装 shell，不自动安装 Node |
| PyPI / OCI / MCPB 等 | 可浏览详情并说明当前未支持 | uv/入口命令、Docker、桌面 bundle 各自需要适配，放到后续阶段 |
| SSE 或未知 transport | 标注不支持，可打开来源或手动编辑 | 不把 SSE 自动转换成 HTTP，也不推测未知协议 |
| 仅仓库地址、安装信息不完整 | 提供来源入口与手动添加 | 不由模型读取 README 后生成并执行安装命令 |

建议引入 `McpInstallSpec`，包含 transport、command、args、env、URL、headers、所需输入和认证提示；扩展配置预览函数接收该结构。参数必须保持数组元素边界，目录中的字符串不能直接拼接为 shell 命令。Windows 下还需覆盖 `npx.cmd` 定位、包含空格的路径和 Unicode 参数。

服务的 Registry 名称可能包含 `/`、`.`，当前本地名称只支持字母、数字、`_`、`-`。保留上游完整 ID，同时生成可编辑的本地别名并检查碰撞。多个 package/remote 是多个安装选项，不应全部写入。

凭据字段在表单中遮罩；目录缓存、来源回执、日志与通知不得包含用户输入的秘密。原生配置如需明文凭据，应准确显示保存位置，并优先利用目标 CLI 实际支持的环境变量/认证方式。不能将 Conduit 中加密保存的值直接当作其他 CLI 能读取的配置引用。

**必须调整的现有行为：自动检测。** `detect_mcp_tools` 会将未禁用服务入队；[现有说明](settings.md)明确检测可能启动命令或连接远程端点。新目录安装不能把“刷新已安装列表”顺带变成首次运行。建议新安装回执默认标记“尚未启动验证”，自动检测队列跳过，提供显式的“验证连接”；既有服务的自动检测策略可继续保留。

保存配置与允许原生 CLI 在下一会话运行仍是两个不同时间点，需要在保存按钮旁明确说明。Conduit 跳过自己的检测，并不阻止原生 CLI 在其加载配置时启动已启用 MCP。

**模块与数据边界**

| 层次 | 建议职责 | 对现有代码的连接 |
| --- | --- | --- |
| 目录适配器 | `search / detail / resolve`；提供分页、来源归属、安装选项和错误分类 | 新增 `src/extensions/catalog/`，不复用面向本地扫描的 `Entry.path` 语义 |
| 缓存 | 按来源、查询、游标、schema 版本分桶；有大小/条数上限，显示缓存时间 | 参考插件目录模式；401、429、超时、格式错误分别处理，失败不清空已有结果 |
| 安装规划 | 固定来源版本，绑定目标引擎/范围，列出必要输入与最终写入位置 | 新增 Skill 安装计划与 `McpInstallSpec`，在用户提交前生成可审阅结果 |
| 安装执行 | 下载、校验、取消、目标互斥、文件发布或配置合并 | 复用 `edit::import_skill`、`edit::save`、MCP 预览转换，按需要补齐并发边界 |
| 安装账本 | 上游身份、版本、摘要、目标路径、安装结果和首次验证状态 | 独立于目录缓存，不能只靠显示名称推断“已安装” |
| 设置 UI | 已安装/发现、来源筛选、详情、配置表单、安装状态 | 新建相邻页面模块，避免继续膨胀 `settings/extensions.rs` |

异步请求和安装任务都绑定发起时的上下文与 generation。切换来源/搜索词后丢弃过期查询结果；切换引擎或工作区后，正在进行的安装仍只能写入最初明确选择的目标，完成通知也要说明该目标。

账本与文件/配置写入涉及多个持久化位置，需要可恢复的中间状态。配置已经保存但账本失败时应报告具体结果并允许重建记录，不能显示一个笼统失败后盲目重试添加；重试不得生成第二份服务。

**发布时的目录分发**

建议把内容下载与目录分发分开：Skill 内容直接从固定 commit 的上游获取，目录可以由 Conduit 自己缓存；MCP 使用官方格式，避免绑定某个平台的安装 shell。

不想运维常驻后端时，可用定时工作流同步官方 MCP Registry 的元数据，生成分页索引、详情和更新时间，再发布到静态站点/CDN。客户端按需获取。该方案仍需要实现同步、状态变更处理和失效恢复，并不是当前已有能力。首次全量同步后使用增量更新，定期校对停用/删除状态；停止发布的服务不应继续作为新安装推荐。

skills.sh 的正式 v1 API 需要支持 OIDC 的服务端部署；普通静态同步不能自动满足其认证条件。若首版采用 CLI 搜索接口，应设置独立开关及缓存回退，保持精选 GitHub 和直接地址安装可用。

**建议交付顺序与验收**

| 阶段 | 交付内容 | 关键验收 |
| --- | --- | --- |
| 1：Skill 闭环 | 发现页、精选 GitHub、直接地址/子目录、可降级 skills.sh 搜索、完整目录安装与回执 | 单 Skill、多 Skill、名称与目录不同、ref 固定、支持文件完整、重名、取消、限流、网络中断 |
| 2：MCP 闭环 | Registry 解析与缓存、HTTP remote/npm stdio、必填变量与秘密表单、配置预览、独立验证 | 三引擎配置保真、注释与无关字段保留、URL 模板、参数含空格、缺少 Node、凭据遮罩、保存后不自动首次启动 |
| 3：发布收尾 | MCP 快照/聚合分发、来源故障隔离、缓存预算、只读与窄窗/键盘/双语支持 | 切换上下文不会错误写入、重启后安装状态可恢复、401/429 正确回退、只读不写入 |
| 后续 | 更新/卸载、更多 Registry、PyPI/OCI、私有源、多引擎批量安装 | 在已有文件被用户修改时避免误覆盖/误删，按具体运行时补齐验收 |

首版不承诺跨引擎自动同步、自动升级、依赖安装、仓库任意脚本执行或所有 MCP 一键可用。它应稳定交付的是选定来源、选定版本、选定目标的一次可追溯安装。

实现时的自动测试应使用固定 JSON/目录样例和 fake transport；真实站点请求另设显式连通性测试。核心失败样例包括目录树截断、超限包、路径穿越、Windows 大小写冲突、同名并发安装、原生配置被外部编辑、过期响应、账本写入失败及远程 URL 的未填写变量。连接测试使用受控的本地假服务器，不能在常规测试中运行目录中的真实程序。

本次只验证资料和匿名接口响应；没有完成安装、配置加载、GUI、OAuth 或真实工具可用性验收。上述顺序是工程建议，不是已交付功能或确定工期。

**外部依据**

| 编号 | 一手资料，核对日期 2026-11-16 |
| --- | --- |
| S1 | skills.sh API 文档：`https://skills.sh/docs/api` |
| S2 | Vercel Skills CLI 搜索源码：`https://raw.githubusercontent.com/vercel-labs/skills/main/src/find.ts`；main 为可变分支，结论以调研当日内容为准 |
| S3 | GitHub REST 限流说明：`https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api`；Git Trees：`https://docs.github.com/en/rest/git/trees` |
| S4 | MCP 官方聚合指南：`https://modelcontextprotocol.io/registry/registry-aggregators`；API 契约：`https://github.com/modelcontextprotocol/registry/blob/main/docs/reference/api/openapi.yaml` |
| S5 | Glama API 文档：`https://glama.ai/mcp/api` |
| S6 | Smithery 服务调整公告：`https://www.smithery.ai/future` |
| S7 | 固定 commit 的 Skill 示例：`https://raw.githubusercontent.com/vercel-labs/agent-skills/1a2c79d4e6c4a2283dc2c614b6538cbe1ed5ca62/skills/react-best-practices/SKILL.md` |
| S8 | Vercel Skills CLI：`https://github.com/vercel-labs/skills` |
| S9 | Agent Skills 规范：`https://agentskills.io/specification` |
| S10 | 本次 Registry 实际返回的 schema：`https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json` |
