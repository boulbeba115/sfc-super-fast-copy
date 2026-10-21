fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/superfastcopy.ico");

    // Rust's self-contained MinGW toolchain ships only some Windows import
    // libraries; the extra ones the GUI needs are kept in .cargo/gnu-libs.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".cargo").join("gnu-libs");
        println!("cargo:rustc-link-search=native={}", dir.display());
    }

    // App icon and version info. On the GNU toolchain this needs windres
    // (from WinLibs, put on PATH by build.ps1); without it the exe just
    // gets the default icon.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/superfastcopy.ico");
        res.set("FileDescription", "Super Fast Copy");
        res.set("ProductName", "Super Fast Copy");
        if let Err(e) = res.compile() {
            println!("cargo:warning=app icon not embedded: {e}");
        }
    }
}
