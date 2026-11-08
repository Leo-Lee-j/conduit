# File icon plugins

[简体中文](icon-themes.zh-CN.md)

Conduit can install declarative file icon themes from local folders. Open **Settings → Plugins → Install from folder**, select a folder containing `conduit-plugin.json`, then choose **Use icons** on the installed plugin.

Plugins are copied into the `plugins` directory beside Conduit’s state file. Editing or deleting the source folder does not change an installed plugin. **Disable** immediately restores the default file icons when that plugin is selected. **Uninstall** removes the installed copy and preserves the source folder. Enablement and the selected theme are saved across restarts. **Refresh** reloads installed packages and reports malformed packages without hiding healthy ones.

The included [Outline Icons example](../../examples/plugins/outline-icons) is ready to install. File icons are applied to the file tree; document tabs and attachment icons retain their existing appearance. The tree’s expand/collapse arrow is independent of the folder icon.

Online discovery and the official plugin are available in **Settings → Plugins → Discover**. See the [GitHub catalog guide](github-catalog.md). Package manifests also accept optional `author`, `license`, and `min_app_version` metadata.

## Manifest

```json
{
  "schema_version": 1,
  "id": "example.my-icons",
  "name": "My Icons",
  "version": "1.0.0",
  "description": "File icons for my workspace",
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

IDs follow the contribution namespace rules (`publisher.plugin-name`); `conduit.*` is reserved. Versions use semantic versioning. This package format accepts only file icon contributions; unknown fields, including executable entry points, are rejected. The experimental Shell runtime has its own manifest and remains a separate development feature.

`file_icons` supports:

| Field | Meaning |
| --- | --- |
| `icons` | Named SVG definitions, each with a relative `path` and optional light/dark `#RRGGBB` colors |
| `file` | Default file icon; omitted values fall back to Conduit |
| `folder`, `folder_expanded` | Default closed/open folder icons |
| `file_names` | Exact base names, including dotfiles |
| `file_extensions` | Suffixes without the leading dot; compound suffixes such as `d.ts` are supported |
| `folder_names`, `folder_names_expanded` | Exact closed/open folder names |

Association keys must be lowercase. Matching is case insensitive: exact file name, longest extension suffix, theme default, then Conduit default. Expanded folders prefer the expanded name mapping, then the normal name mapping, then the expanded/default folder icon.

SVGs render as monochrome shapes tinted with the selected light/dark color, or the theme’s secondary color when omitted. Include `viewBox`; use `svg`, `g`, `path`, `rect`, `circle`, `ellipse`, `line`, `polyline`, `polygon`, `title`, or `desc`. Use inline drawing attributes. Text, scripts, CSS, image references, gradients, filters, and external references are not supported. Export simple outline or filled shapes.

The host validates relative paths and referenced icon IDs. Package limits are 128 KiB of manifest JSON, 128 icons, 64 KiB per SVG, 2 MiB of total icon data, 1,024 name/extension associations, and 64 installed packages. A duplicate plugin ID is rejected; uninstall the current copy before installing a new version.

## Lifecycle

Each enabled theme contributes an owned `file-icons` descriptor to the workbench registry. Disabling or uninstalling revokes its registration. Icon data is loaded in the background and cached; rendering performs no file reads. Missing or invalid selected plugins fall back to Conduit’s icons. Package installation stages validated files before publishing the folder. No package scripts are executed.
