fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // GPUI's debug render builders need more headroom than the default
        // Windows main-thread reserve. This reserves address space; it does
        // not eagerly commit 8 MiB of physical memory.
        println!("cargo:rustc-link-arg-bin=conduit=/STACK:8388608");
    }
}
