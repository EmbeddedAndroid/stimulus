use std::collections::VecDeque;

use crate::consts::PKT;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RxBuffer {
    bytes: VecDeque<u8>,
    pub last_status: [u8; 2],
}

impl RxBuffer {
    /// Appends one USB transfer, removing the FTDI modem-status prefix from
    /// every USB packet, including status-only latency packets.
    pub fn push_raw(&mut self, raw: &[u8]) {
        for packet in raw.chunks(PKT) {
            if packet.len() >= 2 {
                self.last_status.copy_from_slice(&packet[..2]);
                self.bytes.extend(&packet[2..]);
            }
        }
    }

    pub fn available(&self) -> usize {
        self.bytes.len()
    }

    pub fn first(&self) -> Option<u8> {
        self.bytes.front().copied()
    }

    pub fn take(&mut self, n: usize) -> Option<Vec<u8>> {
        if n > self.bytes.len() {
            return None;
        }
        Some(self.bytes.drain(..n).collect())
    }

    /// Discards leading non-protocol residue while retaining `byte` and all
    /// following bytes. Returns the number of discarded bytes.
    pub fn align_to(&mut self, byte: u8) -> usize {
        let count = self
            .bytes
            .iter()
            .position(|value| *value == byte)
            .unwrap_or(self.bytes.len());
        self.bytes.drain(..count);
        count
    }

    /// Align to a complete protocol header with a known packet number. If no
    /// complete match exists yet, retain the final two bytes because they may
    /// be the prefix of a header split across USB transfers.
    pub fn align_to_header(&mut self, opcode: u8, pktno: u16) -> bool {
        let [hi, lo] = pktno.to_be_bytes();
        let bytes = self.bytes.make_contiguous();
        if let Some(position) = bytes
            .windows(3)
            .position(|window| window == [opcode, hi, lo])
        {
            self.bytes.drain(..position);
            true
        } else {
            let discard = self.bytes.len().saturating_sub(2);
            self.bytes.drain(..discard);
            false
        }
    }

    pub fn clear(&mut self) {
        self.bytes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn packetize(payload: &[u8]) -> Vec<u8> {
        let mut raw = Vec::new();
        for chunk in payload.chunks(62) {
            raw.extend_from_slice(&[0x31, 0x60]);
            raw.extend_from_slice(chunk);
        }
        raw
    }

    #[test]
    fn strips_status_per_64() {
        let mut rx = RxBuffer::default();
        rx.push_raw(&[0xaa; 64]);
        assert_eq!(rx.available(), 62);
        rx.clear();
        rx.push_raw(&[0xbb; 130]);
        assert_eq!(rx.available(), 124);
        rx.clear();
        rx.push_raw(&[0x31, 0x60]);
        assert_eq!(rx.available(), 0);
    }

    #[test]
    fn status_bytes_retained() {
        let mut rx = RxBuffer::default();
        rx.push_raw(&[0x31, 0x60, 4, 5]);
        assert_eq!(rx.last_status, [0x31, 0x60]);
    }

    #[test]
    fn take_is_fifo_and_partial() {
        let mut rx = RxBuffer::default();
        rx.push_raw(&[0x31, 0, 1, 2, 3]);
        assert_eq!(rx.take(2), Some(vec![1, 2]));
        assert_eq!(rx.take(2), None);
        assert_eq!(rx.take(1), Some(vec![3]));
    }

    #[test]
    fn aligns_to_expected_protocol_opcode() {
        let mut rx = RxBuffer::default();
        rx.push_raw(&[0x31, 0x60, 0x1f, 0xee, 0xc2, 0, 1, 0x17]);
        assert_eq!(rx.align_to(0xc2), 2);
        assert_eq!(rx.take(4), Some(vec![0xc2, 0, 1, 0x17]));
    }

    #[test]
    fn aligns_split_header_past_duplicate_opcode() {
        let mut rx = RxBuffer::default();
        rx.push_raw(&[0x31, 0x60, 0xc1, 0xc1, 0x00]);
        assert!(!rx.align_to_header(0xc1, 1));
        rx.push_raw(&[0x31, 0x60, 0x01]);
        assert!(rx.align_to_header(0xc1, 1));
        assert_eq!(rx.take(3), Some(vec![0xc1, 0x00, 0x01]));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn proptest_roundtrip(payload in prop::collection::vec(any::<u8>(), 1..8192)) {
            let raw = packetize(&payload);
            let mut rx = RxBuffer::default();
            rx.push_raw(&raw);
            prop_assert_eq!(rx.take(payload.len()), Some(payload));
        }
    }
}
