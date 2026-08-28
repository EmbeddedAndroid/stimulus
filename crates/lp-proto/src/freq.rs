pub const fn counter_to_hz(raw: u32) -> u64 {
    raw as u64 * 10
}

#[cfg(test)]
mod tests {
    #[test]
    fn hundred_ms_gate_scales_by_ten() {
        assert_eq!(super::counter_to_hz(12_345), 123_450);
    }
}
