fn main() {
    println!("cargo:rustc-check-cfg=cfg(conduit_shell_desktop)");
    println!("cargo:rustc-cfg=conduit_shell_desktop");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bin=conduit-shell-desktop=/STACK:8388608");
    }
}
