//! Byte-oriented RLE used by the VOT frames.
//!
//! Mirrors `RLE_ANALISE`/`RLE_UNCODE` of `src/rle.cpp` in the original game.
//! A packet is a control byte followed by data:
//! - `0x00..=0x7F`: repeat the next byte `control + 1` times;
//! - `0x80..=0xFF`: copy the next `(control & 0x7F) + 1` bytes verbatim.
//!
//! The original decoder is driven by the *decompressed* length rather than by
//! the input length, so trailing garbage in the packed stream is ignored.

/// Expand `input` into exactly `output.len()` bytes.
///
/// Returns the number of input bytes consumed, or `None` if the stream ends
/// before the output is filled.
pub fn decode(input: &[u8], output: &mut [u8]) -> Option<usize> {
    let mut src = 0;
    let mut dst = 0;
    while dst < output.len() {
        let control = *input.get(src)?;
        src += 1;
        let len = (control & 0x7F) as usize + 1;
        // A run that overflows the output is truncated: the original writes
        // through a raw pointer and relies on the packer never doing this.
        let end = (dst + len).min(output.len());
        if control & 0x80 != 0 {
            let chunk = input.get(src..src + len)?;
            output[dst..end].copy_from_slice(&chunk[..end - dst]);
            src += len;
        } else {
            let value = *input.get(src)?;
            src += 1;
            output[dst..end].fill(value);
        }
        dst = end;
    }
    Some(src)
}

/// Pack `input` the way the original `RLE_ANALISE` does.
///
/// Only used by the tests and the converter - the game itself never writes
/// VOT files - but keeping it here documents the exact packet layout.
pub fn encode(input: &[u8], output: &mut Vec<u8>) {
    let mut i = 0;
    while i < input.len() {
        let value = input[i];
        let mut run = 1;
        while i + run < input.len() && input[i + run] == value && run < 128 {
            run += 1;
        }
        if run > 1 {
            output.push(run as u8 - 1);
            output.push(value);
            i += run;
            continue;
        }
        // Gather the literals up to (but not including) the next repeat.
        let start = i;
        let mut end = i + 1;
        while end < input.len()
            && end - start < 128
            && !(end + 1 < input.len() && input[end] == input[end + 1])
        {
            end += 1;
        }
        output.push(0x80 | (end - start - 1) as u8);
        output.extend_from_slice(&input[start..end]);
        i = end;
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    fn roundtrip(data: &[u8]) {
        let mut packed = Vec::new();
        encode(data, &mut packed);
        let mut unpacked = vec![0u8; data.len()];
        let used = decode(&packed, &mut unpacked).unwrap();
        assert_eq!(unpacked, data);
        assert_eq!(used, packed.len());
    }

    #[test]
    fn runs_and_literals() {
        roundtrip(&[]);
        roundtrip(&[7]);
        roundtrip(&[1, 1, 1, 1, 2, 3, 4, 5, 5]);
        roundtrip(&[0; 300]);
        roundtrip(&(0..=255u8).collect::<Vec<_>>());
    }

    #[test]
    fn long_runs_are_split() {
        // 128 is the longest run a single packet can express.
        let data = vec![0xAB; 400];
        let mut packed = Vec::new();
        encode(&data, &mut packed);
        assert_eq!(packed, [127, 0xAB, 127, 0xAB, 127, 0xAB, 15, 0xAB]);
        let mut unpacked = vec![0u8; data.len()];
        decode(&packed, &mut unpacked).unwrap();
        assert_eq!(unpacked, data);
    }

    #[test]
    fn known_packets() {
        let mut out = [0u8; 5];
        // repeat 'A' 3 times, then two literals
        decode(&[2, b'A', 0x81, b'B', b'C'], &mut out).unwrap();
        assert_eq!(&out, b"AAABC");
    }

    #[test]
    fn truncated_input_is_rejected() {
        let mut out = [0u8; 8];
        assert_eq!(decode(&[2, b'A'], &mut out), None);
        assert_eq!(decode(&[0x83, b'A'], &mut out), None);
    }
}
