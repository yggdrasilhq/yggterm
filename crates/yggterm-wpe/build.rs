fn main() {
    // Debian sid: all of these live in the default linker search path,
    // installed by libwpewebkit-2.0-dev, libwpebackend-fdo-1.0-dev and
    // libgles-dev.
    for lib in [
        "WPEWebKit-2.0",
        "WPEBackend-fdo-1.0",
        "wpe-1.0",
        "EGL",
        "GLESv2",
        "gobject-2.0",
        "glib-2.0",
    ] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
