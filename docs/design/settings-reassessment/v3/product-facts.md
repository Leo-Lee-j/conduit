# 成熟产品调研记录

核对日期：2026-11-26。以官方文档及文档附图为证据，观察的是该日期取得的页面，不声称截图等于软件最新构建。以下“借鉴”均为本次设计判断。

## 1. JetBrains IntelliJ IDEA：分类树负责定位

官方页面：`https://www.jetbrains.com/help/idea/settings-preferences-dialog.html`

标题 Settings，抓取页标为 IntelliJ IDEA 2026.3 Help，页面日期 20 November 2026。已打开官方截图 `refs/jetbrains-settings.png`，1890×1400；图片来源记录在 `refs/image-sources.json`。

文档明确提供搜索及层级分类列表，项目设置有范围标记；修改使用 OK / Apply / Cancel。截图可见左侧搜索和分类树、右侧分类路径与内容、底部操作区。图片量色：中性底色两类合计约94%，彩色区域约2%；它靠结构和选中态组织内容。

借鉴：导航、搜索结果和分类路径保持一致，用户知道当前编辑属于哪里。Conduit 的11类导航保持稳定，并为字段提供可定位标识。

适配判断：不照搬全局 Apply/OK。Conduit 偏好自动保存，只有服务与配置编辑需要提交；把它们混成全局待保存会改变产品语义。也不继承无限深分类树，默认最多“分组 → 页面 → 页内章节”。

## 2. Visual Studio Code：搜索是设置的主要工具

官方页面：`https://code.visualstudio.com/docs/configure/settings`

标题 User and workspace settings。已看 `refs/vscode-settings.png`、`refs/vscode-filter.png`，分别1285×466、2270×1170。

官方文档说明：搜索会过滤设置；偏好修改直接应用；设置可按分类查看；`@modified`用于查看改变的设置，也包含在对应JSON中显式设置的项；设置菜单支持恢复默认和复制设置标识/链接。截图显示搜索、User/Workspace范围及分类树共处一个编辑器。彩色竖线辅助标识改动。

借鉴：只保留一个全局搜索入口，结果包含分类路径与字段命中；用明确的过滤条件替代重复搜索框。偏好字段可有“已修改”，支持局部恢复。

适配判断：Conduit“已修改”继续指值偏离默认值，不照搬VS Code对显式JSON配置的判定。也不把User/Workspace两级作用域复制到所有设置；Conduit只有部分扩展支持全局/工作区范围。中文设置名优先，技术字段放辅助层。

## 3. Raycast：配置围绕对象归属

官方页面：`https://manual.raycast.com/settings`；辅助页：`https://manual.raycast.com/extensions`。

已看 `refs/raycast-settings.png`、`refs/raycast-agents.png`，均2100×1580。附图展示macOS版本；本次只迁移结构，不复用Mac窗体、系统图标或磨砂材料。

官方说明：设置搜索可检索设置、命令、扩展并跳到位置；已安装扩展在侧栏有自己的入口，选择后查看命令与配置，操作包含停用/启用及卸载。截图里，侧栏对象与右侧配置内容具有稳定的所属关系，组内以紧凑行显示设置。图片量色显示中性深灰主导；图中背景照片是截图环境，不是我们建议的设置背景。

借鉴：引擎、服务、Skill、MCP、插件使用同一种“选择对象 → 查看来源、状态、配置”的模式，对象的动作贴近对象。安装目录与已安装管理分开。

适配判断：不把所有扩展无限塞入Conduit全局导航；11类仍是稳定入口，具体实例留在当前页列表。安装/探测/编辑的状态不得混为一个绿点。

## 辅助核对与未采用来源

Zed 官方 `https://zed.dev/docs/configuring-zed` 在核对时重定向至 `https://zed.dev/docs/reference/all-settings`。确认其提供设置编辑器和配置标识参考；该页未提供适合本次比较的完整UI截图，故不把它列为已完成视觉研究的主参照。

Linear 的候选旧地址 `/docs/preferences` 返回404，未据此作视觉判断，也未使用记忆中的截图代替证据。

## 研究结论（设计判断）

成熟工具可借鉴的不是“深色”“小字号”，而是稳定的定位方式、清楚的对象归属、准确的生效范围和统一的编辑规则。本次建议：独立设置视图 + 固定页面身份 + 四类模板 + 一个搜索入口。范围、默认、连接、保存各有自己的语义。

参考图片仅供设计评审，不作为Conduit产品素材发布。官方图片来源、下载尺寸保存在 `refs/image-sources.json`；四图参考板为 `refs/reference-board.jpg`。
