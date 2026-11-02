# GitHub 自动发布与应用更新评估

日期：2026-11-02。本文保留实施前的评估与取舍；功能现已接入，当前配置与使用见 [自动发布与应用更新](auto-update.md)。范围为 Rust + GPUI、Windows x64 和按用户安装的 NSIS 安装包。

## 结论

可以实现。首版建议沿用 NSIS，增加 GitHub Actions 发布、签名更新清单、Rust 后台更新服务和独立更新助手。交付应覆盖检查、changelog、后台下载与进度、完整性及真实性验证、安装并重启、失败恢复，以及重启前的任务和草稿保护。

“自动发布”定义为：维护者更新版本并推送 `vX.Y.Z` 标签后，CI 自动检查、构建、打包并发布。不要每次推送主分支都发布正式版。

“自动安装重启”定义为：用户点击“安装并重启”后无需再操作安装向导；另提供默认关闭的“空闲时自动安装并重启”。下载完成不能直接强行结束正在运行的 CLI 或 Git 任务。后台下载默认开启，可在设置关闭。

基础可用闭环估计 5–7 个工作日；加上签名、安装失败恢复、跨版本和 Windows 实机验证，建议按 8–12 个工作日安排。估计基于单人熟悉本仓库、仅支持 Windows x64、不含证书申请、企业网络适配和大规模分发；不是已经验证的工期。

## 已有基础与缺口

| 项目 | 当前证据 | 对实现的影响 |
| --- | --- | --- |
| 应用框架 | `Cargo.toml`：Rust 2024、GPUI Kit；`src/main.rs` 使用 Windows API | 使用 Rust 服务及 GPUI UI，无 Tauri updater 可直接接入 |
| 版本 | `Cargo.toml` 为 `0.1.0`；`build.rs:22` 从 Cargo 写入 exe 版本资源 | Cargo 为唯一版本来源，CI 校验标签一致 |
| 打包 | `scripts/package-windows.ps1` 已构建、检查 x64 PE、捆绑 MSVC CRT、生成 SHA-256 | 自动发布可以复用，先补 CI 环境验证 |
| 安装 | `scripts/windows/installer.nsi`：HKCU、当前用户、允许选择路径、开始菜单及卸载信息 | 更新必须安装到当前实际目录，并更新 DLL/卸载信息 |
| 数据 | `src/store.rs:279`：默认 `%LOCALAPPDATA%/Conduit/state.json`，支持 `CONDUIT_DATA_DIR` | 数据与安装目录独立；更新缓存不能覆盖用户数据 |
| 更新入口 | `src/app/settings.rs:401` 已有关于弹窗，`:449` 的按钮仅显示“更新检查暂未开放” | 入口可以复用，但服务、进度和 changelog 均需新增 |
| 后台 UI | `src/app.rs:543` 的后台 timer 驱动 `poll` | 可接收后台事件；网络和安装等待不得进入 UI 线程 |
| 退出 | `src/app.rs:504` 关闭钩子取消任务；`src/app/runtime.rs:945` 等待部分任务退出 | 提取共享退出协调器；不能简单调用退出并假定所有任务已结束 |
| 单实例 | `src/main.rs:180` 的互斥量按数据目录区分实例 | 更新助手需等旧进程退出和互斥量释放；共享安装的多实例也需检测 |
| 草稿 | `src/app.rs` 中 `drafts`、`workspace_drafts`、`commit_drafts` 为内存映射 | 重启前须保存当前输入和各会话草稿；附件也需恢复或明确阻止重启 |
| HTTPS | `ureq` 关闭默认 features，仅启用 `json`；锁定的 `ureq 3.5.0` 依赖无 TLS 后端 | 现有 HTTP 客户端不可直接用于 GitHub HTTPS，需显式启用 TLS |
| CI | 工作区没有 `.github/` | 需要新增校验及发布流程；远端仓库可见性、Actions/Secrets 未核实 |

评估时工作区已有多处未提交改动；上述结论以当前磁盘为准，实施时应保留这些改动。

## 路线选择

