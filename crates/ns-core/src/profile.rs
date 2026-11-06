//! Fast vertical luminance profile extraction from camera Y-planes.
//!
//! A rolling shutter reads rows sequentially, so the vertical row profile
//! maps to a time axis. This module extracts the row-mean luminance (0.0..1.0)
//! with high efficiency using ARM NEON SIMD on aarch64 and an auto-vectorized
//! fallback on other platforms.

/// Extract the vertical row profile from a raw Y-plane buffer into an existing slice.
///
/// `width` and `height` are the image dimensions.
/// `row_stride` is the number of bytes between the start of adjacent rows in memory.
/// `pixel_stride` is the step between columns (1 for contiguous Y planes).
/// `row_step` subsamples rows (1 = every row, 2 = every other row).
/// `col_step` subsamples columns (1 = every column).
pub fn extract_row_profile_into(
    y_plane: &[u8],
    width: usize,
    height: usize,
    row_stride: usize,
    pixel_stride: usize,
    row_step: usize,
    col_step: usize,
    out: &mut [f64],
) {
    let r_step = row_step.max(1);
    let c_step = col_step.max(1);
    let out_len = height / r_step;
    assert!(out.len() >= out_len, "output slice too small");

    if width == 0 || height == 0 || y_plane.is_empty() {
        for v in out.iter_mut().take(out_len) {
            *v = 0.0;
        }
        return;
    }

    // Fast path: contiguous columns with col_step == 1
    if pixel_stride == 1 && c_step == 1 {
        let norm = 1.0 / (width as f64 * 255.0);
        for (i, row_idx) in (0..height).step_by(r_step).enumerate() {
            let offset = row_idx * row_stride;
            if offset + width <= y_plane.len() {
                let row_slice = &y_plane[offset..offset + width];
                let sum = sum_contiguous_row(row_slice);
                out[i] = sum as f64 * norm;
            } else {
                out[i] = 0.0;
            }
        }
        return;
    }

    // Strided path: pixel_stride > 1 or col_step > 1
    for (i, row_idx) in (0..height).step_by(r_step).enumerate() {
        let row_offset = row_idx * row_stride;
        let mut sum = 0u64;
        let mut count = 0usize;
        let mut col = 0usize;

        while col < width {
            let offset = row_offset + col * pixel_stride;
            if offset < y_plane.len() {
                sum += y_plane[offset] as u64;
                count += 1;
            }
            col += c_step;
        }

        out[i] = if count > 0 {
            sum as f64 / (count as f64 * 255.0)
        } else {
            0.0
        };
    }
}

/// Allocating version of [`extract_row_profile_into`].
pub fn extract_row_profile(
    y_plane: &[u8],
    width: usize,
    height: usize,
    row_stride: usize,
    pixel_stride: usize,
    row_step: usize,
    col_step: usize,
) -> Vec<f64> {
    let r_step = row_step.max(1);
    let mut out = vec![0.0; height / r_step];
    extract_row_profile_into(
        y_plane,
        width,
        height,
        row_stride,
        pixel_stride,
        row_step,
        col_step,
        &mut out,
    );
    out
}

/// Sum contiguous row bytes. Uses NEON SIMD on aarch64.
#[inline]
fn sum_contiguous_row(slice: &[u8]) -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        unsafe { sum_contiguous_row_neon(slice) }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        sum_contiguous_row_scalar(slice)
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn sum_contiguous_row_neon(slice: &[u8]) -> u64 {
    use core::arch::aarch64::*;

    let len = slice.len();
    let ptr = slice.as_ptr();
    let chunks = len / 16;
    let remainder = len % 16;

    let mut acc = vdupq_n_u32(0);

    let mut i = 0;
    while i < chunks {
        // Unroll by 4 (64 bytes per iteration) when possible
        if i + 4 <= chunks {
            let v0 = vld1q_u8(ptr.add(i * 16));
            let v1 = vld1q_u8(ptr.add((i + 1) * 16));
            let v2 = vld1q_u8(ptr.add((i + 2) * 16));
            let v3 = vld1q_u8(ptr.add((i + 3) * 16));

            let u16_0 = vpaddlq_u8(v0);
            let u16_1 = vpaddlq_u8(v1);
            let u16_2 = vpaddlq_u8(v2);
            let u16_3 = vpaddlq_u8(v3);

            let s01 = vaddq_u16(u16_0, u16_1);
            let s23 = vaddq_u16(u16_2, u16_3);
            let s_all = vaddq_u16(s01, s23);

            acc = vpadalq_u16(acc, s_all);
            i += 4;
        } else {
            let v = vld1q_u8(ptr.add(i * 16));
            let u16_v = vpaddlq_u8(v);
            acc = vpadalq_u16(acc, u16_v);
            i += 1;
        }
    }

    let mut sum = vaddvq_u32(acc) as u64;

    let rem_start = chunks * 16;
    for k in 0..remainder {
        sum += *ptr.add(rem_start + k) as u64;
    }

    sum
}

#[inline]
#[allow(dead_code)]
fn sum_contiguous_row_scalar(slice: &[u8]) -> u64 {
    // 16-byte chunking encourages compiler auto-vectorization (AVX2/SSE on x86)
    let chunks = slice.chunks_exact(16);
    let remainder = chunks.remainder();

    let mut sum: u64 = 0;
    for chunk in chunks {
        let mut s: u32 = 0;
        for &b in chunk {
            s += b as u32;
        }
        sum += s as u64;
    }
    for &b in remainder {
        sum += b as u64;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_plane() {
        let width = 64;
        let height = 32;
        let row_stride = 64;
        let y = vec![128u8; width * height];
        let p = extract_row_profile(&y, width, height, row_stride, 1, 1, 1);
        assert_eq!(p.len(), height);
        for &val in &p {
            assert!((val - 128.0 / 255.0).abs() < 1e-6);
        }
    }

    #[test]
    fn test_padded_row_stride() {
        let width = 48;
        let height = 16;
        let row_stride = 64; // 16 bytes of padding per row
        let mut y = vec![0u8; row_stride * height];
        for r in 0..height {
            for c in 0..width {
                y[r * row_stride + c] = (r * 10) as u8;
            }
        }
        let p = extract_row_profile(&y, width, height, row_stride, 1, 1, 1);
        assert_eq!(p.len(), height);
        for (r, &val) in p.iter().enumerate() {
            let expected = (r * 10) as f64 / 255.0;
            assert!((val - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn test_scalar_and_simd_parity() {
        let mut data = Vec::with_capacity(1920);
        for i in 0..1920 {
            data.push(((i * 7 + 13) % 256) as u8);
        }
        let scalar = sum_contiguous_row_scalar(&data);
        let actual = sum_contiguous_row(&data);
        assert_eq!(scalar, actual);
    }

    #[test]
    fn test_strided_extraction() {
        let width = 64;
        let height = 16;
        let row_stride = 128;
        let pixel_stride = 2; // e.g. YUYV or interleaved
        let mut y = vec![0u8; row_stride * height];
        for r in 0..height {
            for c in 0..width {
                y[r * row_stride + c * pixel_stride] = 200;
            }
        }
        let p = extract_row_profile(&y, width, height, row_stride, pixel_stride, 1, 1);
        assert_eq!(p.len(), height);
        for &val in &p {
            assert!((val - 200.0 / 255.0).abs() < 1e-6);
        }
    }
}
