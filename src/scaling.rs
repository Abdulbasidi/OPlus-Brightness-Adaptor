// scaling functions

use crate::constants::MIN_VISIBLE_HW;

/// Automatically determine the perceptual gamma curve based on the hardware dynamic range.
///
/// For 8-bit displays (hw_max ~ 255): gamma = 1.6
/// For 9-bit displays (hw_max ~ 511): gamma = 1.8 (matches tuned AMOLED response)
/// For 10-bit displays (hw_max ~ 1023): gamma = 2.0
/// For 11-bit displays (hw_max ~ 2047): gamma = 2.2
/// For 12-bit displays (hw_max ~ 4095/4096): gamma = 2.4 (standard AOSP perceptual sRGB curve)
///
/// Formula: clamp(1.6 + (log2(hw_max) - 8.0) * 0.2, 1.6, 2.5)
pub fn compute_dynamic_gamma(hw_max: i32) -> f32 {
    let bits = (hw_max.max(1) as f32).log2();
    (1.6 + (bits - 8.0).max(0.0) * 0.2).clamp(1.6, 2.5)
}

pub(crate) fn scale_brightness_curved(
    val: i32,
    hw_min: i32,
    hw_max: i32,
    input_min: i32,
    input_max: i32,
) -> i32 {
    let effective_min = hw_min.max(MIN_VISIBLE_HW);
    if val <= input_min { return effective_min; }
    if val >= input_max { return hw_max; }

    let range_input = (input_max - input_min) as f32;
    let range_hw = (hw_max - effective_min) as f32;
    let ratio = (val - input_min) as f32 / range_input;

    let gamma = compute_dynamic_gamma(hw_max);
    let curve = ratio.powf(gamma);

    (effective_min as f32 + curve * range_hw).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_gamma_scaling() {
        let g255 = compute_dynamic_gamma(255);
        assert!((g255 - 1.6).abs() < 0.01, "g255 = {}", g255);

        let g511 = compute_dynamic_gamma(511);
        assert!((g511 - 1.8).abs() < 0.01, "g511 = {}", g511);

        let g1023 = compute_dynamic_gamma(1023);
        assert!((g1023 - 2.0).abs() < 0.01, "g1023 = {}", g1023);

        let g2047 = compute_dynamic_gamma(2047);
        assert!((g2047 - 2.2).abs() < 0.01, "g2047 = {}", g2047);

        let g4095 = compute_dynamic_gamma(4095);
        assert!((g4095 - 2.4).abs() < 0.01, "g4095 = {}", g4095);

        let g4096 = compute_dynamic_gamma(4096);
        assert!((g4096 - 2.4).abs() < 0.01, "g4096 = {}", g4096);

        let g8191 = compute_dynamic_gamma(8191);
        assert!((g8191 - 2.5).abs() < 0.01, "g8191 = {}", g8191);
    }

    #[test]
    fn test_scale_curved_511_vs_4096() {
        // At 35% on 511 panel:
        let val_35_511 = 222 + (0.35 * (8191 - 222) as f32) as i32;
        let out_511 = scale_brightness_curved(val_35_511, 1, 511, 222, 8191);
        // Ratio ~ 0.35^1.8 ~ 0.150 -> 5 + 0.150 * 506 ~ 81
        assert!(out_511 > 70 && out_511 < 90, "out_511 = {}", out_511);

        // At 35% on 4096 panel:
        let val_35_4096 = 222 + (0.35 * (8191 - 222) as f32) as i32;
        let out_4096 = scale_brightness_curved(val_35_4096, 1, 4096, 222, 8191);
        // Ratio ~ 0.35^2.4 ~ 0.0805 -> 5 + 0.0805 * 4091 ~ 334 (8.1% of max instead of 35% which would be 1433)
        assert!(out_4096 > 300 && out_4096 < 370, "out_4096 = {}", out_4096);
    }
}
