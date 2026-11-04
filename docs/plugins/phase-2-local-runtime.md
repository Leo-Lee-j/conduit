# P2-B：独立进程插件运行时备选

设计版本：0.2；日期：2026-11-04；状态：保留的进程路线备选，未实现。前置：[P1 界面贡献](phase-1-contributions.md)与[共同约定](README.md)；后续：[P3 安装管理](phase-3-management.md)。仅在选择独立进程后，本文才是该后端清单、协议及运行资源限额的参数来源。

新增 [Shell 复用评估](gpui-shell-reuse-assessment.md)后，优先验证 [P2-A Shell 方案](phase-2-shell-runtime.md)。本文的 `.exe`、stdio JSON-RPC、固定模板、进程限额和进程信任说明不适用于 Shell。二者不同时实现；原 10–16 人日只对应本备选。

## 目标与信任范围

插件作者可以把清单和 Windows 原生可执行程序放在一个目录中，通过显式开发加载使用三个界面扩展点，无需重新编译主程序。交付清单校验、stdio 协议、原生模板、宿主 API、取消、限额、故障显示，以及应用关闭 / 更新前的完整清理。

本阶段提供拟议开发入口 `conduit.exe --plugin-dev <absolute-directory>`。该参数表示信任本次指定目录中的本地程序，只在当前应用运行期间登记；下次启动仍需显式指定。宿主先复制并校验目录快照，再从快照启动，避免源目录变化直接改变正在运行的包。快照使用 P3 约定的包目录与摘要算法，注册与信任决定暂存内存，普通用户的安装 UI 和持久化注册留到 P3。

不扫描工作区并自动执行插件，不启动市场下载，不提供任意 shell 命令宿主 API。用户信任的是能够以本地用户身份运行的程序；权限列表仅限定经 Conduit 接口发起的访问。原生进程自身的文件和网络能力不由 JSON 权限列表隔离。

## 清单 v1

文件名为 `plugin.json`，UTF-8，无注释。必要字段为 `schemaVersion`、`id`、`version`、`hostApi`、`runtime`、`contributes`；权限默认空列表，显示条件默认 true。未知顶层字段、未知权限、未知模板或高于宿主支持的 schema 均给出诊断并拒绝加载。

插件 ID 必须包含发布者与名称两个非空部分，仅使用小写 ASCII 字母、数字、点和连字符，首字符为字母，最大 128 字节；各段不能是空串。外部 ID 不以 `conduit.` 开头。贡献 ID 最大 192 字节，必须严格以前述插件 ID 加点为前缀，剩余部分非空并遵循相同字符约束。

`version` 是插件版本；`hostApi` 是宿主 API 的 semver 范围，与应用发行版本无关。`schemaVersion` 控制清单解释，`protocolVersion` 控制通信消息的基本结构，二者不根据插件版本隐式推断。

以下是完整示例；相较早期评估示意，活动项明确使用 `action` 联合结构，视图明确声明 `scope`。

```json
{
  "schemaVersion": 1,
  "id": "example.workspace-overview",
  "version": "0.1.0",
  "hostApi": "^1.0.0",
  "runtime": {
    "kind": "process",
    "program": "bin/workspace-overview.exe",
    "args": []
  },
  "permissions": ["workspace.metadata.read", "session.summary.read"],
  "activationEvents": [
    "onView:example.workspace-overview.index",
    "onView:example.workspace-overview.summary",
    "onCommand:example.workspace-overview.show-summary"
  ],
  "contributes": {
    "commands": [
      {
        "id": "example.workspace-overview.show-summary",
        "title": "打开工作区概览"
      }
    ],
    "activityItems": [
      {
        "id": "example.workspace-overview.activity",
        "title": "工作区概览",
        "icon": "folder",
        "order": 100,
        "when": "workspace.open",
        "action": {
          "type": "openView",
          "viewId": "example.workspace-overview.index"
        }
      }
    ],
    "views": [
      {
        "id": "example.workspace-overview.index",
        "title": "工作区概览",
        "location": "navigation",
        "scope": "workspace",
        "renderer": "list",
        "when": "workspace.open"
      },
      {
        "id": "example.workspace-overview.summary",
        "title": "概览详情",
        "location": "right",
        "scope": "workspace",
        "renderer": "keyValue",
        "when": "workspace.open"
      }
    ],
    "statusItems": [
      {
        "id": "example.workspace-overview.status",
        "scope": "workspace",
        "alignment": "right",
        "priority": 20,
        "text": "工作区概览",
        "command": "example.workspace-overview.show-summary",
        "when": "workspace.open"
      }
    ]
  }
}
```

