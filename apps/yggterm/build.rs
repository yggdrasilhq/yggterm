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
    let mut buffer = vec![0; reader.output_buffer_size()];
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

fn main() {
    let root = repo_root();
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
    resource.compile().expect("compile Windows resources");
}
