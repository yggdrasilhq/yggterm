//! A painted frame, read back to the CPU.

use std::ffi::{c_uint, c_void};

use crate::ffi::{self, ImageTargetTexture2DOes};
use crate::{Error, Result};

/// RGBA8 pixels, **top-left origin** — already flipped.
///
/// ⚠ GL's origin is BOTTOM-left and every image format's is top-left, so the
/// readback flips rows once, here, rather than leaving each consumer to
/// remember. On a solid-colour test page the error is invisible, which is
/// exactly why it has to be handled at the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Frame {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Tightly packed RGBA8, `width * height * 4` bytes, top row first.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y * self.width + x) * 4) as usize;
        Some([
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ])
    }

    pub fn centre_pixel(&self) -> [u8; 4] {
        self.pixel(self.width / 2, self.height / 2)
            .unwrap_or([0, 0, 0, 0])
    }

    /// A sub-rectangle, clamped to the frame.
    ///
    /// `None` when the rectangle does not overlap the frame at all — an
    /// element scrolled out of view or of zero size, which is a real answer and
    /// not an error.
    pub fn crop(&self, x: i32, y: i32, width: u32, height: u32) -> Option<Frame> {
        if width == 0 || height == 0 {
            return None;
        }
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = ((x + width as i32).max(0) as u32).min(self.width);
        let y1 = ((y + height as i32).max(0) as u32).min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for row in y0..y1 {
            let start = ((row * self.width + x0) * 4) as usize;
            rgba.extend_from_slice(&self.rgba[start..start + (w * 4) as usize]);
        }
        Some(Frame {
            width: w,
            height: h,
            rgba,
        })
    }

    /// Encode as PNG.
    pub fn to_png(&self) -> Vec<u8> {
        crate::png::encode_rgba(&self.rgba, self.width, self.height)
    }

    /// A frame of nothing — every byte zero.
    ///
    /// The compositor exports one of these BEFORE the page paints, and it is
    /// the reason [`crate::View::last_frame`] can never hand one out: a
    /// pipeline that reported success at every step still produced 307,200
    /// identical `(0,0,0,0)` pixels in spike B, and only a colour assertion
    /// caught it.
    pub fn is_blank(&self) -> bool {
        self.rgba.iter().all(|byte| *byte == 0)
    }

    /// A stable content hash (FNV-1a), for telling two frames apart in a log.
    pub fn fingerprint(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in &self.rgba {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
        hash
    }
}

