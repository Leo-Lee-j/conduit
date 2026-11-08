# Official Conduit plugins

[简体中文](README.zh-CN.md)

This directory contains plugins maintained with Conduit. The repository-root `conduit-plugins.json` is the discoverable catalog; each subdirectory has its own `conduit-plugin.json` and version.

| Plugin | ID | Type |
| --- | --- | --- |
| [Outline Icons](outline-icons/README.md) | `conduit-plugins.outline-icons` | File icons |

Open **Settings → Plugins → Discover** to install the official theme. A bundled copy is available offline and is not enabled until you install it. Select **Use icons** in **Installed** to apply it.

Publish the catalog and plugin files together. A catalog entry without `revision` uses the exact commit from which the catalog was read. To version a plugin independently, set `revision` to a Git tag or full commit SHA containing that package. The installer resolves tags to a full commit before downloading files.

For community discovery, add the `conduit-plugin` GitHub Topic to a repository. `conduit-icon-theme` is an optional additional Topic. The official catalog is fetched directly and does not depend on Topic search. See [publishing and catalog format](../docs/plugins/github-catalog.md).

No plugin-specific GitHub Release is required. If introducing plugin Releases later, keep them separate from the application's `v*` release workflow and do not mark them as the repository's latest release.
