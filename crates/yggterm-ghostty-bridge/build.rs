use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=GHOSTTY_DIR");
    println!("cargo:rerun-if-env-changed=GHOSTTY_LIB_DIR");

    let ghostty_dir = env::var_os("GHOSTTY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../ghostty"));

    let header = ghostty_dir.join("include").join("ghostty.h");
    if header.exists() {
        println!(
            "cargo:rustc-env=YGGTERM_GHOSTTY_HEADER={}",
            header.display()
        );
    }

    if let Some(lib_dir) = env::var_os("GHOSTTY_LIB_DIR") {
        let dir = PathBuf::from(lib_dir);
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
}
