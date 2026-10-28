package android.hardware.light;
@VintfStability
enum BrightnessMode {
    USER = 0,
    SENSOR = 1,
    /**
     * Use a low-persistence mode for display backlights, where the pixel
     * color transition times are lowered.
     *
     * When set, the device driver must switch to a mode optimized for low display
     * persistence that is intended to be used when the device is being treated as a
     * head mounted display (HMD). The actual display brightness in this mode is
     * implementation dependent, and any value set for color in LightState may be
     * overridden by the HAL implementation.
     *
     * For an optimal HMD viewing experience, the display must meet the following
     * criteria in this mode:
     * - Gray-to-Gray, White-to-Black, and Black-to-White switching time must be ≤ 3 ms.
     * - The display must support low-persistence with ≤ 3.5 ms persistence.
     *   Persistence is defined as the amount of time for which a pixel is
     *   emitting light for a single frame.
     * - Any "smart panel" or other frame buffering options that increase display
     *   latency are disabled.
     * - Display brightness is set so that the display is still visible to the user
     *   under normal indoor lighting.
     * - The display must update at a rate that is at least 120 Hz.
     */
    LOW_PERSISTENCE = 2,
}
