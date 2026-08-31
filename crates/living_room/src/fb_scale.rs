//! Nearest-neighbour scale of Spectrum / Timex RGBA into the CRT texture size.

use crate::crt::{SCREEN_H, SCREEN_W};

/// Known host framebuffer sizes (RGBA8 byte lengths → width × height).
#[must_use]
pub fn dims_from_rgba_len(len: usize) -> Option<(u32, u32)> {
    const LORES_BORDER: usize = 352 * 296 * 4;
    const LORES_PAPER: usize = 256 * 192 * 4;
    const HIRES_BORDER: usize = 640 * 296 * 4;
    const HIRES_PAPER: usize = 512 * 192 * 4;
    match len {
        LORES_BORDER => Some((352, 296)),
        LORES_PAPER => Some((256, 192)),
        HIRES_BORDER => Some((640, 296)),
        HIRES_PAPER => Some((512, 192)),
        _ => None,
    }
}

/// Copy or nearest-neighbour scale `src` (row-major RGBA8) into `dst` at
/// [`SCREEN_W`]×[`SCREEN_H`].
pub fn blit_to_crt(dst: &mut [u8], src: &[u8], src_w: u32, src_h: u32) {
    let expect = (SCREEN_W * SCREEN_H * 4) as usize;
    assert!(dst.len() >= expect);
    if src_w == SCREEN_W && src_h == SCREEN_H && src.len() >= expect {
        dst[..expect].copy_from_slice(&src[..expect]);
        return;
    }
    if src_w == 0 || src_h == 0 {
        dst[..expect].fill(0);
        return;
    }
    for y in 0..SCREEN_H {
        let sy = (u64::from(y) * u64::from(src_h) / u64::from(SCREEN_H)) as u32;
        for x in 0..SCREEN_W {
            let sx = (u64::from(x) * u64::from(src_w) / u64::from(SCREEN_W)) as u32;
            let si = ((sy * src_w + sx) * 4) as usize;
            let di = ((y * SCREEN_W + x) * 4) as usize;
            if si + 4 <= src.len() {
                dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dims_recognize_timex_hires() {
        assert_eq!(dims_from_rgba_len(640 * 296 * 4), Some((640, 296)));
        assert_eq!(dims_from_rgba_len(512 * 192 * 4), Some((512, 192)));
        assert_eq!(dims_from_rgba_len(352 * 296 * 4), Some((352, 296)));
    }

    #[test]
    fn scale_hires_paper_to_crt() {
        let mut src = vec![0u8; 512 * 192 * 4];
        // Mark top-left pixel white.
        src[0] = 255;
        src[1] = 255;
        src[2] = 255;
        src[3] = 255;
        let mut dst = vec![0u8; (SCREEN_W * SCREEN_H * 4) as usize];
        blit_to_crt(&mut dst, &src, 512, 192);
        assert_eq!(&dst[0..3], &[255, 255, 255]);
    }
}
