# GitHub plugin catalog

[简体中文](github-catalog.zh-CN.md)

Open **Settings → Plugins → Discover** to browse official plugins and public GitHub repositories with the `conduit-plugin` Topic. Search repository names/descriptions, or enter `owner/repository` or a root GitHub repository URL. Use **Installed** to enable/disable plugins, select file icons, or uninstall them. Local folder installation remains available.

The first supported package type is `file-icons`; see the [SVG package format](icon-themes.md). Other type names may appear in catalogs but cannot be installed until Conduit supports them. Packages may specify `author`, `license`, and a `min_app_version` semantic version. Missing license metadata is shown as unspecified; a public repository or Topic is not a license grant.

## Official and community sources

The official source is `Leo-Lee-j/conduit`, loaded directly. Only that repository receives the Official label. The app includes an offline copy of [Outline Icons](../../plugins/outline-icons/README.md), which can be installed without changing the current theme automatically. Community manifests cannot declare themselves official or claim the reserved `conduit.*` host namespace.

Community discovery uses GitHub repository search with `topic:conduit-plugin fork:false archived:false`. Fetching is explicit when opening Discover or searching/refreshing, with six repositories per page and a **Load more repositories** button. It does not promise exhaustive discovery beyond GitHub's search limit. Add `conduit-icon-theme` as an optional secondary Topic for icon packages.

A repository without a valid package or catalog is reported as unavailable; other repositories remain visible. The last successful pages are cached beside the application state file in `plugin-catalog.json`. Failed requests preserve cached entries, and catalog outages do not disable installed plugins. Public requests do not require a GitHub token; API rate limits can still prevent refreshing. Cached entries retain their original commit and can become unavailable if a repository is removed.

## Single-plugin repository

Place `conduit-plugin.json` at the repository root, together with its referenced SVG files. If there is no catalog index, Conduit reads this package directly.

## Multiple plugins in one repository

Place `conduit-plugins.json` at the repository root:

```json
{
  "schema_version": 1,
  "plugins": [
    {
      "id": "publisher.outline-icons",
      "name": "Outline Icons",
      "version": "1.0.0",
      "description": "File icons for source, configuration and documents",
      "kind": "file-icons",
      "path": "plugins/outline-icons",
      "revision": "icons-v1.0.0",
      "min_app_version": "0.2.6"
    }
  ]
}
```

`path` is relative to the repository root; an empty string means the root. Each directory contains its own `conduit-plugin.json`. `id` and `version` must match the package manifest. `kind` is required. `description`, `path`, `revision`, `license` and `min_app_version` are optional. The index accepts at most 32 entries and 256 KiB, and IDs must be unique within it.

If `revision` is omitted, the plugin is read from the same commit as the catalog. If it is a tag or branch, Conduit resolves it to a full commit once. Every manifest and icon download then uses that commit, so a branch changing during installation cannot mix files from different revisions. Only the declared files are downloaded; a Git clone and archive extraction are unnecessary.

## Installation and publication

Downloads have time and size limits. Transient connection/body failures receive at most one retry per request within the overall deadline; rate limits and invalid content are not retried. After package identity, application compatibility, paths and SVGs pass validation, installation publishes the staged directory through the existing local installer. The `.conduit-source.json` receipt records the repository, commit, package path and SHA-256 digest of the normalized manifest and icon data. This digest records content identity; it is not a publisher signature. Locally edited package content no longer displays a matching receipt.

Existing IDs are never silently replaced. This version supports installation and removal; automatic upgrades and rollback are not implemented. Uninstall an existing copy before installing another version. Disabling or uninstalling the active icon theme restores the built-in icons.

Publish the index and package files before advertising the Topic. The bundled official theme remains installable when the remote catalog is unavailable. A plugin does not require a GitHub Release. If Releases are introduced, use a separate plugin tag/workflow and keep `make_latest=false`: Conduit's application updater uses this repository's latest release.
