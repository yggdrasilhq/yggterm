use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("resolve repo root")
}

fn decode_png_rgba(path: &Path) -> (u32, u32, Vec<u8>) {
    let bytes = fs::read(path).expect("read app icon png");
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().expect("decode app icon metadata");
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .expect("app icon png output buffer size")
    ];
    let info = reader
        .next_frame(&mut buffer)
        .expect("decode app icon pixels");
    (
        info.width,
        info.height,
        buffer[..info.buffer_size()].to_vec(),
    )
}

fn write_windows_icon(icon_png: &Path, out_dir: &Path) -> PathBuf {
    let (width, height, rgba) = decode_png_rgba(icon_png);
    let image = ico::IconImage::from_rgba_data(width, height, rgba);
    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);
    icon_dir.add_entry(ico::IconDirEntry::encode(&image).expect("encode ico entry"));
    let out_path = out_dir.join("yggterm.ico");
    let mut file = fs::File::create(&out_path).expect("create ico output");
    icon_dir.write(&mut file).expect("write ico output");
    out_path
}

/// ⛔ A VERSION NUMBER IS NOT AN IDENTITY. On 2026-08-13 four consecutive
/// numbers each meant two builds: parallel clusters read `Cargo.toml`, add one,
/// and push, so whoever pushes second wears a string the first already spent.
/// A deploy from a pre-rebase tree then lands over another cluster's fix, the
/// GUI re-execs (the pid is unchanged, so `/proc/<pid>/exe` still reads clean),
/// and that cluster's live probe comes back RED against a binary that never
/// carried its fix. The obvious reading — "my root cause was wrong" — is the
/// most expensive wrong conclusion available.
///
/// So the binary states the commit it was built from, and the fleet census
/// prints it. ⛔ `--version` is left byte-identical on purpose: it is a
/// rendezvous key (socket names, the daemon version gate, and
/// `yggterm_executable_reported_version`'s token scan all resolve against it),
/// and widening it would change what those match.
fn stamp_build_commit(root: &Path) {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    };

    // A packaged crate or a source tarball has no `.git` at all, and that is a
    // legitimate build — it says `unknown` rather than failing.
    let commit = match git(&["rev-parse", "--short=12", "HEAD"]) {
        Some(commit) => {
            // Best-effort: no file changes when the tree goes dirty, so this
            // can lag. The deploy verb is where dirt is actually caught.
            let dirty = git(&["status", "--porcelain"]).is_some();
            if dirty {
                format!("{commit}-dirty")
            } else {
                commit
            }
        }
        None => "unknown".to_string(),
    };
    println!("cargo:rustc-env=YGGTERM_BUILD_COMMIT={commit}");

    // Re-stamp when the checked-out commit moves. A commit and a rebase both
    // rewrite HEAD's ref file, which is what makes the stamp honest; only paths
    // that exist are named, because naming a missing one reruns this script on
    // every build for no gain.
    for path in ["HEAD", "packed-refs"] {
        if let Some(resolved) = git(&["rev-parse", "--git-path", path]) {
            let resolved = root.join(resolved);
            if resolved.exists() {
                println!("cargo:rerun-if-changed={}", resolved.display());
            }
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "--quiet", "HEAD"])
        && let Some(resolved) = git(&["rev-parse", "--git-path", &reference])
    {
        let resolved = root.join(resolved);
        if resolved.exists() {
            println!("cargo:rerun-if-changed={}", resolved.display());
        }
    }
}

fn main() {
    let root = repo_root();
    stamp_build_commit(&root);
    let icon_png = root.join("assets/brand/yggterm-icon-512.png");
    println!("cargo:rerun-if-changed={}", icon_png.display());

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bin=yggterm=/SUBSYSTEM:WINDOWS");
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let icon_ico = write_windows_icon(&icon_png, &out_dir);
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon_ico.to_string_lossy().as_ref());
    resource.set("ProductName", "Yggterm");
    resource.set("FileDescription", "Remote-first terminal workspace");
    resource.set("InternalName", "yggterm");
    resource.set("OriginalFilename", "yggterm.exe");
    resource.set("CompanyName", "Yggdrasil HQ");
    if let Err(error) = resource.compile() {
        println!(
            "cargo:warning=skipping Windows resource metadata/icon because resource compilation failed: {error}"
        );
    }
}
