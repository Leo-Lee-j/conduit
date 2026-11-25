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

## Updating

Copy the new package from the Cargo registry over this directory, then
reapply the change above. Remove the patch entry once upstream allocates these
textures on demand.
