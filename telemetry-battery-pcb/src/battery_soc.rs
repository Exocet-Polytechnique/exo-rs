//! Battery state-of-charge (SoC) estimation from voltage.
//!
//! The curve below was read off the battery datasheet's "State of Charge Curve (0.5C, 25°C)"
//! graph by matching the voltage and SoC traces against their common charging-time axis. Two
//! approximations are baked in, both accepted as "good enough for now" for this application:
//!
//! - The source graph is a **charge** curve, not a discharge/OCV curve. Under real discharge
//!   load the pack voltage sags below what this curve assumes for a given SoC (opposite of the
//!   charge-side bump from internal resistance), so readings will tend to look more discharged
//!   than they really are — i.e. errs toward early/conservative warnings rather than missed ones.
//! - The graph only covers 40%-100% SoC (charging started at 40%). The 0%-40% range, which is
//!   exactly where the low-charge warning/error thresholds live, is linearly extrapolated from
//!   the slope of the first charging segment (40% -> 54%). The resulting ~18V zero-charge point
//!   lines up with a plausible 7S Li-ion discharge cutoff, but it is not a datasheet value.
//!
//! Replace this table with a real discharge or OCV curve if better accuracy is needed, especially
//! near the warning/error thresholds.

/// `(voltage in volts, state of charge in percent)`, sorted by ascending voltage.
const SOC_CURVE: &[(f32, f32)] = &[
    (17.96, 0.0),
    (25.10, 40.0),
    (27.60, 54.0),
    (27.80, 68.0),
    (28.00, 82.0),
    (29.00, 93.0),
    (29.30, 100.0),
];

/// Estimate the battery's state of charge (0.0-100.0) from its terminal voltage, by linear
/// interpolation over [`SOC_CURVE`]. Voltages outside the curve's range are clamped.
pub fn estimate_soc_percent(voltage_v: f32) -> f32 {
    let (min_v, min_soc) = SOC_CURVE[0];
    let (max_v, max_soc) = SOC_CURVE[SOC_CURVE.len() - 1];

    if voltage_v <= min_v {
        return min_soc;
    }
    if voltage_v >= max_v {
        return max_soc;
    }

    for pair in SOC_CURVE.windows(2) {
        let (v_lo, soc_lo) = pair[0];
        let (v_hi, soc_hi) = pair[1];

        if voltage_v <= v_hi {
            let t = (voltage_v - v_lo) / (v_hi - v_lo);
            return soc_lo + t * (soc_hi - soc_lo);
        }
    }

    max_soc
}
