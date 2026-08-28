pub fn encode_threshold(vth: f64, cal_offset: i32) -> u16 {
    let vadj = (vth - 1.315) * -0.402_038_109_862_497_4 + 1.315;
    let raw = (((4.559 - vadj) / 5.875) * 1023.0).round() as i32 + cal_offset;
    raw.clamp(0, 1023) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formula_vectors_and_clamp() {
        assert_eq!(encode_threshold(-100.0, 0), 0);
        assert_eq!(encode_threshold(100.0, 0), 1023);
        assert_eq!(encode_threshold(1.315, 0), 565);
        assert_eq!(encode_threshold(1.315, 10), 575);
    }
}
