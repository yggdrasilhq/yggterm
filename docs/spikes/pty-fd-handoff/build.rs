fn main() {
    // forkpty(3) lives in libutil. Everything else is plain libc, which Rust
    // already links.
    println!("cargo:rustc-link-lib=dylib=util");
}
