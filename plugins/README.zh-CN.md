# Conduit 官方插件

[English](README.md)

此目录维护随 Conduit 开发的官方插件。仓库根目录的 `conduit-plugins.json` 提供发现目录；各子目录拥有独立的 `conduit-plugin.json` 和版本号。

| 插件 | ID | 类型 |
| --- | --- | --- |
| [Outline Icons](outline-icons/README.zh-CN.md) | `conduit-plugins.outline-icons` | 文件图标 |

打开 **设置 → 插件 → 发现** 安装官方主题。应用附带可离线安装的副本，用户安装后再到 **已安装** 选择 **使用图标**，不会自动改变当前主题。

发布时一起提交目录清单和插件文件。目录项未指定 `revision` 时，使用读取目录清单时的完整 commit；独立发版可指定包含插件的 Git tag 或完整 commit SHA。安装器会将 tag 解析为固定 commit 后下载。

社区仓库添加 GitHub Topic `conduit-plugin` 后可被搜索，图标主题可额外添加 `conduit-icon-theme`。官方目录使用固定仓库来源，不依赖 Topic 搜索。详见[发布与目录协议](../docs/plugins/github-catalog.zh-CN.md)。

当前无需单独创建插件 Release。以后使用 Release 分发时，应与应用的 `v*` 发布流程分开，且不要设为仓库 Latest Release，以免干扰应用更新。
