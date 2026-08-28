use super::{Provenance, RegWrite};
use crate::{addr::Addr, regs};

pub const CHANNELS: usize = 34;
const FILE_LEN: usize = 51;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Edge {
    #[default]
    None = 0,
    Plane1 = 1,
    Both = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    pub edge: [Edge; CHANNELS],
    pub pattern: [u8; CHANNELS],
    pub count0: u32,
    pub count1: u32,
    pub range_dir: bool,
    pub m20: u8,
    pub m22: u8,
    pub m23: u8,
    pub m24_inverted: bool,
    pub range_armed: bool,
    pub range_value: u64,
    pub range_left: u64,
    pub range_right: u64,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            edge: [Edge::None; CHANNELS],
            pattern: [0; CHANNELS],
            count0: 0,
            count1: 0,
            range_dir: false,
            m20: 0,
            m22: 0,
            m23: 0,
            m24_inverted: false,
            range_armed: false,
            range_value: 0,
            range_left: 0,
            range_right: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerSpec {
    pub a: Level,
    pub b: Level,
    pub combine: u8,
    pub prequalify: [bool; 2],
    pub position_pct: u8,
}

impl Default for TriggerSpec {
    fn default() -> Self {
        // Trigger Immediately has no active term in either level, so do not
        // seed any term/mode payload bytes. Byte +0x18 is active-low, so
        // retain its disabled wire value of zero explicitly.
        let disabled = Level {
            m24_inverted: true,
            ..Level::default()
        };
        Self {
            a: disabled.clone(),
            b: disabled,
            combine: 0,
            prequalify: [false; 2],
            position_pct: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerLayout {
    pub a_base: Addr,
    pub b_base: Addr,
    pub provenance: Provenance,
}

impl Default for TriggerLayout {
    fn default() -> Self {
        Self {
            a_base: regs::trig::A_BASE,
            // Level B is the level-A base (2^21) plus 2^22. Segment 0x40 is
            // independently occupied by the frequency counter and cannot also
            // be trigger level B.
            b_base: regs::trig::b_base(true),
            provenance: Provenance::Provisional,
        }
    }
}

fn put40(dst: &mut [u8], value: u64) {
    dst.copy_from_slice(&value.to_le_bytes()[..5]);
}

fn get40(src: &[u8]) -> u64 {
    let mut bytes = [0; 8];
    bytes[..5].copy_from_slice(src);
    u64::from_le_bytes(bytes)
}

fn encode_level(level: &Level) -> [u8; FILE_LEN] {
    let mut out = [0; FILE_LEN];
    let mut edge1 = 0u64;
    let mut edge2 = 0u64;
    let mut pat_a = 0u64;
    let mut pat_b = 0u64;
    for ch in 0..CHANNELS {
        let bit = 1u64 << ch;
        if level.edge[ch] != Edge::None {
            edge1 |= bit;
        }
        if level.edge[ch] == Edge::Both {
            edge2 |= bit;
        }
        if level.pattern[ch] & 1 != 0 {
            pat_a |= bit;
        }
        if level.pattern[ch] & 2 != 0 {
            pat_b |= bit;
        }
    }
    put40(&mut out[0..5], edge1);
    put40(&mut out[5..10], edge2);
    out[10..14].copy_from_slice(&level.count0.to_le_bytes());
    out[14..18].copy_from_slice(&level.count1.to_le_bytes());
    out[18] = u8::from(level.range_dir);
    out[20] = level.m20;
    out[22] = level.m22;
    out[23] = level.m23;
    out[24] = u8::from(!level.m24_inverted);
    out[25] = u8::from(level.range_armed);
    put40(&mut out[26..31], level.range_value);
    put40(&mut out[31..36], level.range_left);
    put40(&mut out[36..41], level.range_right);
    put40(&mut out[41..46], pat_a);
    put40(&mut out[46..51], pat_b);
    out
}

fn decode_level(bytes: &[u8; FILE_LEN]) -> Level {
    let edge1 = get40(&bytes[0..5]);
    let edge2 = get40(&bytes[5..10]);
    let pat_a = get40(&bytes[41..46]);
    let pat_b = get40(&bytes[46..51]);
    let mut level = Level::default();
    for ch in 0..CHANNELS {
        let b = 1u64 << ch;
        level.edge[ch] = if edge2 & b != 0 {
            Edge::Both
        } else if edge1 & b != 0 {
            Edge::Plane1
        } else {
            Edge::None
        };
        level.pattern[ch] = u8::from(pat_a & b != 0) | (u8::from(pat_b & b != 0) << 1);
    }
    level.count0 = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
    level.count1 = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
    level.range_dir = bytes[18] != 0;
    level.m20 = bytes[20];
    level.m22 = bytes[22];
    level.m23 = bytes[23];
    level.m24_inverted = bytes[24] == 0;
    level.range_armed = bytes[25] != 0;
    level.range_value = get40(&bytes[26..31]);
    level.range_left = get40(&bytes[31..36]);
    level.range_right = get40(&bytes[36..41]);
    level
}

pub fn encode_trigger(spec: &TriggerSpec, layout: &TriggerLayout) -> Vec<RegWrite> {
    [(&spec.a, layout.a_base), (&spec.b, layout.b_base)]
        .into_iter()
        .flat_map(|(level, base)| {
            encode_level(level)
                .into_iter()
                .enumerate()
                .map(move |(off, value)| RegWrite {
                    addr: base.offset(off as u16),
                    value,
                    provenance: layout.provenance,
                })
        })
        .collect()
}

pub fn decode_trigger(writes: &[RegWrite], layout: &TriggerLayout, combine: u8) -> TriggerSpec {
    let mut a = [0; FILE_LEN];
    let mut b = [0; FILE_LEN];
    for write in writes {
        let (base, dst) = if write.addr.0 >= layout.a_base.0
            && write.addr.0 < layout.a_base.0 + FILE_LEN as u32
        {
            (layout.a_base, &mut a)
        } else if write.addr.0 >= layout.b_base.0
            && write.addr.0 < layout.b_base.0 + FILE_LEN as u32
        {
            (layout.b_base, &mut b)
        } else {
            continue;
        };
        dst[(write.addr.0 - base.0) as usize] = write.value;
    }
    TriggerSpec {
        a: decode_level(&a),
        b: decode_level(&b),
        combine,
        ..TriggerSpec::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn roundtrip(ch in 0usize..CHANNELS, edge in 0u8..3, pattern in 0u8..4, count in any::<u32>(), value in 0u64..(1u64 << 40)) {
            let mut spec = TriggerSpec::default();
            spec.a.edge[ch] = [Edge::None, Edge::Plane1, Edge::Both][edge as usize];
            spec.a.pattern[ch] = pattern; spec.a.count0 = count; spec.a.range_value = value;
            let layout = TriggerLayout::default();
            let decoded = decode_trigger(&encode_trigger(&spec, &layout), &layout, spec.combine);
            prop_assert_eq!(decoded.a, spec.a);
            prop_assert_eq!(decoded.b, spec.b);
        }
    }
}
