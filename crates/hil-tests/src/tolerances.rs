pub const LA_TIMEBASE_PPM: f64 = 50.0;
pub const STIM_XTAL_PPM: f64 = 50.0;
pub const FREQ_COUNTER_LSB_HZ: f64 = 10.0;
pub const SKEW_MAX_NS: f64 = 1.0;
pub const STATE_WINDOW_MAX_NS: f64 = 5.0;
pub const D1_EXTRA_DELAY_NS: (f64, f64) = (1.7, 2.3);
pub const SOAK_RSS_GROWTH_MAX: f64 = 0.05;
pub const NO_HW_SUITE_MAX_S: u64 = 600;
pub const SMOKE_MAX_S: u64 = 15;
pub const TIMING_RATES_HZ: [u64; 18] = [
    1_000,
    2_000,
    5_000,
    10_000,
    20_000,
    50_000,
    100_000,
    200_000,
    500_000,
    1_000_000,
    2_000_000,
    5_000_000,
    10_000_000,
    20_000_000,
    50_000_000,
    100_000_000,
    200_000_000,
    500_000_000,
];

pub const fn edge_tol_samples(rate_hz: u64) -> u32 {
    if rate_hz > 200_000_000 { 1 } else { 0 }
}

pub const fn trigger_tol_samples(rate_hz: u64) -> u32 {
    edge_tol_samples(rate_hz)
}

pub fn threshold_tol_v(volts: f64) -> f64 {
    0.100 + 0.05 * volts.abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_table_and_thresholds_are_pinned() {
        assert_eq!(TIMING_RATES_HZ.len(), 18);
        assert_eq!(TIMING_RATES_HZ[0], 1_000);
        assert_eq!(TIMING_RATES_HZ[17], 500_000_000);
        assert_eq!(edge_tol_samples(200_000_000), 0);
        assert_eq!(edge_tol_samples(500_000_000), 1);
        assert!((threshold_tol_v(-2.0) - 0.2).abs() < f64::EPSILON);
    }
}