| 路线 | 适配成本 | 优点 | 主要代价 | 判断 |
| --- | --- | --- | --- | --- |
| 现有 NSIS + Rust 更新服务/助手 | 中 | 沿用安装目录、注册表和现有打包脚本；升级已有安装可控 | 等待进程、备份恢复、静默安装、重启需自行实现和实测 | 当前首版推荐 |
| Velopack Rust SDK + vpk | 中到高 | 官方 Rust SDK 提供检查、下载、应用更新并重启；支持 GitHub 更新源 | 必须改用其打包与安装生命周期；旧 NSIS 的升级、卸载迁移需设计 | 如果愿意现在迁移安装体系，优先做一次技术验证 |
| `self_update` / 仅替换 exe | 表面低 | 适合简单便携程序 | 现有安装还包含 CRT DLL、卸载器和注册表；单独替换 exe 不能覆盖完整升级 | 不选作安装版主方案 |

Velopack 的具体安装恢复、签名和旧 NSIS 迁移保证未在本次验证中确认，不能以“接入 SDK”替代这些验收。也不建议同时维护两套自动安装后端。

## GitHub 自动发布

建议新增 `.github/workflows/ci.yml`、`.github/workflows/release.yml`、`.github/release.yml` 与发布产物生成脚本。

发布顺序：

1. 推送 `vX.Y.Z` 标签；手动 workflow 可用于重跑同一标签，不自动修改源码版本或提交。
2. 检出标签对应提交，校验 Cargo 版本、稳定版标签格式与目标平台；固定 Rust 工具链和 Actions 完整 commit SHA。
3. Windows runner 安装/定位 NSIS、MSVC/Windows SDK，执行格式、clippy、单元测试和 release 构建。涉及真实 CLI 登录和模型调用的测试保持独立，不作为发布必需条件。
4. 复用 `package-windows.ps1`；CI 验证 MSVC CRT 目录和 NSIS 路径，不能依赖开发机的 `%LOCALAPPDATA%/tauri`。
5. 生成安装包、SHA-256、签名清单。若启用 Authenticode，先签安装包，再计算摘要和签清单。
6. 创建或更新 draft Release，生成发布说明，上传全部产物并验证名称、大小、摘要与清单关联。
7. 全部完成后发布 Release；正式版设为 Latest。失败时保留 draft，不让客户端发现半套产物。

建议产物：

- `Conduit-X.Y.Z-windows-x64-setup.exe`
- `Conduit-X.Y.Z-windows-x64-setup.exe.sha256`，供人工下载验证。
- `update.json` 与 `update.json.sig`，供客户端消费。

`update.json` 至少包含协议版本、应用版本、平台/架构、安装包文件名、下载 URL、字节数、SHA-256、发布时间、changelog、最低更新器版本、签名 key ID。可附首次更新时需要的升级说明。签名覆盖原始清单字节，避免 JSON 重新序列化差异。

GitHub Release body 可由 PR 标签自动生成，并允许维护者整理。CI 将同一份说明写入清单，保证发布页与应用一致。跨多个版本更新时首版显示目标版本说明，并提供完整历史入口，不把目标版本说明误称为全部累积变更。

权限：校验 job 只读；只有发布 job 使用 `contents: write`。为同一标签设置 concurrency；同一已发布版本不替换安装包，修复通过新版本发布。签名私钥放受保护的 Actions Secret/Environment，客户端只带公钥。不要为不可信 PR 提供发布密钥。

预发布首版不启用：现有 NSIS `VIProductVersion` 需要四段数字，打包脚本也只提取三段版本，不能直接认为 `0.2.0-beta.1` 已受支持。

## 客户端功能和交互

入口复用“关于 → 检查更新”，设置页新增更新配置。关于页显示当前/目标版本、发布时间、检查状态、进度和 changelog，按状态显示“检查更新 / 下载更新 / 取消下载 / 重试 / 安装并重启 / 稍后”。主界面仅提供轻量更新提示。

建议默认策略：启动后延迟约 30 秒后台检查，之后每 6 小时检查一次；后台下载开启；自动安装重启关闭；允许跳过某一版本。手动检查可绕过检查间隔，但不允许并发请求同一版本。

