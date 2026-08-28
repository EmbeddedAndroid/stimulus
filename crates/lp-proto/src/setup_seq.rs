use crate::{
    addr::Addr,
    encode::{
        Provenance,
        trigger::{Edge, TriggerLayout, TriggerSpec, encode_trigger},
    },
    regs,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegOp {
    pub addr: Addr,
    pub data: Vec<u8>,
    pub provenance: Provenance,
}

impl RegOp {
    fn byte(addr: Addr, value: u8, provenance: Provenance) -> Self {
        Self {
            addr,
            data: vec![value],
            provenance,
        }
    }
    fn word(addr: Addr, value: u16, provenance: Provenance) -> Self {
        Self {
            addr,
            data: value.to_le_bytes().to_vec(),
            provenance,
        }
    }

    fn bytes(addr: Addr, data: impl Into<Vec<u8>>, provenance: Provenance) -> Self {
        Self {
            addr,
            data: data.into(),
            provenance,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub rate: [u8; 2],
    pub mode: u8,
    pub enable_mask: u64,
    pub channel_mask_active: bool,
    pub mask2: u64,
    pub mode_flag: bool,
    pub trigger: TriggerSpec,
    pub trigger_layout: TriggerLayout,
    pub threshold_code: u16,
    pub pre_count: u16,
    pub post_count: u16,
    pub arm: bool,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Dirty {
    pub rate: bool,
    pub mode: bool,
    pub trigger: bool,
    pub threshold: bool,
    pub position: bool,
}

fn u40(value: u64) -> Vec<u8> {
    value.to_le_bytes()[..5].to_vec()
}

pub fn setup_sequence(setup: &Setup, dirty: Dirty) -> Vec<RegOp> {
    let p = setup.provenance;
    let mut ops = Vec::new();
    if dirty.rate {
        // The divider must be written as two one-byte register transactions.
        // A combined two-byte C1 is accepted and acknowledged by USB/FPGA
        // framing but leaves the command clock domain silent.
        ops.push(RegOp::byte(regs::rate::R0, setup.rate[0], p));
        ops.push(RegOp::byte(regs::rate::R1, setup.rate[1], p));
    }
    if dirty.mode {
        ops.push(RegOp::byte(regs::cap::MODE, setup.mode, p));
        ops.push(RegOp::byte(regs::cap::MASK_GATE, 0, p));
        // The channel masks must be written between MASK_GATE=0 and
        // MASK_GATE=1; the selective-mask branch supplies CH_MASK; the
        // non-selective branch keeps the cached all-enabled value and skips
        // only that field.
        if setup.channel_mask_active {
            ops.push(RegOp {
                addr: regs::cap::CH_MASK,
                data: u40(setup.enable_mask),
                provenance: p,
            });
        }
        ops.push(RegOp {
            addr: regs::cap::MASK2,
            data: u40(setup.mask2),
            provenance: p,
        });
        ops.push(RegOp::byte(
            regs::cap::MASK_GATE,
            u8::from(setup.channel_mask_active),
            p,
        ));
        ops.push(RegOp::byte(regs::cap::FLAG1, u8::from(setup.mode_flag), p));
    }
    if dirty.trigger {
        ops.push(RegOp::byte(regs::ctrl::ARM, 0, p));
        ops.push(RegOp::byte(regs::ctrl::RESET, 1, p));
        let encoded = encode_trigger(&setup.trigger, &setup.trigger_layout);
        let a = &encoded[..51];
        let b = &encoded[51..];
        // The trigger fields are written individually addressed in this order:
        // all five-byte planes/ranges, dwords +10/+14, then the one-byte mode
        // fields ending at +24/+25. Preserve that order because +25 is the
        // level commit immediately before the next bank select.
        for (base, bank, level) in [
            (setup.trigger_layout.a_base, a, &setup.trigger.a),
            (setup.trigger_layout.b_base, b, &setup.trigger.b),
        ] {
            let range = level.range_armed
                || level.range_dir
                || level.range_value != 0
                || level.range_left != 0
                || level.range_right != 0;
            let edge = level.edge.iter().any(|value| *value != Edge::None);
            let pattern = level.pattern.iter().any(|value| *value != 0);
            let logic = edge || pattern;
            let mut fields = Vec::new();
            if logic {
                fields.extend_from_slice(&[(0, 5), (5, 5)]);
            }
            if range {
                fields.extend_from_slice(&[(26, 5), (31, 5), (36, 5)]);
            }
            if logic {
                fields.extend_from_slice(&[(41, 5), (46, 5)]);
            }
            fields.extend_from_slice(&[(10, 4), (14, 4)]);
            if range {
                fields.push((18, 1));
            }
            if logic || range {
                fields.extend_from_slice(&[(20, 1), (22, 1), (23, 1)]);
            }
            fields.extend_from_slice(&[(24, 1), (25, 1)]);
            for (offset, len) in fields {
                ops.push(RegOp::bytes(
                    Addr(base.0 + offset as u32),
                    bank[offset..offset + len]
                        .iter()
                        .map(|write| write.value)
                        .collect::<Vec<_>>(),
                    setup.trigger_layout.provenance,
                ));
            }
        }
        ops.push(RegOp::byte(regs::ctrl::COMBINE, setup.trigger.combine, p));
    }
    if dirty.threshold {
        ops.push(RegOp::word(regs::ctrl::DAC, setup.threshold_code, p));
    }
    if dirty.position {
        ops.push(RegOp::word(regs::ctrl::PRE_COUNT, setup.pre_count, p));
        ops.push(RegOp::word(regs::ctrl::POST_COUNT, setup.post_count, p));
    }
    // The re-arm (ARM) dirty flag is set only when the trigger group is
    // reconfigured; scalar threshold/position passes do not write ARM.
    if dirty.trigger {
        ops.push(RegOp::byte(regs::ctrl::ARM, u8::from(setup.arm), p));
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Setup {
        Setup {
            rate: [0x21, 0],
            mode: 0x14,
            enable_mask: (1 << 34) - 1,
            channel_mask_active: true,
            mask2: 0,
            mode_flag: false,
            trigger: TriggerSpec::default(),
            trigger_layout: TriggerLayout::default(),
            threshold_code: 565,
            pre_count: 1032,
            post_count: 1016,
            arm: true,
            provenance: Provenance::Provisional,
        }
    }

    #[test]
    fn dirty_sections_follow_vendor_order_and_rearm_last() {
        let mut configured = setup();
        configured.trigger.combine = 1;
        configured.enable_mask = 0x1234;
        let ops = setup_sequence(
            &configured,
            Dirty {
                rate: true,
                mode: true,
                trigger: true,
                threshold: true,
                position: true,
            },
        );
        assert_eq!(
            &ops[..8].iter().map(|o| o.addr).collect::<Vec<_>>(),
            &[
                regs::rate::R0,
                regs::rate::R1,
                regs::cap::MODE,
                regs::cap::MASK_GATE,
                regs::cap::CH_MASK,
                regs::cap::MASK2,
                regs::cap::MASK_GATE,
                regs::cap::FLAG1
            ]
        );
        assert_eq!(ops[0].data, [0x21]);
        assert_eq!(ops[1].data, [0]);
        assert_eq!(ops[3].addr, regs::cap::MASK_GATE);
        assert_eq!(ops[3].data, [0]);
        assert_eq!(ops[4].addr, regs::cap::CH_MASK);
        assert_eq!(ops[5].addr, regs::cap::MASK2);
        assert_eq!(ops[6].addr, regs::cap::MASK_GATE);
        assert_eq!(ops[6].data, [1]);
        assert_eq!(ops[8].addr, regs::ctrl::ARM);
        assert_eq!(ops[9].addr, regs::ctrl::RESET);
        assert_eq!(ops[10].addr, Addr(setup().trigger_layout.a_base.0 + 10));
        assert!(
            ops.iter().any(|op| {
                op.addr == Addr(setup().trigger_layout.b_base.0 + 24) && op.data == [0]
            })
        );
        assert_eq!(ops[ops.len() - 5].addr, regs::ctrl::COMBINE);
        assert_eq!(ops[ops.len() - 4].addr, regs::ctrl::DAC);
        assert_eq!(ops[ops.len() - 3].addr, regs::ctrl::PRE_COUNT);
        assert_eq!(ops[ops.len() - 2].addr, regs::ctrl::POST_COUNT);
        assert_eq!(ops[ops.len() - 1].addr, regs::ctrl::ARM);
    }

    #[test]
    fn vendor_idle_trigger_writes_both_disabled_common_tails() {
        let ops = setup_sequence(
            &setup(),
            Dirty {
                trigger: true,
                ..Dirty::default()
            },
        );
        assert_eq!(ops[0].addr, regs::ctrl::ARM);
        assert_eq!(ops[1].addr, regs::ctrl::RESET);
        assert_eq!(ops[2].addr, Addr(setup().trigger_layout.a_base.0 + 10));
        assert_eq!(ops[4].addr, Addr(setup().trigger_layout.a_base.0 + 24));
        assert_eq!(ops[4].data, [0]);
        assert_eq!(ops[6].addr, Addr(setup().trigger_layout.b_base.0 + 10));
        assert_eq!(ops[8].addr, Addr(setup().trigger_layout.b_base.0 + 24));
        assert_eq!(ops[9].addr, Addr(setup().trigger_layout.b_base.0 + 25));
        assert_eq!(ops[10].addr, regs::ctrl::COMBINE);
        assert_eq!(ops[11].addr, regs::ctrl::ARM);
        assert_eq!(ops.len(), 12);
    }

    #[test]
    fn active_logic_term_programs_both_plane_pairs_before_common_tail() {
        let mut configured = setup();
        configured.trigger.a.edge[0] = Edge::Plane1;
        let ops = setup_sequence(
            &configured,
            Dirty {
                trigger: true,
                ..Dirty::default()
            },
        );
        assert_eq!(ops[2].addr, configured.trigger_layout.a_base);
        assert_eq!(ops[2].data, [1, 0, 0, 0, 0]);
        assert_eq!(ops[3].addr, Addr(configured.trigger_layout.a_base.0 + 5));
        assert_eq!(ops[4].addr, Addr(configured.trigger_layout.a_base.0 + 41));
        assert_eq!(ops[5].addr, Addr(configured.trigger_layout.a_base.0 + 46));
        assert_eq!(ops[6].addr, Addr(configured.trigger_layout.a_base.0 + 10));
        assert_eq!(ops[7].addr, Addr(configured.trigger_layout.a_base.0 + 14));
        assert_eq!(ops[8].addr, Addr(configured.trigger_layout.a_base.0 + 20));
        assert_eq!(ops[11].addr, Addr(configured.trigger_layout.a_base.0 + 24));
        assert_eq!(ops[12].addr, Addr(configured.trigger_layout.a_base.0 + 25));
    }

    #[test]
    fn channel_mask_is_always_committed_before_enabling_gate() {
        let mut setup = setup();
        setup.enable_mask = 0x1234;
        let ops = setup_sequence(
            &setup,
            Dirty {
                mode: true,
                ..Dirty::default()
            },
        );
        assert_eq!(
            ops.iter().map(|op| op.addr).collect::<Vec<_>>(),
            [
                regs::cap::MODE,
                regs::cap::MASK_GATE,
                regs::cap::CH_MASK,
                regs::cap::MASK2,
                regs::cap::MASK_GATE,
                regs::cap::FLAG1,
            ]
        );
        assert_eq!(ops[2].data, [0x34, 0x12, 0, 0, 0]);
        assert_eq!(ops[4].data, [1]);
    }

    #[test]
    fn inactive_channel_mask_skips_ch_mask_and_leaves_gate_disabled() {
        let mut configured = setup();
        configured.channel_mask_active = false;
        let ops = setup_sequence(
            &configured,
            Dirty {
                mode: true,
                ..Dirty::default()
            },
        );
        assert_eq!(
            ops.iter().map(|op| op.addr).collect::<Vec<_>>(),
            [
                regs::cap::MODE,
                regs::cap::MASK_GATE,
                regs::cap::MASK2,
                regs::cap::MASK_GATE,
                regs::cap::FLAG1,
            ]
        );
        assert_eq!(ops[1].data, [0]);
        assert_eq!(ops[2].addr, regs::cap::MASK2);
        assert_eq!(ops[3].data, [0]);
        assert_eq!(ops[4].addr, regs::cap::FLAG1);
    }

    #[test]
    fn clean_setup_emits_no_register_writes() {
        let ops = setup_sequence(&setup(), Dirty::default());
        assert!(ops.is_empty());
    }
}
