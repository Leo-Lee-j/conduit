# Bundled themes

`conduit.json` contains Conduit's original light and dark themes.

`conduit-lights.json` and `conduit-darks.json` contain Conduit's paired themes:

| Light | Dark |
| --- | --- |
| Graphite & Indigo | Graphite & Indigo Dark |
| Warm Paper & Terracotta | Warm Charcoal & Terracotta |
| Slate & Teal | Deep Slate & Teal |

The dark variants use near-black tinted backgrounds, stepped panel surfaces,
bright accents with dark button text, and deep status fills with lighter text.
Shadows are disabled in favor of surface and border contrast.
Project markers brighten in dark mode without changing their saved colors.

The other JSON files are copied without modification from GPUI Kit's `themes/`
directory at revision `11345ec002d9f182a266b5d5dd3a217090700a87`:
https://github.com/longbridge/gpui-kit/tree/11345ec002d9f182a266b5d5dd3a217090700a87/themes

These GPUI Kit files are distributed under Apache-2.0; see `LICENSE-APACHE`.
Each file retains the upstream author and original theme source URL. Theme
definitions and syntax highlighting remain unchanged; Conduit maps their
resolved colors onto its application palette and adjusts text contrast there.

All themes are embedded in the executable, so selection works offline and
does not depend on the launch directory.