| 状态 | 行为 |
| --- | --- |
| Idle / Checking | 异步读取发布信息；只允许一个检查请求 |
| UpToDate / Available | 用 SemVer 比较，不按字符串排序；不降级、不安装相同版本；跳过 draft/prerelease |
| Downloading | 流式写入版本专属 `.part`，展示字节/百分比，支持取消和重试；未知长度时显示已下载字节 |
| Verifying | 验清单签名、平台、版本、大小和摘要；成功才原子重命名为已就绪包 |
| Ready / WaitingForIdle | 显示“安装并重启”；有活动任务时等待或由用户明确选择停止任务后安装 |
| PreparingRestart / Installing | 阻止新任务、保存恢复信息，启动独立助手；准备失败留在原应用 |
| Failed | 显示可理解的错误和重试入口；原应用继续可用，验证失败的产物不得执行 |

自动安装仅在所有 CLI、命名任务、版本探测、Git 操作等后台活动结束，且草稿/附件恢复信息成功保存后执行。空闲条件应由统一函数定义，并处理等待期间新任务启动的竞态。不能只判断当前会话或 `runs.is_empty()`。

## 检查与下载

首版使用固定仓库 `Leo-Lee-j/conduit` 的 GitHub Release API 获取稳定发布信息，再读取对应的签名清单。不能仅凭 `latest` 或 Release 标签认定可以升级；客户端仍需比较清单里的 SemVer，并核验与发布标签一致。

公开仓库可匿名访问；GitHub 匿名 REST 主限额为每 IP 每小时 60 次。使用缓存、退避和低频检查，遇到 403/429 遵循服务器重试指示，404 显示“暂无可用版本”。私有仓库分发需要独立授权/更新源方案，不能把 PAT 嵌入桌面应用；本次不假定远端为公开仓库。

启用 `ureq` TLS 并配置证书校验、User-Agent、连接/读取/整体超时和代理策略。验证 GitHub CDN 重定向，限制 HTTPS 与允许的下载主机；不要禁止所有重定向导致正常下载失败，也不要允许降级 HTTP。网络请求、磁盘流式写入和摘要计算都在后台线程执行。

缓存建议位于应用数据目录的独立 `updates/` 子目录；设置磁盘空间检查、下载大小上限、取消、有限次数重试和旧缓存清理。首版可在网络中断后重新下载，断点续传另做；若实现续传，必须核对 ETag/版本、Range 响应并重新校验完整包。

仅有 HTTPS + 同一 Release 上的 SHA-256 可以检出损坏，不能独立证明发布者身份。建议首版自动执行安装包前强制验证离线私钥签发的更新清单；公钥随应用发布，设计 key ID 与换钥兼容窗口。Authenticode 作为 Windows 发布信誉与额外身份验证手段，不能承诺消除 SmartScreen 提示。

## NSIS 安装与重启

独立助手建议作为同一 Rust 工程的第二个 binary，避免依赖 PowerShell 执行策略。由打包脚本随主程序分发，运行时从缓存目录启动副本，避免助手自身占用待覆盖的安装目录。助手权限保持当前用户，不要求管理员权限。

交接流程：

1. 主程序确认安装包已验证；检查安装版身份、实际 exe 路径、安装目录可写和其他共享安装实例。开发构建默认禁止自动安装；未知便携安装提示使用正式安装包。
2. 保存当前输入、所有草稿、待恢复附件和工作区/会话选择；等待最终数据写盘成功。传递 `CONDUIT_DATA_DIR` 等必要环境和恢复标识，不原样回放所有进程参数。
3. 启动缓存目录中的助手并握手成功，再退出主程序。助手持有旧进程句柄并等待退出，不能按固定秒数睡眠后直接覆盖；超时则放弃本次安装。
4. 助手再次验证安装包和交接信息，运行 NSIS 静默安装到当前实际目录。NSIS 的 `/S` 区分大小写，`/D=` 必须在最后且其值不能额外加引号，即使目录有空格；应按 Windows 原生命令行规则专门测试，不直接套用普通参数构造。
5. 等待安装退出，记录退出码；不要假定静默安装会自动启动应用，现有 MUI 完成页运行选项也不承担该职责。失败时执行明确的恢复流程。
6. 助手启动当前目录中的新版本，恢复正确数据目录和工作区，记录升级结果；新版本启动后显示该版本 changelog，清理已完成交接和旧缓存。

现有 NSIS 是原地覆盖，不具备事务保证。基础失败处理必须做到：安装前快照当前安装目录及必要注册表版本信息；文件写入失败可还原旧程序与 DLL；保留诊断日志；恢复失败提供重装路径。不能只看安装器非零退出就重新启动可能已被覆盖一半的程序。

