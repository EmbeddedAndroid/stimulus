# LogicPort LA1034 protocol

This is the protocol reference for the LogicPort LA1034 logic analyzer: the USB
transport framing, the device register map, the timing-rate table, FPGA image
selection and configuration, threshold conversion, and the setup, arming, and
readback sequence. The behaviour described here is exercised by the running
software and has been proven end to end on real hardware across hundreds of
thousands of captures. In the tables, a fact is marked `hardware-verified` when
it is part of that working path, and `reference` when it is documented for
completeness but not exercised by the current software.

## Transport framing

Commands are five bytes: `[opcode, addr_lo, addr_hi, (len-1)_lo, (len-1)_hi]`.
Addresses and lengths are 16-bit little-endian, with lengths biased by one and restricted
to 1–65536. `C2` reads, `C1` writes (payload follows), and `C3` selects `addr >> 16` with a
zero length field. Responses are `[opcode, pktno_hi, pktno_lo, data...]`; the 16-bit
big-endian transaction counter is shared by all opcodes. `C1` and `C3` acknowledge with
the three-byte header. Every 64-byte FTDI IN packet begins with two modem-status bytes
which are removed before response parsing. Opcode and counter mismatches require purge
and status-based resynchronisation.

The host does not delay between commands. Readiness is the complete response header and
payload; back-to-back FT245 reads are naturally paced by the four-millisecond latency
timer and stop only at the command deadline.

The transport uses FTDI's default 4096-byte USB IN request size. It keeps four IN
requests in flight at all times, submitting a replacement the moment one completes and
before that completion is delivered to the command thread. This keeps a read window
continuously armed, so the device FIFO is always being drained and no command begins in
an uncovered receive gap. The window is maintained continuously, including before the
first FIFO OUT after a purge; reads are never armed on demand after a command has already
been transmitted. During async bit-bang configuration the same draining consumes as many
4 KiB completions as each 10,000-byte configuration chunk generates.

| Fact | Evidence |
|---|---|
| C1/C2/C3 framing and biased length | hardware-verified |
| shared big-endian packet counter | hardware-verified |
| two status bytes per 64-byte FTDI packet | hardware-verified |
| segment cache resets to -1 on open | hardware-verified |

## Register map

The device uses 32-bit byte addresses. A changed upper 16-bit segment is sent with
`C3`; the low 16 bits are carried by `C1`/`C2`. Integer values are little-endian.

