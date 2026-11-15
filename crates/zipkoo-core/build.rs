fn main() {
    // UnRAR (via unrar_sys) calls registry, token and CryptoAPI functions on Windows.
    // Link advapi32 explicitly instead of relying on another crate to pull it in.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