`runtime.program` 必须为包内相对 `.exe` 路径。拒绝绝对路径、父目录跳转、设备路径、备用数据流及指向包外的重解析点；从已校验快照解析最终绝对路径，使用参数数组创建进程，不拼接 shell 命令。v1 不自动寻找 Python、Node.js 或脚本解释器。

`activationEvents` 可省略，此时从声明的视图和命令派生。显式给出时只支持对应本插件贡献的 `onView` / `onCommand`，且必须覆盖所有可访问的视图和命令；不支持通配符、启动和定时事件。状态项注册只显示静态默认值，点击其命令时才激活。没有活动工作区时，不实例化 workspace / session 作用域视图。

清单校验分为解析、版本 / 权限 / 路径、贡献引用及注册冲突四步。全部通过后批量注册静态入口；进程握手稍后按需发生。运行错误与清单错误分别呈现，清单错误不产生部分注册的入口。
`contributes` 中缺省的数组按空列表处理；激活期间不动态新增或改变贡献定义，包版本替换时才重新校验注册。首版权限名称限定为 `workspace.metadata.read`、`session.summary.read`，以及 P3 实现的 `storage.read` / `storage.write`；权限检查取已声明、已接受且当前宿主支持的交集。开发参数明确授予本次加载所声明的受支持权限，插件仍不能访问未声明的方法能力。

## 宿主与进程生命周期

| 状态 | 进入条件 | 允许行为及退出 |
| --- | --- | --- |
| Registered | 清单通过校验且当前运行已信任 / 启用 | 显示静态入口；首次用户动作进入 Activating |
| Activating | 分配 generation 并启动进程 | 合并同插件激活请求，握手后进入 Active；超时或不兼容进入 Failed |
| Active | 握手通过，宿主发送 initialized | 接受经验证的请求和通知；停用、退出或宿主转只读时进入 Stopping |
| Stopping | 宿主拒绝新请求、撤销用户动作令牌和订阅 | 取消任务、发送 shutdown、限时终止，确认无子进程后回到 Registered 或 Disabled |
| Failed | 崩溃、协议故障、持续超额 | 保留错误占位和用户重试入口；先完成原进程清理，再接受手动重试 |
| Disabled | 未授权、被停用或宿主不可写 | 不运行进程；是否展示不可用入口由宿主原因决定 |
| StopFailed | 超过清理时限仍未确认进程树退出 | 保留句柄与诊断，可重试停止；阻止升级、删除运行包及通过退出屏障 |

每次启动创建新的 generation。进程身份由宿主所持的 IPC 连接和 generation 决定，不能由消息里的 `pluginId` 自报替代。v1 不自动崩溃重启，防止重复执行副作用；重试由用户触发。

开发加载结束或移除插件时，按 P1 规则注销贡献并回退当前视图。一般运行失败保留入口和错误占位，用户可查看诊断后重试。临时无可见视图时进程可以继续存在，但不轮询隐藏视图；空闲自动休眠留待性能测量后决定。

## stdio 协议

使用 JSON-RPC 2.0 的请求、响应、通知和错误对象；Conduit 定义具体方法和传输约束。每行一个紧凑 UTF-8 JSON 对象，以 LF 分隔并接受 CRLF；字符串内换行必须转义。v1 通信双方不发送批请求，宿主收到顶层数组时按不支持的请求拒绝。

stdout 仅承载协议，stderr 由后台有界读取。读取器在累积到消息字节上限时立即终止解析，不能使用无限增长的 `read_line` 缓冲。解析和视图数据校验均在后台完成，UI 线程只接收已验证 DTO。

请求 ID 为字符串：宿主使用 `h:<counter>`，插件使用 `p:<counter>`；不重用同一代次中仍在途的 ID。通知不含 ID，不回复响应。响应必须匹配当前连接上的在途请求；重复、迟到或未知 ID 不能产生 UI 或存储副作用。

