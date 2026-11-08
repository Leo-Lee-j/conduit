# GitHub 插件目录

[English](github-catalog.md)

在 **设置 → 插件 → 发现** 浏览官方插件，以及带有 `conduit-plugin` Topic 的 GitHub 公共仓库。可搜索仓库名称与描述，或直接输入 `owner/repository`、GitHub 仓库首页地址。在 **已安装** 中启停插件、使用图标或卸载；仍可从本地文件夹安装。

当前可安装类型为 `file-icons`，格式见[文件图标插件](icon-themes.zh-CN.md)。目录可以声明其他类型，但应用支持前无法安装。包清单新增可选的 `author`、`license`、`min_app_version` 字段，最低应用版本采用语义化版本。没有许可证信息时显示“未声明许可证”；公共仓库或 Topic 本身不代表授权再分发。

## 官方与社区来源

官方来源固定为 `Leo-Lee-j/conduit`，直接读取目录，只有该仓库显示“官方”标识。应用附带 [Outline Icons](../../plugins/outline-icons/README.zh-CN.md) 的离线副本，安装后仍需手动选择“使用图标”。社区清单无法自行声明官方身份，也不能使用宿主保留的 `conduit.*` 命名空间。

社区搜索固定使用 `topic:conduit-plugin fork:false archived:false`。打开“发现”或搜索／刷新时请求 GitHub，每页读取六个仓库，点击“加载更多仓库”翻页；不承诺突破 GitHub 搜索限制获得完整目录。图标仓库可额外添加 `conduit-icon-theme` Topic。

缺失或损坏的插件清单会显示来源错误，不影响其他仓库。成功获取的页面缓存在应用状态文件旁的 `plugin-catalog.json`；网络失败时保留缓存结果，不影响已安装插件。公共请求不要求配置 GitHub token，API 限流时仍可能无法刷新。缓存保留原始 commit，仓库被删除后可能无法再安装。

## 单插件仓库

在仓库根目录放置 `conduit-plugin.json` 及其引用的 SVG。没有目录清单时，应用直接读取根目录中的插件包。

## 同一仓库包含多个插件

根目录放置 `conduit-plugins.json`：

```json
{
  "schema_version": 1,
  "plugins": [
    {
      "id": "publisher.outline-icons",
      "name": "Outline Icons",
      "version": "1.0.0",
      "description": "源码、配置与文档的文件图标",
      "kind": "file-icons",
      "path": "plugins/outline-icons",
      "revision": "icons-v1.0.0",
      "min_app_version": "0.2.6"
    }
  ]
}
```

`path` 相对于仓库根目录，空字符串表示根目录。每个目录有独立的 `conduit-plugin.json`，其 `id` 和 `version` 必须与目录项一致。`kind` 必填；`description`、`path`、`revision`、`license`、`min_app_version` 可省略。目录最多 32 项、256 KiB，仓库内的插件 ID 不可重复。

省略 `revision` 时，使用读取目录的同一个 commit；指定 tag 或分支时，先解析为完整 commit，再从该 commit 读取全部清单与图标，避免下载过程中分支变化导致版本混合。只下载声明的文件，不需要克隆仓库或解压压缩包。

## 安装与发布

下载设置时间与大小限制。瞬时连接或响应中断最多重试一次，并受总时限限制；限流及内容校验错误不重试。身份、版本兼容性、路径和 SVG 校验通过后，复用本地安装器暂存并发布安装目录。`.conduit-source.json` 保存仓库、commit、包路径和规范化清单及图标数据的 SHA-256 摘要；摘要用于记录内容身份，不是发布者签名。本地修改包内容后，旧安装记录不再显示为匹配来源。

同 ID 插件不会被静默覆盖。当前实现安装和卸载，尚未提供自动升级及回滚；更换版本需要先卸载旧副本。停用或卸载当前图标主题会恢复内置图标。

先发布目录和插件文件，再给仓库添加 Topic。远程目录不可用时，仍可安装应用附带的官方副本。插件不需要独立的 GitHub Release；未来采用 Release 分发时，应使用独立的插件 tag／工作流并设置 `make_latest=false`，避免干扰读取 Latest Release 的应用更新。
