//! A minimal PNG writer — no dependencies, matching the crate's zero-dep stance.
//!
//! The spike crate is dependency-free so that the binding surface it measures
//! is the whole cost. Pulling in `png` + `flate2` to write an acceptance
//! artifact would hide that. PNG's IDAT is a zlib stream, and zlib permits
//! STORED (uncompressed) deflate blocks, so a valid PNG needs only CRC-32 and
//! Adler-32 — about sixty lines. The output is large; it is evidence, not a
//! shipping format.

fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc = table[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    let mut with_kind = Vec::with_capacity(4 + payload.len());
    with_kind.extend_from_slice(kind);
    with_kind.extend_from_slice(payload);
    out.extend_from_slice(&with_kind);
    out.extend_from_slice(&crc32(&with_kind).to_be_bytes());
}

/// Encode tightly-packed RGBA8 (`width * height * 4` bytes, top row first).
pub(crate) fn encode_rgba(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    assert_eq!(
        rgba.len(),
        (width as usize) * (height as usize) * 4,
        "pixel buffer does not match the stated dimensions",
    );

    // PNG requires a filter byte at the start of every scanline; 0 = None.
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in 0..height as usize {
        raw.push(0);
        let start = row * width as usize * 4;
        raw.extend_from_slice(&rgba[start..start + width as usize * 4]);
    }

    // zlib: 0x78 0x01 (deflate, 32K window, no preset dict) + stored blocks.
    let mut zlib = vec![0x78, 0x01];
    for (index, block) in raw.chunks(65_535).enumerate() {
        let is_last = (index + 1) * 65_535 >= raw.len();
        zlib.push(u8::from(is_last)); // BFINAL, BTYPE=00 (stored)
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::with_capacity(zlib.len() + 128);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit, RGBA, deflate, no filter, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}
