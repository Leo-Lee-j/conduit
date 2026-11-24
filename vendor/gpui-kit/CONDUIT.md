# Conduit GPUI patches

This directory contains `gpui-base` and `gpui-component` from upstream
`longbridge/gpui-kit`, revision `11345ec002d9f182a266b5d5dd3a217090700a87`.
The upstream Apache 2.0 license is retained in `LICENSE-APACHE`. Files modified
for Conduit are listed below; all other crate files are the upstream snapshot.

Conduit's root `Cargo.toml` patches those two packages with repository-relative
paths. The umbrella Kit, assets, macros, and GPUI versions remain pinned to the
original revision/versions. Ordinary `cargo build --locked` works from a clean
checkout without changing Cargo's cache or running a preparation script.

## Local changes

- `crates/base/src/text/style.rs`: optional quote/list/list-item refinements,
  list indentation, body-column text refinements, and a shared per-URL link
  decoration and SVG icon resolvers. Defaults retain upstream behavior.
- `crates/base/src/text/text_view.rs`: optional view builders applied after
  theme/style resolution.
- `crates/base/src/text/node.rs`: applies refinements to existing semantic
  nodes, shares list indentation across markers/nesting/continuations, and
  measures table cells with the same text style that paints them. Table cache
  invalidation includes column and link typography. Links retain native text,
  source offsets, selection, wrapping, and click handling.
- `crates/component/src/text/compat.rs`: forwards the builders to Base without
  adding fields to Component's public `TextViewStyle` struct.
- `crates/base/src/text/state.rs`: retains the parsed link-reference context.
  Documents containing reference definitions reparse the complete source on
  append so earlier/later references resolve correctly; ordinary documents
  retain the tail-only parse path. The parser reports the actual full-parse
  status so highlights/reveal requests are reconciled across all blocks and
  cached heights are invalidated. Source appends retain selection compatibility
  and splice added rows to preserve the logical scroll anchor.
- `crates/base/src/text/inline_flow.rs`: reserves and paints a leading SVG
  decoration at the link label's first grapheme. The icon shares wrapping and
  table measurement with native text without introducing document bytes or an
  atomic label. Layout caches include icon identity; click and drag remain distinct.
- `crates/base/src/accordion.rs`: makes enabled accordion triggers keyboard
  tab stops and exposes the disabled state to accessibility tools.
- `crates/component/src/accordion.rs`: sends toggle callbacks from the trigger
  so that clicking a control inside the panel does not toggle the accordion.
- `crates/component/src/setting/item.rs`: exposes the existing field renderer
  for use without group borders inside an accordion.
- `crates/component/src/setting/group.rs`: optional style for the shared content
  surface, leaving the group heading and footer outside it.
- `crates/component/src/setting/settings.rs`: omits the resizable sidebar and
  its divider when the host sets the sidebar width to zero.
- `crates/component/src/icon.rs`: applies the runtime icon scale to SVG paint
  transforms while preserving each icon's layout and hitbox.
- `crates/component/src/theme/mod.rs`: stores the runtime icon scale outside
  serialized theme files.
- `Cargo.toml`: trims the upstream workspace to these two crates and pins
  references to the remaining Kit crates to the original Git revision.

The body-column API deliberately accepts text refinements only. It does not
offer per-cell padding, backgrounds, or a general row/column layout callback.
The application opts file links into the existing FileText asset; other links
keep their default appearance. The icon size is 0.9em with a 0.25em gap.

## Maintenance

`upstream-files.json` records SHA-256 hashes of every copied crate file and
the exact upstream revision. `python scripts/check-gpui-vendor.py` checks that
only the declared files differ and that the application, vendor
workspace, and lockfile still select the intended dependency graph.

When upgrading Kit, replace both crate snapshots from one revision, reapply
the local changes, regenerate the upstream hashes from the pristine
revision, and run the Markdown selection/search/streaming tests plus the core
checks. Do not edit the Cargo checkout or change only one crate's revision.
Once upstream provides equivalent APIs, remove these patches and this directory
in the same dependency update.