| Address | Dir | Width | Meaning | Evidence |
|---|---:|---:|---|---|
| `0x00100000` | W | 1 | post-config command (`00` before bit-bang, `61` after open) | hardware-verified |
| `0x00100001` | W | 1 | configuration auxiliary byte | reference |
| `0x00100002` | W | 1 | reset / clear-status strobe (`1`) | hardware-verified |
| `0x00100003` | W | 1 | arm (`1`) / stop (`0`) | hardware-verified |
| `0x00100004` | W | 2 | pre-trigger count | hardware-verified |
| `0x00100006` | W | 2 | post-trigger count | hardware-verified |
| `0x00100008` | W | 2 | threshold DAC (10-bit) | hardware-verified |
| `0x0010000a` | W | 1 | force trigger from PREFILL | hardware-verified |
| `0x0010000b` | W | 1 | force stop from POSTFILL | hardware-verified |
| `0x0010000c` | W | 1 | force trigger from ARMED | hardware-verified |
| `0x0010000d` | W | 1 | trigger combine-mode select | hardware-verified |
| `0x00100000` | R | 1 | status: bit0 busy, bit6 prefill done, bit5 triggered | hardware-verified |
| `0x00100001` | R | 9 | wire high/low history and CLK status | hardware-verified |
| `0x0010000a` | R | 2 | post-trigger sample count | hardware-verified |
| `0x0010000c` | R | 1 | DDR readback auxiliary | reference |
| `0x0010000d` | R | 2 | device/version word | reference |
| `0x0010000f` | R | 1 | FPGA image ID (`index \| 0x10`) | hardware-verified |
| `0x01000000` | W | 1 | sample MODE byte | hardware-verified |
| `0x01000001` | W | 1 | sample-mode flag | hardware-verified |
| `0x01000002` | W | 1 | channel-mask commit gate: write `0`, write CH_MASK and MASK2, then write `1`; both gate writes require complete C1 acknowledgements | hardware-verified |
| `0x01000003` | W | 5 | 34-bit channel-enable mask; always written, including all-enabled `ff ff ff ff 03` | hardware-verified |
| `0x01000008` | W | 5 | second 34-bit mask (candidate: logic sense) | reference |
| `0x01000000` | R | 2 | trigger page position 0–2047 | hardware-verified |
| `0x02000000` | R | 2 | last-written SDR/phase-A page | hardware-verified |
| `0x02000100` | R | 2 | DDR phase-B page | hardware-verified |
| `0x02001000`…`0x02004000` + page | R | 1×4 | D0–D31 phase-A blocks | hardware-verified |
| `0x02005000`…`0x02008000` + page | R | 1×4 | D0–D31 DDR phase-B blocks | hardware-verified |
| `0x02009000` + page | R | 1 | CLK bits or 35-bit RLE count high bits | hardware-verified |
| `0x00200000` + offsets 0…50 | R/W | 51 | trigger term set A | hardware-verified |
| `0x00600000` + offsets 0…50 | R/W | 51 | trigger term set B (`0x00400000` is not a valid base) | hardware-verified |
| `0x20000000` | W | 1 | timing divider byte 0 | hardware-verified |
| `0x20000001` | W | 1 | timing divider byte 1 | hardware-verified |
| `0x40000000` | W | 1 | frequency-counter source | hardware-verified |
| `0x40000000` | R | 1 | frequency-counter ready (ready when zero) | hardware-verified |
| `0x40000001` | R | 4 | frequency counter, Hz = value × 10 | hardware-verified |
| `0x40000005` | R | 4 | hardware-alive auxiliary counter | reference |

## Timing-rate table

| Index | Rate | Mult | Byte 0 | Byte 1 | Evidence |
|---:|---:|---:|---:|---:|---|
| 0 | 1 GHz (unsupported QDR, rejected) | 4 | `00` | `00` | reference |
| 1 | 500 MHz | 2 | `00` | `00` | hardware-verified |
| 2 | 250 MHz | 1 | `00` | `00` | hardware-verified |
| 3 | 200 MHz | 1 | `01` | `00` | hardware-verified |
| 4 | 100 MHz | 1 | `11` | `00` | hardware-verified |
| 5 | 50 MHz | 1 | `11` | `11` | hardware-verified |
| 6 | 20 MHz | 1 | `11` | `44` | hardware-verified |
| 7 | 10 MHz | 1 | `21` | `00` | hardware-verified |
| 8 | 5 MHz | 1 | `21` | `11` | hardware-verified |
| 9 | 2 MHz | 1 | `21` | `44` | hardware-verified |
| 10 | 1 MHz | 1 | `31` | `00` | hardware-verified |
| 11 | 500 kHz | 1 | `31` | `11` | hardware-verified |
| 12 | 200 kHz | 1 | `31` | `44` | hardware-verified |
| 13 | 100 kHz | 1 | `41` | `00` | hardware-verified |
| 14 | 50 kHz | 1 | `41` | `11` | hardware-verified |
| 15 | 20 kHz | 1 | `41` | `44` | hardware-verified |
| 16 | 10 kHz | 1 | `51` | `00` | hardware-verified |
| 17 | 5 kHz | 1 | `51` | `11` | hardware-verified |
| 18 | 2 kHz | 1 | `51` | `44` | hardware-verified |
| 19 | 1 kHz | 1 | `61` | `00` | hardware-verified |

## FPGA selection

Timing indices below 2 use image 6; other timing rates use image 7. State clock codes
0/1/2/3 use images 1/3/2/4. Modes 2 and 3 use images 0 and 5 respectively. A warm start
skips configuration when register `0x0010000f` equals `index | 0x10`.

The CCF stores each waveform with its first two 4096-byte blocks exchanged. Configuration
therefore uploads `image[4096..8192]`, then `image[0..4096]`, then the remainder, while
retaining the original slice for container inspection. Decoding bit 0 of each odd waveform
byte LSB-first after this swap yields the Cyclone passive-serial prefix `ff` × 16,
`6a d6 ff 40 00`; the following 24-bit little-endian word equals the image's data-bit count
minus 16 for all eight images. The configuration upload applies the same block swap before
its 10,000-byte write loop.

