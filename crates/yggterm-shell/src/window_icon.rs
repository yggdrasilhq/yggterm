use std::io::Cursor;
use tao::window::Icon;
use yggterm_core::YGGTERM_ICON_ASSETS;

pub fn load_window_icon_from_png(png_bytes: &[u8], asset_name: &str) -> Icon {
    let decoder = png::Decoder::new(Cursor::new(png_bytes));
    let mut reader = decoder
        .read_info()
        .unwrap_or_else(|_| panic!("decode {asset_name} icon metadata"));
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .unwrap_or_else(|_| panic!("decode {asset_name} icon pixels"));
    assert!(
        matches!(info.color_type, png::ColorType::Rgba),
        "{asset_name} window icon must be RGBA"
    );
    assert!(
        matches!(info.bit_depth, png::BitDepth::Eight),
        "{asset_name} window icon must use 8-bit channels"
    );
    Icon::from_rgba(
        buffer[..info.buffer_size()].to_vec(),
        info.width,
        info.height,
    )
    .unwrap_or_else(|_| panic!("construct {asset_name} window icon"))
}

pub fn load_yggterm_window_icon() -> Icon {
    load_window_icon_from_png(YGGTERM_ICON_ASSETS.png_512_bytes, "yggterm")
}
