# Conduit Windows renderer patch

This directory is the crates.io package `gpui-pre-windows` 0.3.9 (Zed's
`gpui_windows` at `93d745668805029476e6d7fbb19652116fed51de`). The upstream
Apache 2.0 license is retained in `LICENSE-APACHE`. Conduit's root `Cargo.toml`
replaces the registry package through `[patch.crates-io]`.

## Local changes

- `src/directx_renderer.rs`: the path intermediate texture and its 4x MSAA
  target are created by the first path batch instead of with the swap chain.
  A frame with no paths releases them after 10 seconds without path drawing,
  and a resize drops them so the next path batch creates them at the new size.
  At 2560x1392 the two textures take about 68 MiB of GPU memory, which an
  integrated GPU counts as process private memory. Rendered output is unchanged.
- `src/platform.rs`: Explorer reveal initializes and balances COM on the calling
  worker thread (while respecting an existing apartment), selects a single
  absolute shell item, frees its PIDL, and falls back to opening the parent even
  when parsing fails. Paths are made absolute and stripped of unnecessary
  verbatim prefixes before passing them to the shell.

The native Explorer regression is opt-in because it opens real shell windows.
From the repository root, run:

```powershell
cargo test --locked -p conduit -p gpui-pre-windows --features conduit/ui-tests --lib test_reveal_path_from_background_thread -- --ignored --nocapture --test-threads=1
```

## Updating

Copy the new package from the Cargo registry over this directory, then
reapply the changes above. Remove the patch entry once upstream includes these
fixes.