会话 ID、contextVersion、generation 和视图 revision 在协议中使用十进制字符串或不透明字符串，避免语言间整数精度差异。宿主对需要比较的计数器执行有界整数解析。协议不传输 Rust 指针、GPUI Entity、CLI 凭据或整份 Store。

握手顺序为 `initialize` 请求 → 版本与身份匹配的响应 → 宿主 `initialized` 通知 → 经权限过滤的 `context/changed`。握手前除对应响应与 stderr 外的访问均拒绝。初始化请求提供 protocolVersion、hostApiVersion、预期插件 ID / 版本及宿主支持的能力；响应必须匹配身份与协议，并声明支持的视图和命令处理能力。由清单模板及权限推导必需能力，缺失时不进入 Active。

P2 实现读取、视图和命令能力；`storage.read` / `storage.write` 的持久化能力由 P3 增加，在未实现的宿主中不得提前广告或授予。P3 安装和升级只注册静态贡献；实际握手仍在用户显式打开入口时执行，避免包管理操作隐式启动程序。

| 方法 | 方向 | 关键参数 / 结果 | 语义 |
| --- | --- | --- | --- |
| `initialize` / `initialized` | 宿主 → 插件 | 协议、API、插件身份、能力 | 仅初始化一次 |
| `context/changed` | 宿主 → 插件通知 | contextVersion、授权摘要、数据修订号 | 仅 Active 实例；同代次的旧上下文通知可合并 |
| `views/read` | 宿主 → 插件请求 | viewId、instanceId、contextVersion；返回模板快照与 revision | 首次打开、上下文变化或可见视图失效时调用 |
| `views/changed` | 插件 → 宿主通知 | instanceId、contextVersion、revision | 只标记已绑定实例失效；可见时合并触发 read，隐藏时不请求 |
| `commands/execute` | 宿主 → 插件请求 | commandId、args、contextVersion、invocationToken | 用户点击后的逻辑处理；结果用于结束忙碌状态 |
| `status/update` | 插件 → 宿主通知 | id、contextVersion、revision、text、icon、tooltip | 只能更新本插件已声明项的显示值，不能改位置或命令 |
| `context/get` | 插件 → 宿主请求 | 当前 contextVersion | 返回该插件授权范围内的快照 |
| `workspace/getSummary` | 插件 → 宿主请求 | contextVersion | 当前工作区名称、授权路径与会话计数；需 workspace.metadata.read |
| `session/getSummary` | 插件 → 宿主请求 | contextVersion | 当前会话 ID、标题、引擎、运行状态；需 session.summary.read；不返回消息正文 |
| `events/subscribe` / `events/unsubscribe` | 插件 → 宿主请求 | topics / subscriptionId | 仅 workspace.summaryChanged、session.summaryChanged；按相应读取权限校验 |
| `events/changed` | 宿主 → 插件通知 | subscriptionId、contextVersion、dataRevision | 摘要失效通知，按作用域合并；不广播 CLI 原始事件 |
| `ui/openView` | 插件 → 宿主请求 | viewId、contextVersion、invocationToken | 只能打开自己已注册且可见的视图，需当前有效用户动作令牌 |
| `cancel` | 任一方 → 对方通知 | requestId | 本协议自定义的尽力取消；接收方停止相关工作，仍需丢弃迟到结果 |
| `shutdown` | 宿主 → 插件请求 | reason | 允许在初始化结束前或 Active 状态停止；不允许无限延长退出 |

对外上下文的基础字段只包含路由版本、不透明工作区 / 会话标识、当前文档种类与标识、宿主可写状态；工作区名称 / 路径和会话标题等摘要分别受对应读取权限约束。活动文档元信息不包含文件内容，首版没有文件全文读取 API。

所有有作用域的方法重新检查 contextVersion 和当前目标身份。权限不是 `when` 条件的结果。`ui/openView` 的令牌由宿主在转发用户命令时生成，绑定插件、generation、上下文与该命令，在 10 秒内且命令未完成时有效，首次成功使用后消耗；背景通知不能生成令牌或抢焦点。激活完成前若用户已经切换上下文，原命令直接取消，不再生成令牌。

应用级错误码固定为：`-32001 PermissionDenied`、`-32002 StaleContext`、`-32003 NotAvailable`、`-32004 ResourceLimit`、`-32005 ReadOnly`、`-32006 Cancelled`、`-32007 DeadlineExceeded`，另预留 `-32008 RevisionConflict` 供 P3 存储比较并交换使用。标准错误使用 JSON-RPC 规定的解析、请求、方法和参数错误码。错误文本不得直接包含完整凭据、环境变量或会话内容。

