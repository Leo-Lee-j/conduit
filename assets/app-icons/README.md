# Conduit application icons

Generated from the user-provided Conduit logo. `source.png` is the cropped,
1024 x 1024 PNG with transparent rounded corners. Regenerate this directory with:

```powershell
npx --yes @tauri-apps/cli icon assets/app-icons/source.png --output assets/app-icons
```

`build.rs` embeds `icon.ico` as Windows executable resource ID 1, which GPUI also
uses for the window and taskbar. The title bar and About dialog embed
`128x128@2x.png`; no external icon files are needed at runtime.
