fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/app-icons/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resources();
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // GPUI's debug render builders need more headroom than the default
        // Windows main-thread reserve. This reserves address space; it does
        // not eagerly commit 8 MiB of physical memory.
        println!("cargo:rustc-link-arg-bin=conduit=/STACK:8388608");
    }
}

fn embed_windows_resources() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let icon = root.join("assets/app-icons/icon.ico");
    let icon = icon.to_string_lossy().replace('\\', "/");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let numeric_version = ["MAJOR", "MINOR", "PATCH"]
        .map(|part| std::env::var(format!("CARGO_PKG_VERSION_{part}")).unwrap())
        .join(",");
    let resource = output.join("conduit.rc");
    // GPUI loads the window/taskbar icon from executable resource ID 1.
    // The dependency already embeds the DPI-aware application manifest.
    std::fs::write(
        &resource,
        format!(
            r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric_version},0
PRODUCTVERSION {numeric_version},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "FileDescription", "Conduit"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "conduit"
            VALUE "OriginalFilename", "conduit.exe"
            VALUE "ProductName", "Conduit"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#
        ),
    )
    .expect("write Windows resources");
    embed_resource::compile_for(resource, ["conduit"], embed_resource::NONE)
        .manifest_required()
        .expect("compile Conduit icon and version resources");
}
