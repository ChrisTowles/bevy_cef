fn main() {
    // Linux: libcef.so sits beside the render process (both are copied next to the app), and
    // nothing puts that directory on the loader's path.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
}
