//! Fixed-width control-byte classification with scalar and SIMD backends.

pub(crate) const WIDTH: usize = 16;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ControlGroupMask {
    pub(crate) empty: u16,
    pub(crate) full: u16,
    pub(crate) tombstone: u16,
}

/// Authoritative portable classification for one complete control group.
#[allow(dead_code)]
pub(crate) fn classify_scalar(group: &[u8; WIDTH]) -> ControlGroupMask {
    let mut result = ControlGroupMask::default();
    for (lane, value) in group.iter().copied().enumerate() {
        let bit = 1_u16 << lane;
        match value {
            0 => result.empty |= bit,
            1 => result.full |= bit,
            2 => result.tombstone |= bit,
            _ => {}
        }
    }
    result
}

#[cfg(all(target_arch = "aarch64", not(miri)))]
fn classify_arch(group: &[u8; WIDTH]) -> ControlGroupMask {
    // Basic NEON is part of the AArch64 target contract.
    unsafe { classify_neon(group) }
}

#[cfg(all(target_arch = "aarch64", not(miri)))]
#[target_feature(enable = "neon")]
unsafe fn classify_neon(group: &[u8; WIDTH]) -> ControlGroupMask {
    use core::arch::aarch64::*;

    // SAFETY: `group` points to exactly 16 readable bytes. All operations are
    // lane-wise NEON operations supported by the AArch64 target.
    unsafe {
        let controls = vld1q_u8(group.as_ptr());
        let empty = vceqq_u8(controls, vdupq_n_u8(0));
        let full = vceqq_u8(controls, vdupq_n_u8(1));
        let tombstone = vceqq_u8(controls, vdupq_n_u8(2));
        ControlGroupMask {
            empty: neon_mask(empty),
            full: neon_mask(full),
            tombstone: neon_mask(tombstone),
        }
    }
}

#[cfg(all(target_arch = "aarch64", not(miri)))]
unsafe fn neon_mask(mask: core::arch::aarch64::uint8x16_t) -> u16 {
    use core::arch::aarch64::*;

    const WEIGHTS: [u8; WIDTH] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
    // Each comparison lane is 0xff or 0. Shift to 0/1, weight each lane, then
    // pairwise widen-add to produce two independent 8-bit masks.
    // SAFETY: `WEIGHTS` and all intermediate vectors contain 16 valid lanes.
    unsafe {
        let bits = vshrq_n_u8(mask, 7);
        let weights = vld1q_u8(WEIGHTS.as_ptr());
        let weighted = vmulq_u8(bits, weights);
        let pairs = vpaddlq_u8(weighted);
        let quads = vpaddlq_u16(pairs);
        let octets = vpaddlq_u32(quads);
        let low = vgetq_lane_u64(octets, 0) as u16;
        let high = vgetq_lane_u64(octets, 1) as u16;
        low | (high << 8)
    }
}

#[cfg(all(target_arch = "x86_64", not(miri)))]
fn classify_arch(group: &[u8; WIDTH]) -> ControlGroupMask {
    // SSE2 is guaranteed on x86-64.
    unsafe { classify_sse2(group) }
}

#[cfg(all(target_arch = "x86_64", not(miri)))]
#[target_feature(enable = "sse2")]
unsafe fn classify_sse2(group: &[u8; WIDTH]) -> ControlGroupMask {
    use core::arch::x86_64::*;

    // SAFETY: `group` points to exactly 16 readable bytes; unaligned loads are
    // supported by SSE2 and do not escape this function.
    unsafe {
        let controls = _mm_loadu_si128(group.as_ptr().cast::<__m128i>());
        let empty = _mm_movemask_epi8(_mm_cmpeq_epi8(controls, _mm_set1_epi8(0))) as u16;
        let full = _mm_movemask_epi8(_mm_cmpeq_epi8(controls, _mm_set1_epi8(1))) as u16;
        let tombstone = _mm_movemask_epi8(_mm_cmpeq_epi8(controls, _mm_set1_epi8(2))) as u16;
        ControlGroupMask {
            empty,
            full,
            tombstone,
        }
    }
}

#[cfg(any(miri, all(not(target_arch = "aarch64"), not(target_arch = "x86_64"))))]
fn classify_arch(group: &[u8; WIDTH]) -> ControlGroupMask {
    classify_scalar(group)
}

#[inline]
pub(crate) fn classify(group: &[u8; WIDTH]) -> ControlGroupMask {
    classify_arch(group)
}

#[cfg(test)]
mod tests {
    use super::{classify_arch, classify_scalar, ControlGroupMask, WIDTH};

    #[test]
    fn architecture_classifier_matches_scalar_for_control_patterns() {
        let patterns = [0_u8, 1, 2];
        for state in patterns {
            for lane in 0..WIDTH {
                let mut group = [state; WIDTH];
                group[lane] = patterns[(lane + 1) % patterns.len()];
                assert_eq!(classify_arch(&group), classify_scalar(&group));
            }
        }

        for first in 0_u8..=u8::MAX {
            let mut group = [0_u8; WIDTH];
            for (lane, value) in group.iter_mut().enumerate() {
                *value = first.wrapping_add((lane as u8).wrapping_mul(37));
            }
            assert_eq!(classify_arch(&group), classify_scalar(&group));
        }

        let mut seed = 0x9e37_79b9_u32;
        for _ in 0..20_000 {
            let mut group = [0_u8; WIDTH];
            for value in &mut group {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *value = (seed % 3) as u8;
            }
            assert_eq!(classify_arch(&group), classify_scalar(&group));
        }
    }

    #[test]
    fn scalar_masks_use_lane_order() {
        let mut group = [2_u8; WIDTH];
        group[1] = 0;
        group[7] = 1;
        group[15] = 0;
        assert_eq!(
            classify_scalar(&group),
            ControlGroupMask {
                empty: (1 << 1) | (1 << 15),
                full: 1 << 7,
                tombstone: u16::MAX ^ ((1 << 1) | (1 << 7) | (1 << 15)),
            }
        );
    }
}
