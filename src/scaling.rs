// scaling functions

// Minimum hardware level that ensures panel luminescence is visible to the human eye.
// On the Infinix X6739 NT37703 panel driver, hw level 1 maps to PWM 12 (0.29%), which is
// completely pitch black. hw level 10 maps to PWM 125, which is dim but visible.
pub(crate) const MIN_VISIBLE_HW: i32 = 10;

pub(crate) fn scale_brightness_linear(
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

    (effective_min as f32 + ratio * range_hw).round() as i32
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

    // Gamma 1.8 approximation provides smoother low-end transition on AMOLED
    let gamma: f32 = 1.8; 
    let curve = ratio.powf(gamma);

    (effective_min as f32 + curve * range_hw).round() as i32
}

// Custom % Curve:
// Example
// Min -> 10
// Max -> 511
// @ 75% Input -> 255 Output
pub(crate) fn scale_brightness_custom(
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
    let normalized = (val - input_min) as f32 / range_input;

    let mid_in = 0.75;
    let mid_out = 255.0;
    if normalized <= mid_in {
        let ratio = normalized / mid_in;
        (effective_min as f32 + ratio * (mid_out - effective_min as f32)).round() as i32
    } else {
        let ratio = (normalized - mid_in) / (1.0 - mid_in);
        (mid_out + ratio * (hw_max as f32 - mid_out)).round() as i32
    }
}