## 原生视图模板

模板快照的公共字段为 `kind`、`instanceId`、`contextVersion`、`revision`。kind 必须匹配清单 renderer，实例必须属于当前插件，contextVersion 必须与对应请求一致，revision 在同实例同代次中单调递增。

```json
{
  "jsonrpc": "2.0",
  "id": "h:12",
  "result": {
    "kind": "list",
    "instanceId": "vi-7",
    "contextVersion": "12",
    "revision": "1",
    "items": [
      {
        "id": "workspace.summary",
        "label": "当前工作区",
        "description": "Conduit · 3 个会话",
        "command": {
          "id": "example.workspace-overview.show-summary",
          "args": {}
        }
      }
    ]
  }
}
```

| renderer | 数据与交互 | 限制 |
| --- | --- | --- |
| `list` | 稳定行 ID、label、description、可选图标与本插件命令；宿主滚动、选择和虚拟化 | 最多 1000 行；行 ID 唯一；单个文本字段最多 4096 UTF-8 字节 |
| `keyValue` | `entries` 包含 key / value，宿主提供复制和选择 | 最多 200 项；单个 value 最多 8192 字节 |
| `markdown` | `text`，使用宿主主题渲染 | 最多 128 KiB；首版不加载远程图片、不执行 HTML / 脚本、不启用 Mermaid；链接只显示文本 |

模板中的命令通过同一分发器执行，宿主重新生成用户动作上下文。插件不直接修改焦点或滚动条，不接收任意布局属性。非法快照被拒绝；保留最近一次有效内容并标记过期 / 错误，没有有效内容则显示错误占位。

视图更新先按 `(generation, instanceId, contextVersion)` 去重，再按 revision 判定新旧。宿主只允许当前已挂载实例更新；隐藏或已注销实例的通知不能重新创建实例。数据校验通过也必须在 UI 应用结果之前再检查上下文，防止校验期间用户切换工作区。

## 初始资源限额

这些是首版实现默认值和需要验证的设计预算，不是已测得的性能结果。修改时同步更新验收夹具和开发文档。

| 项目 | 初始值 / 行为 |
| --- | --- |
| 清单大小与贡献量 | 256 KiB；每插件最多 8 个视图、8 个活动项、8 个状态项、32 个命令 |
| 同时运行 | 每插件 1 个进程；全应用最多 8 个外部插件进程，超过时返回明确资源错误 |
| 请求量 | 每插件最多 32 个在途请求，激活期间最多挂起 8 个用户请求 |
| 单条协议消息 | 最多 1 MiB，超限终止该实例 |
| 命令参数 | args 为 JSON 对象，序列化后最多 16 KiB；禁止任意宿主对象或函数 |
| 收发队列 | 每插件、每方向最多 128 条或 4 MiB 普通消息；另留 32 条 / 256 KiB 控制空间 |
| 控制消息 | 仅小型取消、关闭、错误及订阅控制；大视图响应计入普通消息，控制空间耗尽时停止该实例 |
| 通知频率 | 按实例合并可替代的更新；每插件至多接收 100 条 / 秒，允许 200 条短时突发；持续超额 5 秒停止实例 |
| 状态项刷新 | 单项最多应用 5 次 / 秒，中间值合并；单项 text 最多 256 字节，tooltip 最多 2048 字节 |
| 订阅 | 每插件最多 16 个；停用和代次变化自动清理 |
| UI 归并预算 | 每轮所有插件合计最多 64 条已校验消息或 1ms，轮转处理插件，不能每插件各占 1ms |
| 超时 | 激活握手 5 秒；视图 / 读取请求 5 秒；用户命令 10 秒 |
| 停止 | 最多 2 秒礼貌关闭，随后终止 Job；再用最多 3 秒后台确认进程树退出，超时进入 StopFailed |

协议读写和资源清理由后台线程或任务管理，UI 不等待管道、进程退出或大对象解析。队列满时可替代通知先合并；无法保留必要请求 / 控制响应则停止该插件，不能阻塞关闭流程。每个插件独立队列，避免单个发送者占用整个应用的事件预算。

