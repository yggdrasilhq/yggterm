use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=GHOSTTY_DIR");
    println!("cargo:rerun-if-env-changed=GHOSTTY_LIB_DIR");

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let ghostty_dir = env::var_os("GHOSTTY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            manifest_dir
                .join("../../../ghostty")
                .canonicalize()
                .unwrap_or_else(|_| manifest_dir.join("../../../ghostty"))
        });

    let header = ghostty_dir.join("include").join("ghostty.h");
    if header.exists() {
        println!(
            "cargo:rustc-env=YGGTERM_GHOSTTY_HEADER={}",
            header.display()
        );
    }

    if let Some(lib_dir) = env::var_os("GHOSTTY_LIB_DIR") {
        let dir = PathBuf::from(lib_dir);
        println!("cargo:rustc-env=YGGTERM_GHOSTTY_LIB_DIR={}", dir.display());
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
    } else if let Some(dir) = discover_ghostty_lib_dir(&ghostty_dir) {
        println!("cargo:rustc-env=YGGTERM_GHOSTTY_LIB_DIR={}", dir.display());
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
    }
}

fn discover_ghostty_lib_dir(ghostty_dir: &PathBuf) -> Option<PathBuf> {
    discover_ghostty_lib_in_dir(ghostty_dir, 4).and_then(|path| path.parent().map(PathBuf::from))
}

fn discover_ghostty_lib_in_dir(dir: &PathBuf, depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }

    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "libghostty.so")
        {
            return Some(path);
        }

        if path.is_dir()
            && let Some(found) = discover_ghostty_lib_in_dir(&path, depth.saturating_sub(1))
        {
            return Some(found);
        }
    }

    None
}