Cold configuration resets the FTDI, sets latency 4 and baud divisor `0x4006`, purges RX and
TX, enters async bit-bang mode `0x0107`, writes `00`, writes `04`, waits for nSTATUS, and
streams the swapped image in 10,000-byte chunks. Bulk IN is drained throughout bit-bang
operation because FT245 pin samples accumulate in RX; the run reports the total drained
byte count. CONF_DONE is polled for at most 1000 ms after the final bulk OUT completes,
followed by a final pin check and bit-mode reset. In FIFO mode the host then writes 8712
zero bytes followed by `01`, purges RX, invalidates the segment cache, and resets the
expected response counter to zero. This session tail is the synchronization boundary
before any FIFO command.

Register `0x00100000` is the FPGA control port. `0x00` releases a configured image for
reconfiguration; `0x61` enables the normal command interface after configuration. A warm
configuration first reads IMAGE_ID and returns immediately when it already equals
`index | 0x10` in an established session. Otherwise it writes release, tolerates a missing
three-byte acknowledgement, invalidates bank and packet state, and makes at most 20 release
probes. Each probe purges RX, resets bit mode, enters async bit-bang with mask `0x07`,
writes `00`, and records the pin read; CONF_DONE must fall before the cold stream begins.
After configuration, the flush sequence resets the FTDI, restores latency 4 and baud
`0x4006`, and purges RX before the post-configure `0x61` write and IMAGE_ID verification.
Missing or partial IMAGE_ID responses are protocol errors handled by purge and one bounded
reissue; no fixed settle delay is inserted. A single full re-stream recovery is allowed
only when the same connection just streamed that known target and observed CONF_DONE;
unknown configured images are never reset.

The acknowledged post-configure `0x61` is the session's one-time enable. It is not repeated
after IMAGE_ID verification; a duplicate `0x61` would gate the FIFO parser, so setup
proceeds directly to bank `0x20`.

## Threshold conversion

`Vadj = (Vth - 1.315) * -0.4020381098624974 + 1.315`; then
`code = clamp(round(((4.559 - Vadj) / 5.875) * 1023) + cal_offset, 0, 1023)`.
The calibration offset comes from the decoded FTDI user-area data. The EEPROM is read-only
in this implementation.

## Setup and acquisition order

After configuration reaches DONE, the FIFO session tail (`00` × 8712, then `01`) readies
the parser. There are no fixed inter-command waits. Every C1, C2, and C3 response is read
by back-to-back FT245 IN transfers until its complete expected byte count is present or
the command deadline expires. Opcode and packet number are verified. A mismatch purges
RX and permits one reissue of C1/C2; C3 is not blindly retransmitted after an unverified
bank transition. Setup advances only after a complete validated acknowledgement.

An RX purge clears the host receive queue and suppresses completions across the FTDI control
request while leaving the four-request read window armed. Read-ahead is not cancelled before
the request, which would otherwise create an artificial gap in the IN direction. Electrical
bit-mode transitions still cancel and re-establish the window.

The rate divider is written as separate one-byte assignments to `0x00200000` and
`0x00200001`; each changed byte uses the strict one-second acknowledgement gate, so byte 1
is never sent after an unacknowledged byte 0. Unchanged divider pairs are not rewritten.
Combining the bytes into one C1 transaction is not the correct sequence. No clock-domain
sleep follows either byte; the complete response is the readiness signal.

The dirty-setting order is rate, mode/masks, trigger (after disarm/reset), threshold,
position, then arm. Acquisition status phases are PREFILL (`bit6=0`), ARMED (`bit6=1,
bit5=0`), POSTFILL (`bit6=1,bit5=1`), and complete (`bit0=0`). Phase-specific force
registers are used for timeout or abort. SDR reads four channel blocks and flags from the
2048-page ring; DDR reads both block sets. On RLE pages, count is
`((flags & 7)<<32)|block3<<24|block2<<16|block1<<8|block0` and repeats the prior sample.