现有 [event_queue.rs](../../src/event_queue.rs) 绑定 CLI `Event`，可借鉴数量 / 字节上限及控制空间，或提取通用内部队列；插件消息不得直接混入 CLI 事件枚举。[stream_events.rs](../../src/app/stream_events.rs) 已有聊天流的公平处理预算，新增插件预算需要一起做主线程性能验证。

## 进程环境与退出屏障

[child.rs](../../src/child.rs) 当前的 `spawn_with_env` 从宿主环境生成子进程环境再应用覆盖。建议增加显式环境模式，例如 `ProcessEnvironment::Explicit`，只供插件使用；CLI 现有继承和覆盖行为保持原有语义。

插件环境只传必要 Windows 系统变量、指向宿主创建的该插件临时目录的 TEMP / TMP，以及插件身份和协议版本。PATH 仅包含明确的 Windows 系统目录；可执行文件用绝对路径启动。凭据、代理认证、CLI 配置、CONDUIT_DATA_DIR 和其他宿主环境不自动传入；插件当前目录为包快照目录。临时目录与包路径均由宿主创建和管理。

普通关闭和安装更新都需要将插件计入完成条件：

1. 标记宿主停止接受新激活和新命令，使当前用户动作令牌失效。
2. 撤销订阅、取消在途任务，启动每插件的限时关闭；写入器仍完成已受理的数据保存。
3. 后台清理进程树并归并确认消息；确认已退出后释放句柄和实例，不能先从管理表移除再等待。
4. 全部插件和相关持久化完成后，才允许窗口退出或安装助手继续。

现有 [runtime.rs](../../src/app/runtime.rs) 在 `poll_updates` 后遇到 preparing / exiting 会提前返回。插件清理轮询应在这个早返回之前执行；[update.rs](../../src/app/update.rs) 的 `update_idle` 和安装准备状态机必须包含插件停止与写入确认。不能简单等待长期 Active 插件自行变成 idle：用户请求安装时应先发起插件 quiesce，再等待清理屏障。

关闭按钮的同步回调只发起停止并返回是否已完成；不要在 [app.rs](../../src/app.rs) 的回调中 join 读取线程。P3 的安装注册和插件数据写入器接入同一屏障。

## 模块、实施顺序与验收

建议在 `src/plugins/` 新增 `manifest.rs`、`protocol.rs`、`host.rs`、`process.rs`、`queue.rs`、`permissions.rs`；`src/app/plugin_views.rs` 增加模板渲染；在应用启动参数处增加显式开发加载；`runtime.rs`、`app.rs`、`update.rs` 接入停止流程。

| 工作项 | 交付 | 验证重点 |
| --- | --- | --- |
| P2-1 清单与快照 | 版本、路径、引用及能力检查，显式开发入口 | 同 ID 冲突、越界路径、未知模板、重复参数和不兼容版本 |
| P2-2 传输与生命周期 | 后台读写、握手、generation、限额、关闭 | 分段 UTF-8、超长行、stderr 洪泛、超时、EOF、子进程树清理 |
| P2-3 模板与宿主 API | 权限过滤、上下文快照、三个模板、用户动作令牌 | 跨 owner / scope 拒绝、旧结果丢弃、背景通知不抢焦点 |
| P2-4 外部示例 | 与 P1 示例相同的三处入口和交互 | 无需编译宿主即可加载、停止、改版本后重载 |
| P2-5 退出与性能 | 关闭 / 更新屏障、队列公平性 | 活跃插件、超时插件并存时聊天继续；安装前确认无插件子进程 |

使用无网络、无模型调用的可控子进程夹具：正常响应、握手挂起、发送不完整 JSON、发送大消息、创建孙进程、延迟旧上下文响应和忽略 shutdown。现有 [child.rs](../../src/child.rs) 的 Unicode 参数与进程树清理测试可作为回归基础。夹具包只能写入隔离测试数据目录。

P2 完成条件：外部示例通过三处联动；未激活插件没有进程；权限与上下文校验覆盖正反例；错误实例可以手动重试；普通关闭与安装准备均验证子进程树退出；留存资源限额和 UI 归并的测试记录。预估 10–16 人日。

JSON-RPC 对象与标准错误的规范依据：`https://www.jsonrpc.org/specification`。按行传输、具体方法、取消、权限和资源预算是本项目的协议设计，需要在 SDK 文档中明确区分。