“安装过程中断可恢复”和“新版本逻辑有 bug 自动回滚”是不同能力。首版保障前者；后者涉及启动健康确认、数据格式迁移和备份恢复，作为单独增强项。至少在升级前备份 `state.json`，新版本数据迁移失败必须保留原文件，不能自动用默认数据覆盖。

共享安装可由不同 `CONDUIT_DATA_DIR` 的多进程同时运行，当前单实例互斥并不能阻止这种情况。需要安装目录级更新锁与使用中检测；发现其他实例时提示退出，不能强杀未知 PID。

## 建议代码分工

| 文件 | 职责 |
| --- | --- |
| `src/update.rs`、`src/update/` | 版本/清单模型、GitHub 检查、签名验证、下载缓存、后台事件、安装交接 |
| `src/bin/conduit-updater.rs` | 等待退出、复验、备份、静默安装、失败恢复、重启和日志 |
| `src/app/update.rs` | 接收更新事件、驱动 UI、协调准备重启 |
| `src/app/settings.rs` / `settings/general.rs` | 关于弹窗更新入口、changelog 和偏好配置 |
| `src/app.rs` / `runtime.rs` | 共用退出协调器、禁止新任务、最后一次持久化和退出原因 |
| `src/store.rs` | 向后兼容的更新偏好、草稿和恢复数据 |
| `scripts/package-windows.ps1` / `windows/installer.nsi` | 打包助手、明确静默升级契约、失败检测、恢复与清理 |
| `.github/workflows/` | 校验、构建、打包、签名、完整发布 |

依赖预计增加 SemVer、SHA-256、签名验证库，并显式开启 HTTPS。具体库版本在实施时核实维护状态；不需要为此引入浏览器组件或迁移 UI 框架。

## 验收与分期

| 阶段 | 交付 | 估计 |
| --- | --- | --- |
| 1 | Windows CI、版本校验、自动生成说明、完整 Release 产物 | 1–2 天 |
| 2 | 后台检查/下载、版本比较、签名验证、关于页与设置 | 2–3 天 |
| 3 | 助手、静默安装重启、任务/草稿保护、安装失败恢复 | 3–4 天 |
| 4 | 两个版本间的真实升级、异常路径、文档与发布演练 | 2–3 天 |

核心测试覆盖：SemVer 与标签不一致、没有 Release、网络超时/限流、缺资产、损坏包/错签名、错误平台、取消/重复下载、磁盘不足、目录无权限、中文及带空格安装路径、旧进程未退出、其他数据目录实例、安装器失败及恢复、更新后版本/卸载信息正确、用户历史和草稿保留、CLI/Git 任务不会被后台更新中断。

发布演练必须使用独立测试数据目录与测试 Release/更新源，完成版本 A → B → 验证历史/草稿 → 重启结果的闭环。纯单元测试或“启动安装器成功”不足以验收自动更新。此评估未执行应用构建、安装或升级测试。

后续增强：断点续传、beta 通道、便携版更新、代理/镜像策略、差分包、跨版本累积 changelog、带数据迁移约束的健康回滚。首版使用全量安装包。

## 官方依据

2026-11-02 读取并核实：GitHub REST Releases、GitHub REST rate limits、ureq 3.5.0 文档、NSIS 用户手册第三章、Velopack 官网与 Rust crate 文档。引用页面名称作为实现时复核入口；本地结论以以上源文件为依据。

- GitHub：`GET /repos/{owner}/{repo}/releases/latest` 文档排除 draft/prerelease；仍需客户端比较版本。Release body 支持发布说明，支持生成 release notes API。
- GitHub rate limits：匿名主限额按 IP 为每小时 60 请求。
- ureq 3.5.0：TLS 是可选能力；当前工程配置没有启用其 TLS 后端。
- NSIS Chapter 3：静默参数 `/S` 及安装目录 `/D=` 的命令行约束。
- Velopack Rust：`UpdateManager`、`check_for_updates`、`download_updates`、`apply_updates_and_restart`；需集成早期启动生命周期并用 vpk 打包。

复核地址：`https://docs.github.com/en/rest/releases/releases`、`https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api`、`https://docs.rs/ureq/3.5.0/ureq/`、`https://nsis.sourceforge.io/Docs/Chapter3.html`、`https://docs.velopack.io/`、`https://docs.rs/velopack/latest/velopack/`。
