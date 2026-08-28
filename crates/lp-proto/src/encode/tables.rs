pub const RATE_TOML: &str = include_str!("../../tables/rate.toml");
pub const THRESHOLD_TOML: &str = include_str!("../../tables/threshold.toml");
pub const TRIGGER_TOML: &str = include_str!("../../tables/trigger.toml");
pub const CLOCK_TOML: &str = include_str!("../../tables/clock.toml");
pub const STATUS_TOML: &str = include_str!("../../tables/status.toml");

pub const ALL: &[(&str, &str)] = &[
    ("rate", RATE_TOML),
    ("threshold", THRESHOLD_TOML),
    ("trigger", TRIGGER_TOML),
    ("clock", CLOCK_TOML),
    ("status", STATUS_TOML),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_row_declares_provenance() {
        for (name, table) in ALL {
            assert!(table.contains("verified ="), "{name} lacks verified field");
            assert!(table.contains("evidence ="), "{name} lacks evidence field");
        }
    }
}
