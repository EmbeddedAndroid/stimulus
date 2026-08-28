pub const MAGIC: [u8; 4] = *b"LPC1";
pub const HEADER_LEN: usize = 16;
pub const RLE_SLOT_LEN: usize = 12;
pub const EXPANDED_SAMPLE_LEN: usize = 8;
pub const FLAG_LAST: u8 = 1;
pub const FLAG_CHANNEL_SUBSET: u8 = 2;
pub const SLOT_CLK1: u32 = 1;
pub const SLOT_CLK2: u32 = 2;
pub const SLOT_TRIGGER: u32 = 4;
pub const SLOT_REFERENCE: u32 = 8;
pub const SLOT_CONTINUES: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Rle = 0,
    Expanded = 1,
}
impl TryFrom<u8> for Kind {
    type Error = BinaryError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Rle),
            1 => Ok(Self::Expanded),
            other => Err(BinaryError::Kind(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub flags: u8,
    pub channel_count: u16,
    pub capture_id: u32,
    pub chunk_index: u16,
    pub chunk_count: u16,
}
impl Header {
    pub fn encode(self) -> [u8; HEADER_LEN] {
        let mut out = [0_u8; HEADER_LEN];
        out[..4].copy_from_slice(&MAGIC);
        out[4] = self.kind as u8;
        out[5] = self.flags;
        out[6..8].copy_from_slice(&self.channel_count.to_le_bytes());
        out[8..12].copy_from_slice(&self.capture_id.to_le_bytes());
        out[12..14].copy_from_slice(&self.chunk_index.to_le_bytes());
        out[14..16].copy_from_slice(&self.chunk_count.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, BinaryError> {
        if bytes.len() < HEADER_LEN {
            return Err(BinaryError::Length {
                expected: HEADER_LEN,
                got: bytes.len(),
            });
        }
        if bytes[..4] != MAGIC {
            return Err(BinaryError::Magic);
        }
        let header = Self {
            kind: Kind::try_from(bytes[4])?,
            flags: bytes[5],
            channel_count: u16::from_le_bytes([bytes[6], bytes[7]]),
            capture_id: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            chunk_index: u16::from_le_bytes([bytes[12], bytes[13]]),
            chunk_count: u16::from_le_bytes([bytes[14], bytes[15]]),
        };
        if header.channel_count == 0
            || header.channel_count > 34
            || header.chunk_count == 0
            || header.chunk_index >= header.chunk_count
        {
            return Err(BinaryError::Header);
        }
        Ok(header)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RleSlot {
    pub data: u32,
    pub flags: u32,
    pub count: u32,
}
impl RleSlot {
    pub fn encode(self) -> [u8; RLE_SLOT_LEN] {
        let mut out = [0; RLE_SLOT_LEN];
        out[..4].copy_from_slice(&self.data.to_le_bytes());
        out[4..8].copy_from_slice(&self.flags.to_le_bytes());
        out[8..].copy_from_slice(&self.count.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, BinaryError> {
        if bytes.len() != RLE_SLOT_LEN {
            return Err(BinaryError::Length {
                expected: RLE_SLOT_LEN,
                got: bytes.len(),
            });
        }
        let value = Self {
            data: u32::from_le_bytes(bytes[..4].try_into().map_err(|_| BinaryError::Header)?),
            flags: u32::from_le_bytes(bytes[4..8].try_into().map_err(|_| BinaryError::Header)?),
            count: u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| BinaryError::Header)?),
        };
        if value.count == 0 {
            return Err(BinaryError::ZeroCount);
        }
        Ok(value)
    }
}

pub fn encode_rle(header: Header, slots: &[RleSlot]) -> Result<Vec<u8>, BinaryError> {
    if header.kind != Kind::Rle {
        return Err(BinaryError::KindMismatch);
    }
    let mut out = Vec::with_capacity(HEADER_LEN + slots.len() * RLE_SLOT_LEN);
    out.extend_from_slice(&header.encode());
    for slot in slots {
        if slot.count == 0 {
            return Err(BinaryError::ZeroCount);
        }
        out.extend_from_slice(&slot.encode());
    }
    Ok(out)
}
pub fn decode_rle(bytes: &[u8]) -> Result<(Header, Vec<RleSlot>), BinaryError> {
    let header = Header::decode(bytes)?;
    if header.kind != Kind::Rle {
        return Err(BinaryError::KindMismatch);
    }
    let payload = &bytes[HEADER_LEN..];
    if !payload.len().is_multiple_of(RLE_SLOT_LEN) {
        return Err(BinaryError::Alignment);
    }
    let slots = payload
        .as_chunks::<RLE_SLOT_LEN>()
        .0
        .iter()
        .map(|bytes| RleSlot::decode(bytes))
        .collect::<Result<_, _>>()?;
    Ok((header, slots))
}
pub fn encode_expanded(header: Header, samples: &[u64]) -> Result<Vec<u8>, BinaryError> {
    if header.kind != Kind::Expanded {
        return Err(BinaryError::KindMismatch);
    }
    let mut out = Vec::with_capacity(HEADER_LEN + samples.len() * 8);
    out.extend_from_slice(&header.encode());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(out)
}
pub fn decode_expanded(bytes: &[u8]) -> Result<(Header, Vec<u64>), BinaryError> {
    let header = Header::decode(bytes)?;
    if header.kind != Kind::Expanded {
        return Err(BinaryError::KindMismatch);
    }
    let payload = &bytes[HEADER_LEN..];
    if !payload.len().is_multiple_of(8) {
        return Err(BinaryError::Alignment);
    }
    Ok((
        header,
        payload
            .as_chunks::<8>()
            .0
            .iter()
            .map(|value| u64::from_le_bytes(*value))
            .collect(),
    ))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BinaryError {
    #[error("invalid LPC1 magic")]
    Magic,
    #[error("unknown LPC1 kind {0}")]
    Kind(u8),
    #[error("LPC1 kind does not match payload codec")]
    KindMismatch,
    #[error("invalid LPC1 header fields")]
    Header,
    #[error("invalid length: expected {expected}, got {got}")]
    Length { expected: usize, got: usize },
    #[error("payload is not aligned to its record width")]
    Alignment,
    #[error("RLE count must be positive")]
    ZeroCount,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header(kind: Kind) -> Header {
        Header {
            kind,
            flags: FLAG_LAST,
            channel_count: 34,
            capture_id: 42,
            chunk_index: 0,
            chunk_count: 1,
        }
    }
    #[test]
    fn header_is_exactly_sixteen_little_endian_bytes() {
        let bytes = header(Kind::Rle).encode();
        assert_eq!(bytes.len(), 16);
        assert_eq!(&bytes[..6], b"LPC1\0\x01");
        assert_eq!(&bytes[8..12], &42_u32.to_le_bytes());
        assert_eq!(Header::decode(&bytes), Ok(header(Kind::Rle)));
    }
    #[test]
    fn rle_and_expanded_round_trip() {
        let slots = [RleSlot {
            data: 0x1234,
            flags: SLOT_CLK1 | SLOT_TRIGGER,
            count: 99,
        }];
        let bytes = encode_rle(header(Kind::Rle), &slots).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(decode_rle(&bytes), Ok((header(Kind::Rle), slots.to_vec())));
        let samples = [0_u64, 1_u64 << 41, 0x3_ffff_ffff];
        let bytes =
            encode_expanded(header(Kind::Expanded), &samples).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            decode_expanded(&bytes),
            Ok((header(Kind::Expanded), samples.to_vec()))
        );
    }
    #[test]
    fn rejects_corruption_and_zero_counts() {
        assert_eq!(
            Header::decode(b"bad"),
            Err(BinaryError::Length {
                expected: 16,
                got: 3
            })
        );
        assert_eq!(RleSlot::decode(&[0; 12]), Err(BinaryError::ZeroCount));
        let mut bytes = header(Kind::Rle).encode();
        bytes[4] = 9;
        assert_eq!(Header::decode(&bytes), Err(BinaryError::Kind(9)));
    }
}
