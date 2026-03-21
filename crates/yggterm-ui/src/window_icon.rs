use std::io::Cursor;
use tao::window::Icon;

const WINDOW_ICON_BYTES: &[u8] = include_bytes!("../../../assets/brand/yggterm-icon-512.png");

pub fn load_window_icon() -> Icon {
    let decoder = png::Decoder::new(Cursor::new(WINDOW_ICON_BYTES));
    let mut reader = decoder.read_info().expect("decode yggterm icon metadata");
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .expect("decode yggterm icon pixels");
    assert!(
        matches!(info.color_type, png::ColorType::Rgba),
        "yggterm window icon must be RGBA"
    );
    assert!(
        matches!(info.bit_depth, png::BitDepth::Eight),
        "yggterm window icon must use 8-bit channels"
    );
    Icon::from_rgba(buffer[..info.buffer_size()].to_vec(), info.width, info.height)
        .expect("construct yggterm window icon")
}
