//! Parser for the ISO BMFF `sidx` (segment index) box used by single-file DASH.

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct IndexedSegment {
    pub offset: u64,
    pub size: u64,
    pub duration: f64,
}

/// Parse the first `sidx` box found in `data`, which starts at absolute file offset
/// `data_offset`. Returns the media subsegments it references.
pub fn parse_sidx(data: &[u8], data_offset: u64) -> Result<Vec<IndexedSegment>> {
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let size32 = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as u64;
        let kind = &data[pos + 4..pos + 8];
        let (header, size) = match size32 {
            1 if pos + 16 <= data.len() => (
                16usize,
                u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap()),
            ),
            0 => (8usize, (data.len() - pos) as u64),
            _ => (8usize, size32),
        };
        if size < header as u64 {
            return Err(Error::Parse("corrupt box size in segment index".into()));
        }
        let end = pos.saturating_add(size as usize).min(data.len());
        if kind == b"sidx" {
            let body = &data[pos + header..end];
            return parse_body(body, data_offset + end as u64);
        }
        pos = end;
        if size as usize == 0 {
            break;
        }
    }
    Err(Error::Parse("no sidx box in index range".into()))
}

fn parse_body(b: &[u8], anchor: u64) -> Result<Vec<IndexedSegment>> {
    let rd32 = |at: usize| -> Result<u64> {
        b.get(at..at + 4)
            .map(|s| u32::from_be_bytes(s.try_into().unwrap()) as u64)
            .ok_or_else(|| Error::Parse("truncated sidx".into()))
    };
    let rd64 = |at: usize| -> Result<u64> {
        b.get(at..at + 8)
            .map(|s| u64::from_be_bytes(s.try_into().unwrap()))
            .ok_or_else(|| Error::Parse("truncated sidx".into()))
    };
    let version = *b.first().ok_or_else(|| Error::Parse("empty sidx".into()))?;
    let timescale = rd32(8)?;
    if timescale == 0 {
        return Err(Error::Parse("sidx timescale is zero".into()));
    }
    let (first_offset, mut at) = if version == 0 {
        (rd32(16)?, 20)
    } else {
        (rd64(20)?, 28)
    };
    let count = rd32(at)? & 0xffff;
    at += 4;
    let mut offset = anchor + first_offset;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let w = rd32(at)?;
        let dur = rd32(at + 4)?;
        at += 12;
        if w >> 31 == 1 {
            return Err(Error::Unsupported(
                "hierarchical sidx (index references index)".into(),
            ));
        }
        let size = w & 0x7fff_ffff;
        out.push(IndexedSegment {
            offset,
            size,
            duration: dur as f64 / timescale as f64,
        });
        offset += size;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_sidx(version: u8, first_offset: u64, entries: &[(u32, u32)]) -> Vec<u8> {
        let mut body = vec![version, 0, 0, 0];
        body.extend_from_slice(&1u32.to_be_bytes()); // reference_ID
        body.extend_from_slice(&1000u32.to_be_bytes()); // timescale
        if version == 0 {
            body.extend_from_slice(&0u32.to_be_bytes());
            body.extend_from_slice(&(first_offset as u32).to_be_bytes());
        } else {
            body.extend_from_slice(&0u64.to_be_bytes());
            body.extend_from_slice(&first_offset.to_be_bytes());
        }
        body.extend_from_slice(&0u16.to_be_bytes());
        body.extend_from_slice(&(entries.len() as u16).to_be_bytes());
        for (size, dur) in entries {
            body.extend_from_slice(&size.to_be_bytes());
            body.extend_from_slice(&dur.to_be_bytes());
            body.extend_from_slice(&0x9000_0000u32.to_be_bytes());
        }
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(b"sidx");
        b.extend(body);
        b
    }

    #[test]
    fn parses_v0_and_v1_with_leading_boxes() {
        for version in [0u8, 1] {
            let mut data = vec![0, 0, 0, 16, b's', b't', b'y', b'p', 0, 0, 0, 0, 0, 0, 0, 0];
            let sidx = build_sidx(version, 0, &[(500, 2000), (700, 2000)]);
            data.extend(sidx);
            let index_end = 100 + data.len() as u64;
            let segs = parse_sidx(&data, 100).unwrap();
            assert_eq!(segs.len(), 2);
            assert_eq!(
                segs[0],
                IndexedSegment {
                    offset: index_end,
                    size: 500,
                    duration: 2.0
                }
            );
            assert_eq!(segs[1].offset, index_end + 500);
        }
    }

    #[test]
    fn first_offset_and_missing_box() {
        let data = build_sidx(1, 24, &[(10, 1000)]);
        let segs = parse_sidx(&data, 0).unwrap();
        assert_eq!(segs[0].offset, data.len() as u64 + 24);
        assert!(parse_sidx(b"\0\0\0\x08free", 0).is_err());
    }
}
