fn main() {
    // All four live in the default linker search path on Debian sid
    // (/usr/lib/x86_64-linux-gnu), installed by libwpewebkit-2.0-dev and
    // libwpebackend-fdo-1.0-dev.
    for lib in [
        "WPEWebKit-2.0",
        "WPEBackend-fdo-1.0",
        "wpe-1.0",
        "EGL",
        "gobject-2.0",
        "glib-2.0",
    ] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
}