/// Import the exported `EGLImage` as a texture, attach it to an FBO and read it
/// back to host memory.
pub(crate) fn read_frame(
    image_target: ImageTargetTexture2DOes,
    egl_image: *mut c_void,
    width: u32,
    height: u32,
) -> Result<Frame> {
    unsafe {
        let mut texture: c_uint = 0;
        ffi::glGenTextures(1, &mut texture);
        ffi::glBindTexture(ffi::GL_TEXTURE_2D, texture);
        ffi::glTexParameteri(ffi::GL_TEXTURE_2D, ffi::GL_TEXTURE_MIN_FILTER, ffi::GL_NEAREST);
        ffi::glTexParameteri(ffi::GL_TEXTURE_2D, ffi::GL_TEXTURE_MAG_FILTER, ffi::GL_NEAREST);
        image_target(ffi::GL_TEXTURE_2D, egl_image);

        let mut fbo: c_uint = 0;
        ffi::glGenFramebuffers(1, &mut fbo);
        ffi::glBindFramebuffer(ffi::GL_FRAMEBUFFER, fbo);
        ffi::glFramebufferTexture2D(
            ffi::GL_FRAMEBUFFER,
            ffi::GL_COLOR_ATTACHMENT0,
            ffi::GL_TEXTURE_2D,
            texture,
            0,
        );
        let status = ffi::glCheckFramebufferStatus(ffi::GL_FRAMEBUFFER);
        if status != ffi::GL_FRAMEBUFFER_COMPLETE {
            ffi::glDeleteFramebuffers(1, &fbo);
            ffi::glDeleteTextures(1, &texture);
            return Err(Error::Readback("framebuffer incomplete"));
        }

        let mut raw = vec![0u8; (width as usize) * (height as usize) * 4];
        ffi::glReadPixels(
            0,
            0,
            width as i32,
            height as i32,
            ffi::GL_RGBA,
            ffi::GL_UNSIGNED_BYTE,
            raw.as_mut_ptr().cast(),
        );
        ffi::glFinish();
        let gl_error = ffi::glGetError();

        ffi::glBindFramebuffer(ffi::GL_FRAMEBUFFER, 0);
        ffi::glDeleteFramebuffers(1, &fbo);
        ffi::glDeleteTextures(1, &texture);

        if gl_error != ffi::GL_NO_ERROR {
            return Err(Error::Readback("glReadPixels reported a GL error"));
        }

        // Flip once, here. See the type doc.
        let stride = width as usize * 4;
        let mut rgba = vec![0u8; raw.len()];
        for row in 0..height as usize {
            let src = row * stride;
            let dst = (height as usize - 1 - row) * stride;
            rgba[dst..dst + stride].copy_from_slice(&raw[src..src + stride]);
        }
        Ok(Frame {
            width,
            height,
            rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, px: [u8; 4]) -> Frame {
        Frame {
            width,
            height,
            rgba: (0..width * height).flat_map(|_| px).collect(),
        }
    }

    #[test]
    fn a_frame_of_zeroes_is_blank_and_anything_else_is_not() {
        assert!(solid(4, 4, [0, 0, 0, 0]).is_blank());
        assert!(!solid(4, 4, [255, 0, 0, 255]).is_blank());
        // Fully transparent black is the compositor's pre-paint frame; an
        // OPAQUE black page is real content and must NOT be treated as blank.
        assert!(!solid(4, 4, [0, 0, 0, 255]).is_blank());
    }

    #[test]
    fn pixels_are_addressable_and_bounds_checked() {
        let frame = solid(3, 2, [1, 2, 3, 4]);
        assert_eq!(frame.pixel(0, 0), Some([1, 2, 3, 4]));
        assert_eq!(frame.pixel(2, 1), Some([1, 2, 3, 4]));
        assert_eq!(frame.pixel(3, 0), None, "x out of range");
        assert_eq!(frame.pixel(0, 2), None, "y out of range");
    }

    #[test]
    fn crop_clamps_to_the_frame_and_reports_a_miss_honestly() {
        let frame = solid(10, 10, [7, 8, 9, 255]);
        let inner = frame.crop(2, 2, 4, 4).expect("inside");
        assert_eq!((inner.width(), inner.height()), (4, 4));
        // A rect hanging off the edge is CLAMPED, not refused: an element half
        // off-screen still has a visible part worth capturing.
        let clipped = frame.crop(8, 8, 100, 100).expect("overlaps");
        assert_eq!((clipped.width(), clipped.height()), (2, 2));
        // No overlap at all is None — a real answer for an element scrolled out
        // of view, not an error.
        assert!(frame.crop(50, 50, 4, 4).is_none());
        assert!(frame.crop(-20, 0, 4, 4).is_none());
        assert!(frame.crop(0, 0, 0, 5).is_none(), "zero width");
    }

    #[test]
    fn a_crop_carries_the_right_pixels() {
        let mut frame = solid(4, 4, [0, 0, 0, 255]);
        // Mark (1,1) so the crop's origin is checkable.
        let i = ((1 * 4 + 1) * 4) as usize;
        frame.rgba[i] = 255;
        let crop = frame.crop(1, 1, 2, 2).expect("inside");
        assert_eq!(crop.pixel(0, 0), Some([255, 0, 0, 255]));
    }

    #[test]
    fn different_content_fingerprints_differently() {
        let red = solid(4, 4, [255, 0, 0, 255]);
        let blue = solid(4, 4, [0, 0, 255, 255]);
        assert_ne!(red.fingerprint(), blue.fingerprint());
        assert_eq!(red.fingerprint(), solid(4, 4, [255, 0, 0, 255]).fingerprint());
    }
}
