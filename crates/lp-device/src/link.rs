use crate::{
    clock::{Clock, WallClock},
    transport::{Transport, TransportError},
};
use lp_ftdi::consts::{SIO_RESET, SIO_RESET_PURGE_RX};
use lp_ftdi::rx::RxBuffer;
use lp_proto::{
    ProtoError,
    addr::Addr,
    bank::BankTracker,
    packet::{self, Cmd, PktNo},
};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkConfig {
    pub read_timeout: Duration,
    pub write_ack_timeout: Duration,
    pub bank_timeout: Duration,
    pub bulk_out_timeout: Duration,
    pub reopen_attempts: u8,
    pub reopen_gap: Duration,
    pub notify_after_errors: u64,
}
impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            read_timeout: Duration::from_secs(2),
            write_ack_timeout: Duration::from_secs(1),
            bank_timeout: Duration::from_secs(1),
            bulk_out_timeout: Duration::from_secs(5),
            reopen_attempts: 20,
            reopen_gap: Duration::from_millis(500),
            notify_after_errors: 10,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DevStats {
    pub usb_error_count: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

pub struct Link<T: Transport, C: Clock = WallClock> {
    pub(crate) transport: T,
    pub(crate) clock: C,
    pub(crate) config: LinkConfig,
    pub(crate) bank: BankTracker,
    pub(crate) pktno: PktNo,
    pub(crate) rx: RxBuffer,
    stats: DevStats,
    pub(crate) configured_idx: Option<u8>,
    /// Zero bytes written before the `0x01` FIFO resync byte. The vendor
    /// binary writes 0xffff + 30000.
    pub(crate) fifo_tail_zeros: usize,
}

impl<T: Transport, C: Clock> Link<T, C> {
    pub fn new(transport: T, clock: C, config: LinkConfig) -> Self {
        Self {
            transport,
            clock,
            config,
            bank: BankTracker::default(),
            pktno: PktNo::default(),
            rx: RxBuffer::default(),
            stats: DevStats::default(),
            configured_idx: None,
            fifo_tail_zeros: crate::fpga::FIFO_TAIL_ZEROS,
        }
    }
    /// Override the post-configure FIFO tail length (diagnostics/A-B runs).
    pub fn set_fifo_tail_zeros(&mut self, zeros: usize) {
        self.fifo_tail_zeros = zeros;
    }
    pub fn stats(&self) -> DevStats {
        self.stats
    }
    pub fn configured_image(&self) -> Option<u8> {
        self.configured_idx
    }
    pub fn into_parts(self) -> (T, C) {
        (self.transport, self.clock)
    }

    pub fn read(&mut self, addr: Addr, len: u16) -> Result<Vec<u8>, DevError> {
        let len = if len == 0 { 65_536 } else { u32::from(len) };
        let cmd = Cmd::Read {
            addr: addr.off(),
            len,
        };
        self.select_bank(addr)?;
        match self.exec(cmd, &[]) {
            Err(DevError::Protocol(ProtoError::PacketNumber { expected, got })) => {
                // Self-heal a FIFO packet-counter drift (a dropped or duplicated
                // FT245 response accumulated under heavy use): purge stale
                // device->host bytes, drop our RX buffer, resync the shared
                // counter to whatever the device sends next, force a fresh C3,
                // and retry the read ONCE. The FPGA configuration is untouched -
                // no power-cycle. A second failure is a genuine fault and
                // propagates (see the negative-gate tests).
                eprintln!(
                    "read pktno drift at {addr:?}: expected {expected} got {got}; resync + retry"
                );
                self.resync_fifo();
                self.select_bank(addr)?;
                self.exec(cmd, &[])
            }
            other => other,
        }
    }

    /// Flush any stale readback backlog before an acquisition: discard the
    /// transport read-ahead pool's queued-but-unconsumed IN transfers and clear
    /// the framing buffer. This is prevention, not recovery -- it stops the
    /// previous capture's over-read from desyncing the next capture, which is
    /// the root cause of the sustained-load degradation (fast 502s after tens
    /// of captures). Returns the number of queued transfers discarded.
    pub fn purge_input(&mut self) -> usize {
        let discarded = self.transport.drain_input();
        self.rx.clear();
        discarded
    }

    /// Recover from a FIFO packet-counter drift without a power-cycle: purge any
    /// stale device->host bytes from the FT245, drop our RX buffer, resync the
    /// shared packet counter to whatever the device sends next, and force a
    /// fresh C3 bank-select. The FPGA configuration is left intact. Each drift
    /// is a device->host framing anomaly, so it is counted as a USB error: a
    /// clean soak (with the capture-boundary purge in place) should never resync.
    fn resync_fifo(&mut self) {
        self.stats.usb_error_count = self.stats.usb_error_count.saturating_add(1);
        let _ = self.transport.control_out(SIO_RESET, SIO_RESET_PURGE_RX, 0);
        self.rx.clear();
        self.pktno.resync();
        self.bank.invalidate();
    }
    pub fn write(&mut self, addr: Addr, data: &[u8]) -> Result<(), DevError> {
        self.write_checked(addr, data, None)
    }
    /// Execute the single vendor C1 write primitive and, when the caller knows
    /// the following register, establish its bank before returning.
    pub fn write_checked(
        &mut self,
        addr: Addr,
        data: &[u8],
        next: Option<Addr>,
    ) -> Result<(), DevError> {
        if data.is_empty() {
            return Err(DevError::Protocol(ProtoError::InvalidLength(0)));
        }
        self.select_bank(addr)?;
        let cmd = Cmd::Write {
            addr: addr.off(),
            len: data.len() as u32,
        };
        let next_bank = next.filter(|next| next.bank() != addr.bank());
        self.exec(cmd, data)?;
        if let Some(next) = next_bank {
            self.select_bank(next)?;
        }
        Ok(())
    }

    /// Execute setup through the vendor's checked write primitive. Each C1
    /// header and payload are separate USB writes, and its complete ACK is
    /// consumed before any following C1 or C3 is sent.
    pub fn write_checked_sequence(&mut self, writes: &[(Addr, Vec<u8>)]) -> Result<(), DevError> {
        for (index, (addr, data)) in writes.iter().enumerate() {
            let next = writes.get(index + 1).map(|(addr, _)| *addr);
            self.write_checked(*addr, data, next)?;
        }
        Ok(())
    }

    pub(crate) fn write_release(&mut self, addr: Addr) -> Result<(), DevError> {
        self.select_bank(addr)?;
        let result = self.write_transition(addr, &[0], 0);
        self.invalidate();
        result.map(|_| ())
    }
    fn write_transition(
        &mut self,
        addr: Addr,
        data: &[u8],
        minimum_ack_prefix: usize,
    ) -> Result<bool, DevError> {
        let cmd = Cmd::Write {
            addr: addr.off(),
            len: data.len() as u32,
        };
        self.exec_transition(cmd, data, minimum_ack_prefix)
    }
    fn exec_transition(
        &mut self,
        cmd: Cmd,
        data: &[u8],
        minimum_ack_prefix: usize,
    ) -> Result<bool, DevError> {
        // Read the FT245 for the full one-second response deadline so a
        // clock-domain transition cannot leave its response path occupied
        // while the host merely sleeps.
        let timeout = Duration::from_secs(1);
        let opcode = self.send_transition_command(cmd, data)?;
        match self.read_response(3, timeout, opcode) {
            Ok(response) => {
                let header = packet::parse_header([response[0], response[1], response[2]]);
                packet::check_header(cmd, header)?;
                self.check_packet_number(cmd, header.pktno)?;
                return Ok(true);
            }
            Err(DevError::Timeout { got, .. })
                if got >= minimum_ack_prefix
                    && (minimum_ack_prefix == 0 || self.rx.first() == Some(cmd.opcode())) =>
            {
                self.invalidate_framing_preserve_bank();
            }
            Err(DevError::Protocol(ProtoError::PacketNumber { expected, got }))
                if minimum_ack_prefix == 0 =>
            {
                // A clock-changing write can acknowledge with the newly
                // selected domain's counter (observed for trigger +25 as
                // 0x0012 -> 0x0050).  The opcode was already verified by
                // read_response; discard both counters and prove the next
                // bank command instead of retrying this transition.
                eprintln!(
                    "protocol transition ack: cmd={cmd:?} expected_pktno={expected} actual_pktno={got}"
                );
                self.invalidate_framing_preserve_bank();
            }
            Err(DevError::Transport(error))
                if minimum_ack_prefix == 0 && error.0.contains("transfer was cancelled") =>
            {
                self.invalidate_framing_preserve_bank();
            }
            Err(error) => return Err(error),
        }
        Ok(false)
    }
    fn select_bank(&mut self, addr: Addr) -> Result<(), DevError> {
        if let Some(cmd) = self.bank.plan(addr) {
            match self.exec_with_timeout(cmd, &[], self.config.bank_timeout) {
                Ok(_) => Ok(()),
                Err(DevError::Timeout { got, .. }) if (1..=2).contains(&got) => {
                    // The segment mux can switch after emitting only the C3
                    // opcode (or opcode plus one byte); a partial header is
                    // not a verified response.
                    let _prefix = self.rx.take(got).unwrap_or_default();
                    self.invalidate();
                    Err(DevError::Timeout { wanted: 3, got })
                }
                Err(error) => {
                    // BankTracker::plan records the proposed segment so a
                    // successful C3 can make following accesses cheap. An
                    // unanswered or malformed C3 does not prove that segment
                    // became current. Roll back both the cache and framing so
                    // a retry must establish and acknowledge C3 again before
                    // it is allowed to send C1/C2.
                    self.invalidate();
                    Err(error)
                }
            }
        } else {
            Ok(())
        }
    }
    fn exec(&mut self, cmd: Cmd, payload: &[u8]) -> Result<Vec<u8>, DevError> {
        let timeout = match cmd {
            Cmd::Read { .. } => self.config.read_timeout,
            Cmd::Write { .. } => self.config.write_ack_timeout,
            Cmd::Bank(_) => self.config.bank_timeout,
        };
        self.exec_with_timeout(cmd, payload, timeout)
    }
    fn exec_with_timeout(
        &mut self,
        cmd: Cmd,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, DevError> {
        self.exec_once(cmd, payload, timeout)
    }

    fn exec_once(
        &mut self,
        cmd: Cmd,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, DevError> {
        self.send_command(cmd, payload)?;
        let result = self.read_checked_response(cmd, timeout);
        if let Err(DevError::Timeout { wanted, got }) = &result {
            // A readback stall: the FT245/FPGA stopped answering mid-command.
            // Log exactly which command and how much of its response arrived so
            // the wedge can be root-caused (backbone tool -- always on, rare).
            eprintln!(
                "readback wedge: cmd={cmd:?} wanted={wanted} got={got} rx_avail={} pump_queued={} timeout={timeout:?}",
                self.rx.available(),
                self.transport.debug_input_queued()
            );
        }
        result
    }

    fn send_command(&mut self, cmd: Cmd, payload: &[u8]) -> Result<u8, DevError> {
        let packet = packet::encode(cmd)?;
        eprintln!(
            "protocol tx: cmd={cmd:?} expected_pktno={:?} header={packet:02x?} payload={payload:02x?}",
            self.pktno.expected(),
        );
        if !payload.is_empty() {
            self.transport
                .bulk_out_pair(&packet, payload, self.config.bulk_out_timeout)?;
        } else if matches!(cmd, Cmd::Bank(_)) {
            self.transport
                .bulk_out_unprimed(&packet, self.config.bulk_out_timeout)?;
        } else {
            self.transport
                .bulk_out(&packet, self.config.bulk_out_timeout)?;
        }
        eprintln!("protocol tx header complete: opcode=0x{:02x}", packet[0]);
        self.stats.bytes_out = self.stats.bytes_out.saturating_add(packet.len() as u64);
        if !payload.is_empty() {
            eprintln!("protocol tx payload complete: len={}", payload.len());
            self.stats.bytes_out = self.stats.bytes_out.saturating_add(payload.len() as u64);
        }
        Ok(packet[0])
    }

    fn send_transition_command(&mut self, cmd: Cmd, payload: &[u8]) -> Result<u8, DevError> {
        let packet = packet::encode(cmd)?;
        eprintln!(
            "protocol tx: cmd={cmd:?} expected_pktno={:?} header={packet:02x?} payload={payload:02x?}",
            self.pktno.expected(),
        );
        if payload.is_empty() {
            self.transport
                .bulk_out_unprimed(&packet, self.config.bulk_out_timeout)?;
        } else {
            self.transport
                .bulk_out_pair(&packet, payload, self.config.bulk_out_timeout)?;
        }
        eprintln!("protocol tx header complete: opcode=0x{:02x}", packet[0]);
        self.stats.bytes_out = self.stats.bytes_out.saturating_add(packet.len() as u64);
        if !payload.is_empty() {
            eprintln!("protocol tx payload complete: len={}", payload.len());
            self.stats.bytes_out = self.stats.bytes_out.saturating_add(payload.len() as u64);
        }
        Ok(packet[0])
    }

    fn read_checked_response(&mut self, cmd: Cmd, timeout: Duration) -> Result<Vec<u8>, DevError> {
        let packet = packet::encode(cmd)?;
        let wanted = packet::expected_response_len(cmd)?;
        let response = self.read_response(wanted, timeout, packet[0])?;
        let header = packet::parse_header([response[0], response[1], response[2]]);
        packet::check_header(cmd, header)?;
        self.check_packet_number(cmd, header.pktno)?;
        Ok(response[3..].to_vec())
    }
    fn check_packet_number(&mut self, cmd: Cmd, actual: u16) -> Result<(), DevError> {
        eprintln!(
            "protocol ack: cmd={cmd:?} expected_pktno={:?} actual_pktno={actual}",
            self.pktno.expected()
        );
        self.pktno.check(actual)?;
        Ok(())
    }
    fn read_response(
        &mut self,
        wanted: usize,
        timeout: Duration,
        expected_opcode: u8,
    ) -> Result<Vec<u8>, DevError> {
        let deadline = self.clock.elapsed().saturating_add(timeout);
        loop {
            if self.rx.available() >= 3 {
                self.rx.align_to(expected_opcode);
                let aligned = self.rx.available() >= 3;
                if !aligned {
                    // Discard non-protocol FT245 residue and keep draining
                    // until a complete response arrives. Packet-number
                    // mismatches after the opcode remain hard failures.
                    continue;
                }
                let prefix = self.rx.take(3).ok_or(DevError::Timeout {
                    wanted,
                    got: self.rx.available(),
                })?;
                let header = packet::parse_header([prefix[0], prefix[1], prefix[2]]);
                if header.opcode != expected_opcode {
                    return Err(DevError::Protocol(ProtoError::Opcode {
                        expected: expected_opcode,
                        got: header.opcode,
                    }));
                }
                if let Some(expected) = self.pktno.expected()
                    && header.pktno != expected
                {
                    return Err(DevError::Protocol(ProtoError::PacketNumber {
                        expected,
                        got: header.pktno,
                    }));
                }
                // Put the verified header back in front of any payload so the
                // existing caller consumes one complete response atomically.
                let mut response = prefix;
                if wanted > 3 {
                    while self.rx.available() < wanted - 3 {
                        if self.clock.elapsed() >= deadline {
                            return Err(DevError::Timeout {
                                wanted,
                                got: 3 + self.rx.available(),
                            });
                        }
                        let result = self.transport.bulk_in_raw(
                            wanted.saturating_sub(3 + self.rx.available()) + 2,
                            deadline.saturating_sub(self.clock.elapsed()),
                        );
                        self.clock.io_poll();
                        let raw = match result {
                            Ok(raw) => raw,
                            Err(error)
                                if error.0.contains("transfer was cancelled")
                                    || error.0.contains("timed out waiting") =>
                            {
                                continue;
                            }
                            Err(error) => return Err(error.into()),
                        };
                        eprintln!("protocol rx raw: {raw:02x?}");
                        self.stats.bytes_in += raw.len() as u64;
                        self.rx.push_raw(&raw);
                    }
                    response.extend(self.rx.take(wanted - 3).unwrap_or_default());
                }
                return Ok(response);
            }
            if self.clock.elapsed() >= deadline {
                return Err(DevError::Timeout {
                    wanted,
                    got: self.rx.available(),
                });
            }
            let result = self.transport.bulk_in_raw(
                wanted.saturating_sub(self.rx.available()) + 2,
                deadline.saturating_sub(self.clock.elapsed()),
            );
            self.clock.io_poll();
            let raw = match result {
                Ok(raw) => raw,
                Err(error)
                    if error.0.contains("transfer was cancelled")
                        || error.0.contains("timed out waiting") =>
                {
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            eprintln!("protocol rx raw: {raw:02x?}");
            self.stats.bytes_in += raw.len() as u64;
            self.rx.push_raw(&raw);
        }
    }

    pub fn invalidate(&mut self) {
        self.bank.invalidate();
        self.invalidate_framing_preserve_bank();
    }

    /// Invalidate only the cached segment/bank so the next access re-sends a
    /// C3 bank-select, WITHOUT disturbing the shared packet counter or RX
    /// framing. The vendor never issues more than ~9 C1/C2 commands without a
    /// C3 refresh; skipping C3 for longer wedges the FPGA command FSM.
    pub fn invalidate_bank(&mut self) {
        self.bank.invalidate();
    }
    fn invalidate_framing_preserve_bank(&mut self) {
        self.pktno.resync();
        self.rx.clear();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DevError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Protocol(#[from] ProtoError),
    #[error("timed out waiting for {wanted} protocol bytes; got {got}")]
    Timeout { wanted: usize, got: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{clock::VirtualClock, transcript::DeviceIdentity};
    use std::collections::VecDeque;
    #[derive(Default)]
    struct Mock {
        writes: Vec<Vec<u8>>,
        reads: VecDeque<Vec<u8>>,
        reads_before_cancel: usize,
        cancelled_reads: usize,
        queued: usize,
        drained: usize,
    }
    impl Transport for Mock {
        fn control_out(&mut self, _: u8, _: u16, _: u16) -> Result<(), TransportError> {
            Ok(())
        }
        fn control_in(&mut self, _: u8, _: u16, _: u16, _: u16) -> Result<Vec<u8>, TransportError> {
            Ok(vec![])
        }
        fn bulk_out(&mut self, d: &[u8], _: Duration) -> Result<(), TransportError> {
            self.writes.push(d.to_vec());
            Ok(())
        }
        fn bulk_in_raw(&mut self, _: usize, _: Duration) -> Result<Vec<u8>, TransportError> {
            if self.reads_before_cancel > 0 {
                self.reads_before_cancel -= 1;
                return Ok(self.reads.pop_front().unwrap_or_default());
            }
            if self.cancelled_reads > 0 {
                self.cancelled_reads -= 1;
                return Err(TransportError(
                    "bulk I/O failed: transfer was cancelled".into(),
                ));
            }
            Ok(self.reads.pop_front().unwrap_or_else(|| vec![0x31, 0x60]))
        }
        fn drain_input(&mut self) -> usize {
            let discarded = self.queued;
            self.queued = 0;
            self.drained += 1;
            discarded
        }
        fn reopen(&mut self) -> Result<(), TransportError> {
            Ok(())
        }
        fn identity(&self) -> DeviceIdentity {
            DeviceIdentity {
                serial: "mock".into(),
                bcd_device: 0,
                vid: 0x403,
                pid: 0xdc48,
            }
        }
    }
    fn raw(payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0x31, 0x60];
        v.extend_from_slice(payload);
        v
    }
    // Prevention against the sustained-load wedge: a capture-boundary purge must
    // discard the transport read-ahead pool's queued IN transfers AND clear the
    // framing buffer, so the next readback never pops stale bytes.
    #[test]
    fn purge_input_drains_pool_and_clears_rx() {
        let mock = Mock {
            queued: 7,
            ..Mock::default()
        };
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        // Stale device->host bytes left framed from a prior over-read.
        link.rx.push_raw(&raw(&[0xaa, 0xbb, 0xcc]));
        assert!(link.rx.available() > 0);

        let discarded = link.purge_input();
        assert_eq!(discarded, 7, "must discard every queued transfer");
        assert_eq!(link.rx.available(), 0, "framing buffer must be cleared");

        // Idempotent: a second purge on a clean queue finds nothing.
        assert_eq!(link.purge_input(), 0);
    }
    #[test]
    fn auto_bank_write_and_packet_sequence() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads
            .extend([raw(&[0xc3, 0, 0]), raw(&[0xc1, 0, 1]), raw(&[0xc1, 0, 2])]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        link.write(Addr(0x0010_0003), &[1])?;
        link.write(Addr(0x0010_0004), &[8, 4])?;
        let (mock, _) = link.into_parts();
        assert_eq!(mock.writes[0], [0xc3, 0x10, 0, 0, 0]);
        assert_eq!(mock.writes[1], [0xc1, 3, 0, 0, 0]);
        assert_eq!(mock.writes[2], [1]);
        assert_eq!(mock.writes[3], [0xc1, 4, 0, 1, 0]);
        assert_eq!(mock.writes[4], [8, 4]);
        assert_eq!(mock.writes.len(), 5);
        Ok(())
    }
    #[test]
    fn checked_write_acknowledges_before_selecting_next_bank() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads
            .extend([raw(&[0xc3, 0, 0]), raw(&[0xc1, 0, 1]), raw(&[0xc3, 0, 2])]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        link.write_checked(Addr(0x0100_0001), &[0], Some(Addr(0x0010_0003)))?;
        let (mock, _) = link.into_parts();
        assert_eq!(
            mock.writes,
            [
                vec![0xc3, 0, 1, 0, 0],
                vec![0xc1, 1, 0, 0, 0],
                vec![0],
                vec![0xc3, 0x10, 0, 0, 0],
            ]
        );
        Ok(())
    }
    #[test]
    fn checked_sequence_consumes_each_ack_before_the_next_command() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads.extend([
            raw(&[0xc3, 0, 0]),
            raw(&[0xc1, 0, 1]),
            raw(&[0xc1, 0, 2]),
            raw(&[0xc3, 0, 3]),
            raw(&[0xc1, 0, 4]),
        ]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        link.write_checked_sequence(&[
            (Addr(0x0100_0000), vec![0x14]),
            (Addr(0x0100_0002), vec![0]),
            (Addr(0x0010_0003), vec![0]),
        ])?;
        assert_eq!(link.pktno.expected(), Some(5));
        let (mock, _) = link.into_parts();
        assert_eq!(
            mock.writes,
            [
                vec![0xc3, 0, 1, 0, 0],
                vec![0xc1, 0, 0, 0, 0],
                vec![0x14],
                vec![0xc1, 2, 0, 0, 0],
                vec![0],
                vec![0xc3, 0x10, 0, 0, 0],
                vec![0xc1, 3, 0, 0, 0],
                vec![0],
            ]
        );
        Ok(())
    }
    #[test]
    fn flag1_and_following_bank_share_the_vendor_counter_sequence() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads.extend([
            raw(&[0xc3, 0, 7]),
            raw(&[0xc1, 0, 8]),
            raw(&[0xc3, 0, 9]),
            raw(&[0xc1, 0, 10]),
        ]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        link.write_checked(
            lp_proto::regs::cap::FLAG1,
            &[0],
            Some(lp_proto::regs::ctrl::ARM),
        )?;
        link.write(lp_proto::regs::ctrl::ARM, &[0])?;
        assert_eq!(link.pktno.expected(), Some(11));
        Ok(())
    }
    #[test]
    fn divider_bytes_use_the_ordinary_contiguous_vendor_ack_sequence() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads.extend([
            raw(&[0xc3, 0, 3]),
            raw(&[0xc1, 0, 4]),
            raw(&[0xc1, 0, 5]),
            raw(&[0xc3, 0, 6]),
            raw(&[0xc1, 0, 7]),
        ]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        link.write_checked_sequence(&[
            (lp_proto::regs::rate::R0, vec![0x21]),
            (lp_proto::regs::rate::R1, vec![0]),
            (lp_proto::regs::cap::MODE, vec![0x14]),
        ])?;
        assert_eq!(link.pktno.expected(), Some(8));
        let (mock, _) = link.into_parts();
        assert_eq!(
            mock.writes
                .iter()
                .filter(|write| write.first() == Some(&0xc1))
                .count(),
            3,
            "R1 must use the same checked C1 path as every other field"
        );
        Ok(())
    }
    #[test]
    fn reset_requires_a_complete_ack() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads.extend([raw(&[0xc3, 0, 0])]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        assert!(matches!(
            link.write(lp_proto::regs::ctrl::RESET, &[1]),
            Err(DevError::Timeout { .. })
        ));
        let (mock, _) = link.into_parts();
        assert_eq!(mock.writes[0], [0xc3, 0x10, 0, 0, 0]);
        assert_eq!(mock.writes[1], [0xc1, 2, 0, 0, 0]);
        assert_eq!(mock.writes[2], [1]);
        Ok(())
    }
    #[test]
    fn acknowledged_reset_still_reestablishes_the_next_bank() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads
            .extend([raw(&[0xc3, 0, 7]), raw(&[0xc1, 0, 8]), raw(&[0xc3, 0, 9])]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        link.write_checked(
            lp_proto::regs::ctrl::RESET,
            &[1],
            Some(lp_proto::regs::trig::A_BASE),
        )?;
        assert_eq!(link.pktno.expected(), Some(10));
        let (mock, _) = link.into_parts();
        assert_eq!(
            mock.writes,
            [
                vec![0xc3, 0x10, 0, 0, 0],
                vec![0xc1, 2, 0, 0, 0],
                vec![1],
                vec![0xc3, 0x20, 0, 0, 0],
            ]
        );
        Ok(())
    }
    #[test]
    fn trigger_fields_use_the_ordinary_checked_write_path() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads
            .extend([raw(&[0xc3, 0, 7]), raw(&[0xc1, 0, 8]), raw(&[0xc1, 0, 9])]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        link.write_checked(
            lp_proto::regs::trig::A_BASE.offset(lp_proto::regs::trig::RANGE_ARMED),
            &[0],
            Some(lp_proto::regs::trig::A_BASE.offset(lp_proto::regs::trig::COUNT1)),
        )?;
        link.write(
            lp_proto::regs::trig::A_BASE.offset(lp_proto::regs::trig::COUNT1),
            &[0, 0, 0, 0],
        )?;
        assert_eq!(link.pktno.expected(), Some(10));
        let (mock, _) = link.into_parts();
        assert_eq!(
            mock.writes,
            [
                vec![0xc3, 0x20, 0, 0, 0],
                vec![0xc1, 25, 0, 0, 0],
                vec![0],
                vec![0xc1, 14, 0, 3, 0],
                vec![0, 0, 0, 0],
            ]
        );
        Ok(())
    }
    #[test]
    fn nonzero_m22_requires_the_complete_ack_seen_with_d2xx() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads
            .extend([raw(&[0xc3, 0, 21]), raw(&[0xc1, 0, 22])]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());

        link.write_checked(
            lp_proto::regs::trig::A_BASE.offset(lp_proto::regs::trig::M22),
            &[3],
            None,
        )?;

        assert_eq!(link.pktno.expected(), Some(23));
        let (mock, _) = link.into_parts();
        assert_eq!(
            mock.writes,
            [
                vec![0xc3, 0x20, 0, 0, 0],
                vec![0xc1, lp_proto::regs::trig::M22 as u8, 0, 0, 0],
                vec![3],
            ]
        );
        Ok(())
    }
    #[test]
    fn partial_m22_ack_is_a_hard_failure() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads.extend([raw(&[0xc3, 0, 21]), raw(&[0xc1, 0])]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());

        assert!(matches!(
            link.write_checked(
                lp_proto::regs::trig::A_BASE.offset(lp_proto::regs::trig::M22),
                &[0],
                None,
            ),
            Err(DevError::Timeout { wanted: 3, got: 2 })
        ));
        Ok(())
    }
    #[test]
    fn checked_write_reselects_an_unanswered_bank_before_retry() -> Result<(), DevError> {
        let mock = Mock::default();
        let config = LinkConfig {
            bank_timeout: Duration::from_millis(5),
            ..LinkConfig::default()
        };
        let mut link = Link::new(mock, VirtualClock::default(), config);

        assert!(matches!(
            link.write(Addr(0x2000_0000), &[0x11]),
            Err(DevError::Timeout { .. })
        ));
        link.transport
            .reads
            .extend([raw(&[0xc3, 0, 7]), raw(&[0xc1, 0, 8])]);
        link.write(Addr(0x2000_0000), &[0x11])?;

        let (mock, _) = link.into_parts();
        assert_eq!(
            mock.writes,
            [
                vec![0xc3, 0, 0x20, 0, 0],
                vec![0xc3, 0, 0x20, 0, 0],
                vec![0xc1, 0, 0, 0, 0],
                vec![0x11],
            ]
        );
        Ok(())
    }
    #[test]
    fn read_packet_drift_self_heals_with_resync_retry() -> Result<(), DevError> {
        // A single FIFO packet-counter drift on a read must self-heal: the
        // driver purges/resyncs and retries once, transparently returning valid
        // data instead of surfacing a USB error that needs a power-cycle.
        let mut mock = Mock::default();
        mock.reads.extend([
            raw(&[0xc3, 0, 0]),       // C3 bank ack pktno 0 -> expect next 1
            raw(&[0xc2, 0, 2, 0x17]), // C2 read pktno 2 != 1 -> drift, triggers resync
            raw(&[0xc3, 0, 5]),       // retry C3 ack pktno 5 adopted -> next 6
            raw(&[0xc2, 0, 6, 0x17]), // retry C2 read pktno 6 matches -> data 0x17
        ]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        assert_eq!(link.read(Addr(0x0010_000f), 1)?, [0x17]);
        // The recovered drift is still counted as a USB error, so a soak can
        // assert it never happens once the capture-boundary purge is in place.
        assert_eq!(link.stats().usb_error_count, 1);
        Ok(())
    }
    #[test]
    fn read_packet_drift_twice_is_a_hard_error() -> Result<(), DevError> {
        // NEGATIVE GATE: a drift that persists through the one retry is a real
        // fault and must surface, never be masked by an unbounded resync loop.
        let mut mock = Mock::default();
        mock.reads.extend([
            raw(&[0xc3, 0, 0]),
            raw(&[0xc2, 0, 2, 0x17]), // 1st drift
            raw(&[0xc3, 0, 5]),
            raw(&[0xc2, 0, 9, 0x17]), // retry drifts again -> hard error
        ]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        assert!(matches!(
            link.read(Addr(0x0010_000f), 1),
            Err(DevError::Protocol(ProtoError::PacketNumber { .. }))
        ));
        Ok(())
    }
    #[test]
    fn checked_write_packet_mismatch_is_not_retried() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads.extend([
            raw(&[0xc3, 0, 0]),
            raw(&[0xc1, 0xc1, 0]),
            raw(&[0xc1, 0, 1]),
        ]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        assert!(matches!(
            link.write(Addr(0x2000_0000), &[0x11]),
            Err(DevError::Protocol(ProtoError::PacketNumber {
                expected: 1,
                got: 0xc100
            }))
        ));
        let (mock, _) = link.into_parts();
        assert_eq!(
            mock.writes,
            [
                vec![0xc3, 0, 0x20, 0, 0],
                vec![0xc1, 0, 0, 0, 0],
                vec![0x11],
            ]
        );
        Ok(())
    }
    #[test]
    fn read_strips_status_and_returns_payload() -> Result<(), DevError> {
        let mut mock = Mock::default();
        mock.reads
            .extend([raw(&[0xc3, 0, 0]), raw(&[0xc2, 0, 1, 0xaa, 0xbb])]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        assert_eq!(link.read(Addr(0x0010_000d), 2)?, [0xaa, 0xbb]);
        Ok(())
    }

    #[test]
    fn read_retries_cancelled_bulk_in_within_deadline() -> Result<(), DevError> {
        let mut mock = Mock {
            reads_before_cancel: 2,
            cancelled_reads: 1,
            ..Mock::default()
        };
        mock.reads
            .extend([raw(&[0xc3, 0, 0]), raw(&[0xc2]), raw(&[0, 1, 0xaa, 0xbb])]);
        let mut link = Link::new(mock, VirtualClock::default(), LinkConfig::default());
        assert_eq!(link.read(Addr(0x0010_000d), 2)?, [0xaa, 0xbb]);
        Ok(())
    }
}
