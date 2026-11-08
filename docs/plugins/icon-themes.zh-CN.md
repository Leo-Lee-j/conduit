# 文件图标插件

[English](icon-themes.md)

打开 **设置 → 插件 → 从文件夹安装**，选择包含 `conduit-plugin.json` 的文件夹，再点击已安装插件的 **使用图标**。仓库附带可直接安装的 [Outline Icons 示例](../../examples/plugins/outline-icons)。

安装会将插件复制到应用状态文件旁的 `plugins` 目录。修改或删除原始文件夹不影响已安装副本。停用当前图标插件会立即恢复默认图标；卸载只删除安装副本，保留原始文件夹。启停状态和所选主题会保存，重启后继续生效。刷新可重新读取安装目录，损坏的插件会显示加载错误，不影响其他插件。

图标主题作用于文件树，文件标签页和附件图标维持现有样式。文件夹的展开箭头独立于主题图标。

在 **设置 → 插件 → 发现** 可在线查找并安装官方与社区插件，详见 [GitHub 插件目录](github-catalog.zh-CN.md)。包清单支持可选的 `author`、`license`、`min_app_version` 元数据。

## 清单

```json
{
  "schema_version": 1,
  "id": "example.my-icons",
  "name": "我的图标",
  "version": "1.0.0",
  "description": "工作区文件图标",
  "file_icons": {
    "icons": {
      "code": {"path": "icons/code.svg", "light": "#285d9c", "dark": "#8bb6e8"},
      "folder": {"path": "icons/folder.svg"}
    },
    "folder": "folder",
    "file_names": {"dockerfile": "code"},
    "file_extensions": {"rs": "code", "d.ts": "code"},
    "folder_names": {"src": "folder"}
  }
}
```

插件 ID 使用 `publisher.plugin-name` 形式，`conduit.*` 为保留命名空间，版本遵循语义化版本。本格式只接受声明式文件图标贡献，不接受脚本入口或未知字段。实验中的 Shell 插件使用独立清单，仍通过开发工具加载。

| 字段 | 用途 |
| --- | --- |
| `icons` | 图标定义：相对 SVG 路径，以及可选的浅色／深色 `#RRGGBB` 颜色 |
| `file` | 默认文件图标；省略时回退到应用默认图标 |
| `folder` / `folder_expanded` | 文件夹收起／展开图标 |
| `file_names` | 完整文件名匹配，支持点文件 |
| `file_extensions` | 不含开头点号的扩展名，支持 `d.ts` 等复合后缀 |
| `folder_names` / `folder_names_expanded` | 指定文件夹名称的收起／展开图标 |

匹配键使用小写；匹配时忽略大小写。文件依次尝试完整文件名、最长扩展名、主题默认值、应用默认值。展开的文件夹依次尝试展开名称映射、普通名称映射、展开默认值、普通默认值。

SVG 按单色形状渲染，使用清单中对应主题的颜色；未指定颜色时使用应用次要文字色。必须提供 `viewBox`，支持 `svg`、`g`、`path`、`rect`、`circle`、`ellipse`、`line`、`polyline`、`polygon`、`title`、`desc` 和内联绘图属性。不支持文字、脚本、CSS、引用图片、渐变、滤镜及外部资源。导出为简单描边或填充图形即可。

限制：清单 128 KiB、128 个图标、单个 SVG 64 KiB、总图标数据 2 MiB、1,024 条映射、64 个安装包。路径必须在插件目录内，引用的图标必须已定义。重复 ID 会被拒绝；更新时先卸载旧副本，再安装新版本。

## 实现

每个启用的主题以插件所有者身份向工作台注册 `file-icons` 贡献；停用和卸载会撤销注册。加载和校验在后台完成，图标缓存在内存中，文件树渲染不读取磁盘。所选主题丢失或无效时回退到默认图标。安装先写入暂存目录，全部校验完成后发布，不执行插件脚本。
