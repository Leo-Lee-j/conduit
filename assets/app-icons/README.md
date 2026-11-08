# Conduit application icons

`source.png` is the original, user-provided **480 × 480** PNG logo, supplied as
`icon.png` on November 8, 2026. Its colors, proportions, and transparent corners
are preserved. This file is the source for all generated application icons.

## Regenerate with the Rust CLI

The checked-in icons were generated with **Tauri CLI 2.12.4**, a Rust tool.
Install that version and run the generator from the repository root:

```powershell
cargo install tauri-cli --version 2.12.4 --locked
cargo tauri icon assets/app-icons/source.png --output assets/app-icons
```

For this refresh, the official prebuilt `cargo-tauri.exe` for
`x86_64-pc-windows-msvc` was used. Its release archive SHA-256 was verified before
running it. The equivalent direct command is:

```powershell
cargo-tauri.exe icon assets/app-icons/source.png --output assets/app-icons
```

The executable is available from the [Tauri CLI 2.12.4 release](https://github.com/tauri-apps/tauri/releases/tag/tauri-cli-v2.12.4).
The CLI is only used to generate image assets; the application uses GPUI.

## Where the icons are used

- `build.rs` embeds `icon.ico` as Windows executable resource ID 1. GPUI uses that
  resource for the window and taskbar icon.
- The Windows installer and uninstaller also use `icon.ico`.
- The application title bar, startup page, and About dialog embed `source.png`.
  Rust resizes it directly to the actual device-pixel size with Lanczos filtering,
  preserving transparency with premultiplied-alpha filtering. Painting is aligned
  to device pixels to avoid another fractional scaling pass. A 16-entry LRU
  shares the resulting images across views and DPI settings.
- The project READMEs use `128x128@2x.png`. Application icons are embedded at
  compile time; no external icon files are needed at runtime.
- The remaining PNG, ICNS, iOS, and Android assets are generated together to keep
  the existing asset set consistent. Their presence does not imply support for
  those platforms; Conduit currently ships for Windows x64.

Sizes above 480 pixels are resampled from the supplied source, not new
higher-resolution artwork. To improve those variants, replace `source.png` with
a higher-resolution original and regenerate the set.
