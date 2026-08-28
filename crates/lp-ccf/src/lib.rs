use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub const HEADER_XOR: u32 = 0x91_66_62;
pub const IMAGE_COUNT: usize = 8;
pub const PINNED_SHA256: &str = "9298a5eacb3cf1a791b8ce48460a19e031eb81baffa378bab3f94f2d78483f9c";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub raw: [u32; 9],
    pub lengths: [u32; IMAGE_COUNT],
    pub checksum_ok: bool,
}

pub fn parse_header(line: &str) -> Result<Header> {
    let values = line
        .trim_end_matches(['\r', '\n'])
        .split(',')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()?;
    let raw: [u32; 9] = values.try_into().map_err(|v: Vec<u32>| {
        anyhow::anyhow!("CCF header must contain 9 fields, got {}", v.len())
    })?;
    let decoded = raw.map(|value| value ^ HEADER_XOR);
    let lengths = std::array::from_fn(|i| decoded[i] * 16);
    let checksum_ok = decoded[8] == decoded[..IMAGE_COUNT].iter().fold(0, |sum, n| sum ^ n);
    ensure!(checksum_ok, "CCF header checksum mismatch");
    Ok(Header {
        raw,
        lengths,
        checksum_ok,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageRange {
    offset: usize,
    length: usize,
}

#[derive(Debug, Clone)]
pub struct Ccf {
    bytes: Vec<u8>,
    header: Header,
    ranges: [ImageRange; IMAGE_COUNT],
}

impl Ccf {
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        let newline = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .context("missing CCF header")?;
        let header = parse_header(std::str::from_utf8(&bytes[..newline])?)?;
        let mut offset = newline + 1;
        let ranges = std::array::from_fn(|i| {
            let length = header.lengths[i] as usize;
            let range = ImageRange { offset, length };
            offset += length;
            range
        });
        ensure!(
            offset == bytes.len(),
            "CCF image lengths do not cover body exactly"
        );
        let ccf = Self {
            bytes,
            header,
            ranges,
        };
        for idx in 0..IMAGE_COUNT as u8 {
            validate_waveform(ccf.image(idx)?)?;
        }
        Ok(ccf)
    }

    pub fn load(path: impl AsRef<Path>, require_pin: bool) -> Result<Self> {
        let bytes = fs::read(path.as_ref())
            .with_context(|| format!("reading {}", path.as_ref().display()))?;
        let ccf = Self::parse(bytes)?;
        if require_pin {
            ensure!(
                ccf.sha256() == PINNED_SHA256,
                "CCF SHA-256 does not match pinned vendor fixture"
            );
        }
        Ok(ccf)
    }

    pub fn header(&self) -> &Header {
        &self.header
    }
    pub fn image(&self, idx: u8) -> Result<&[u8]> {
        let range = self
            .ranges
            .get(idx as usize)
            .context("CCF image index out of range")?;
        Ok(&self.bytes[range.offset..range.offset + range.length])
    }
    pub fn image_for_upload(&self, idx: u8) -> Result<Vec<u8>> {
        let image = self.image(idx)?;
        ensure!(
            image.len() >= 8192,
            "CCF image is too short for the 4096-byte block swap"
        );
        let mut upload = Vec::with_capacity(image.len());
        upload.extend_from_slice(&image[4096..8192]);
        upload.extend_from_slice(&image[..4096]);
        upload.extend_from_slice(&image[8192..]);
        Ok(upload)
    }
    pub fn sha256(&self) -> String {
        hex(&Sha256::digest(&self.bytes))
    }
}

pub fn validate_waveform(image: &[u8]) -> Result<()> {
    ensure!(!image.is_empty(), "FPGA waveform is empty");
    ensure!(
        image.len().is_multiple_of(16),
        "FPGA waveform length is not 16-byte aligned"
    );
    for (pair_idx, pair) in image.as_chunks::<2>().0.iter().enumerate() {
        ensure!(
            (4..=7).contains(&pair[0]) && (4..=7).contains(&pair[1]),
            "invalid waveform byte in pair {pair_idx}"
        );
        ensure!(
            pair[0] & 2 == 0 && pair[1] & 2 == 2,
            "CCLK does not toggle in pair {pair_idx}"
        );
        ensure!(
            pair[0] & 1 == pair[1] & 1,
            "DIN changes within clock pair {pair_idx}"
        );
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        include_bytes!("../../../fixtures/vendor/LogicPort.ccf").to_vec()
    }

    fn decode_data(image: &[u8]) -> Vec<u8> {
        image
            .as_chunks::<16>()
            .0
            .iter()
            .map(|chunk| (0..8).fold(0, |byte, bit| byte | ((chunk[bit * 2 + 1] & 1) << bit)))
            .collect()
    }

    const PREAMBLE: [u8; 21] = [
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x6a, 0xd6, 0xff, 0x40, 0x00,
    ];

    #[test]
    fn upload_images_have_passive_serial_preamble_and_length() -> Result<()> {
        let ccf = Ccf::parse(fixture())?;
        for idx in 0..IMAGE_COUNT as u8 {
            let upload = ccf.image_for_upload(idx)?;
            let decoded = decode_data(&upload);
            assert_eq!(&decoded[..PREAMBLE.len()], &PREAMBLE, "image {idx}");
            let encoded_length = u32::from(decoded[21])
                | (u32::from(decoded[22]) << 8)
                | (u32::from(decoded[23]) << 16);
            assert_eq!(
                encoded_length as usize,
                upload.len() / 2 - 16,
                "image {idx}"
            );
        }
        Ok(())
    }

    #[test]
    fn raw_images_do_not_have_passive_serial_preamble() -> Result<()> {
        let ccf = Ccf::parse(fixture())?;
        for idx in 0..IMAGE_COUNT as u8 {
            let decoded = decode_data(ccf.image(idx)?);
            assert_ne!(&decoded[..PREAMBLE.len()], &PREAMBLE, "image {idx}");
        }
        Ok(())
    }

    #[test]
    fn header_decodes_vendor_line() {
        let bytes = fixture();
        let newline = bytes
            .iter()
            .position(|b| *b == b'\n')
            .unwrap_or_else(|| panic!("missing newline"));
        let header =
            parse_header(std::str::from_utf8(&bytes[..newline]).unwrap_or_else(|e| panic!("{e}")))
                .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            header.lengths,
            [
                1_057_840, 1_044_512, 1_046_896, 1_047_328, 1_046_512, 1_054_992, 1_031_232,
                1_058_384
            ]
        );
    }
    #[test]
    fn bad_checksum_rejected() {
        let mut line =
            "9462817,9541984,9542133,9542096,9541917,9463795,9543078,9462791,9477193".to_owned();
        line.replace_range(..1, "8");
        assert!(parse_header(&line).is_err());
    }
    #[test]
    fn offsets_tile_body_exactly() {
        let ccf = Ccf::parse(fixture()).unwrap_or_else(|e| panic!("{e}"));
        for p in ccf.ranges.windows(2) {
            assert_eq!(p[0].offset + p[0].length, p[1].offset)
        }
        let last = &ccf.ranges[7];
        assert_eq!(last.offset + last.length, ccf.bytes.len());
    }
    #[test]
    fn fixture_sha256_pinned() {
        assert_eq!(
            Ccf::parse(fixture())
                .unwrap_or_else(|e| panic!("{e}"))
                .sha256(),
            PINNED_SHA256
        );
    }
    #[test]
    fn every_image_is_valid_waveform() {
        let ccf = Ccf::parse(fixture()).unwrap_or_else(|e| panic!("{e}"));
        for i in 0..8 {
            validate_waveform(ccf.image(i).unwrap_or_else(|e| panic!("{e}")))
                .unwrap_or_else(|e| panic!("{e}"));
        }
    }
    #[test]
    fn image_index_out_of_range() {
        assert!(
            Ccf::parse(fixture())
                .unwrap_or_else(|e| panic!("{e}"))
                .image(8)
                .is_err()
        );
    }
}
