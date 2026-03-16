use crate::{DecodeError, EncodeError};

include!(concat!(env!("OUT_DIR"), "/kps9566_tables.rs"));

const REPLACEMENT_CHARACTER: char = '\u{FFFD}';

#[inline]
fn is_kps9566_lead(byte: u8) -> bool {
    (KPS9566_LEAD_MIN..=KPS9566_LEAD_MAX).contains(&byte)
}

#[inline]
fn decode_kps9566_pair(lead: u8, trail: u8) -> Option<char> {
    let ptr = (lead - KPS9566_LEAD_MIN) as usize * KPS9566_TRAIL_COUNT
        + (trail - KPS9566_TRAIL_MIN) as usize;
    let code = KPS9566_DECODE[ptr];
    (code != 0xFFFF)
        .then_some(code as u32)
        .and_then(char::from_u32)
}

#[inline]
fn encode_kps9566_char(ch: char) -> Option<u16> {
    let cp = ch as u32;
    if cp >= 0x80 && cp <= u16::MAX as u32 {
        KPS9566_ENCODE
            .binary_search_by_key(&(cp as u16), |&(u, _)| u)
            .ok()
            .map(|pos| KPS9566_ENCODE[pos].1)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// One-shot convenience functions
// ---------------------------------------------------------------------------

/// Decode KPS 9566 encoded bytes into a Unicode string.
///
/// ASCII bytes (0x00–0x7F) pass through unchanged.
/// All other bytes are treated as the lead byte of a 2-byte KPS 9566 sequence.
pub fn decode(bytes: &[u8]) -> Result<String, DecodeError> {
    let mut out = String::with_capacity(bytes.len());
    Decoder::new().decode_to_string_without_replacement(bytes, &mut out, true)?;
    Ok(out)
}

/// Encode a Unicode string into KPS 9566 bytes.
///
/// ASCII characters (U+0000–U+007F) are written as single bytes.
/// Returns an error if any character has no KPS 9566 mapping.
pub fn encode(s: &str) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::with_capacity(s.len() * 2);
    Encoder.encode_to_vec_without_replacement(s, &mut out)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Streaming Decoder
// ---------------------------------------------------------------------------

/// Stateful streaming decoder for KPS 9566.
///
/// Maintains a pending lead byte across [`decode_to_string`](Decoder::decode_to_string)
/// calls, so input can be fed in arbitrary chunks.
///
/// Mirrors the design of [`encoding_rs::Decoder`](https://docs.rs/encoding_rs/latest/encoding_rs/struct.Decoder.html).
#[derive(Debug, Default, Clone)]
pub struct Decoder {
    /// A lead byte waiting for its trail byte in the next chunk.
    pending_lead: Option<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Discard any pending lead byte and return to the initial state.
    pub fn reset(&mut self) {
        self.pending_lead = None;
    }

    /// Streaming decode with **U+FFFD replacement** on errors.
    ///
    /// Appends decoded text to `dst`. Set `last = true` on the final chunk; a
    /// dangling lead byte at end-of-stream is then treated as an error and
    /// replaced with U+FFFD.
    ///
    /// Returns `(bytes_read, had_errors)`.
    /// `had_errors` is `true` if any replacement character was emitted.
    pub fn decode_to_string(&mut self, src: &[u8], dst: &mut String, last: bool) -> (usize, bool) {
        let mut had_errors = false;
        let mut i = 0;

        while i < src.len() {
            let b = src[i];

            if let Some(lead) = self.pending_lead.take() {
                if b < KPS9566_TRAIL_MIN {
                    // Invalid trail: replace the lead, then re-process b fresh.
                    dst.push(REPLACEMENT_CHARACTER);
                    had_errors = true;
                    // Do NOT increment i — b is reprocessed next iteration.
                } else {
                    match decode_kps9566_pair(lead, b) {
                        Some(ch) => dst.push(ch),
                        None => {
                            dst.push(REPLACEMENT_CHARACTER);
                            had_errors = true;
                        }
                    }
                    i += 1;
                }
            } else if b < 0x80 {
                dst.push(b as char);
                i += 1;
            } else if is_kps9566_lead(b) {
                // Stash the lead byte; trail arrives in this chunk or the next.
                self.pending_lead = Some(b);
                i += 1;
            } else {
                dst.push(REPLACEMENT_CHARACTER);
                had_errors = true;
                i += 1;
            }
        }

        if last && self.pending_lead.take().is_some() {
            dst.push(REPLACEMENT_CHARACTER);
            had_errors = true;
        }

        (i, had_errors)
    }

    /// Streaming decode that **stops at the first error** instead of replacing.
    ///
    /// Returns `Ok(bytes_read)` on success, or `Err` describing the first error.
    /// The decoder state is left consistent so decoding can resume after the
    /// offending position if desired.
    pub fn decode_to_string_without_replacement(
        &mut self,
        src: &[u8],
        dst: &mut String,
        last: bool,
    ) -> Result<usize, DecodeError> {
        let mut i = 0;

        while i < src.len() {
            let b = src[i];

            if let Some(lead) = self.pending_lead.take() {
                if b < KPS9566_TRAIL_MIN {
                    return Err(DecodeError::InvalidByte { offset: i, byte: b });
                }
                match decode_kps9566_pair(lead, b) {
                    Some(ch) => dst.push(ch),
                    None => {
                        // Report the trail byte offset (lead may have been in a prior chunk).
                        return Err(DecodeError::UnmappedBytes { offset: i });
                    }
                }
                i += 1;
            } else if b < 0x80 {
                dst.push(b as char);
                i += 1;
            } else if is_kps9566_lead(b) {
                self.pending_lead = Some(b);
                i += 1;
            } else {
                return Err(DecodeError::InvalidByte { offset: i, byte: b });
            }
        }

        if last && self.pending_lead.take().is_some() {
            return Err(DecodeError::UnexpectedEof { offset: i });
        }

        Ok(i)
    }
}

// ---------------------------------------------------------------------------
// Streaming Encoder
// ---------------------------------------------------------------------------

/// Stateless streaming encoder for KPS 9566.
///
/// KPS 9566 encoding has no inter-character state, so this struct carries no
/// fields — it exists for API symmetry with [`Decoder`] and
/// [`encoding_rs::Encoder`](https://docs.rs/encoding_rs/latest/encoding_rs/struct.Encoder.html).
#[derive(Debug, Default, Clone, Copy)]
pub struct Encoder;

impl Encoder {
    pub fn new() -> Self {
        Self
    }

    /// Encode with **`'?'` (0x3F) substitution** for unmappable characters.
    ///
    /// Returns `(bytes_written, had_errors)`.
    pub fn encode_to_vec(&self, src: &str, dst: &mut Vec<u8>) -> (usize, bool) {
        let mut had_errors = false;
        let start = dst.len();

        for ch in src.chars() {
            let cp = ch as u32;
            if cp < 0x80 {
                dst.push(cp as u8);
            } else {
                match encode_kps9566_char(ch) {
                    Some(kps) => {
                        dst.push((kps >> 8) as u8);
                        dst.push((kps & 0xFF) as u8);
                    }
                    None => {
                        dst.push(b'?');
                        had_errors = true;
                    }
                }
            }
        }

        (dst.len() - start, had_errors)
    }

    /// Encode without replacement; returns `Err` on the first unmappable character.
    pub fn encode_to_vec_without_replacement(
        &self,
        src: &str,
        dst: &mut Vec<u8>,
    ) -> Result<usize, EncodeError> {
        let start = dst.len();

        for (char_idx, ch) in src.chars().enumerate() {
            let cp = ch as u32;
            if cp < 0x80 {
                dst.push(cp as u8);
            } else {
                match encode_kps9566_char(ch) {
                    Some(kps) => {
                        dst.push((kps >> 8) as u8);
                        dst.push((kps & 0xFF) as u8);
                    }
                    None => return Err(EncodeError::UnmappableChar { char_idx, ch }),
                }
            }
        }

        Ok(dst.len() - start)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- exhaustive table correctness ---

    /// Decode every non-FFFF slot in the decode table and verify the result.
    #[test]
    fn exhaustive_decode_table() {
        for lead_idx in 0..126usize {
            for trail_idx in 0..KPS9566_TRAIL_COUNT {
                let ptr = lead_idx * KPS9566_TRAIL_COUNT + trail_idx;
                let code = KPS9566_DECODE[ptr];
                if code == 0xFFFF {
                    continue;
                }
                let lead = KPS9566_LEAD_MIN + lead_idx as u8;
                let trail = KPS9566_TRAIL_MIN + trail_idx as u8;
                let result = decode(&[lead, trail]).unwrap();
                let expected = char::from_u32(code as u32).unwrap();
                assert_eq!(
                    result.chars().next().unwrap(),
                    expected,
                    "decode({lead:#04x},{trail:#04x}) should be U+{code:04X}"
                );
            }
        }
    }

    /// Encode every entry in the encode table and verify the result.
    #[test]
    fn exhaustive_encode_table() {
        for &(unicode, kps) in KPS9566_ENCODE.iter() {
            let ch = char::from_u32(unicode as u32).unwrap();
            let encoded = encode(&ch.to_string()).unwrap();
            let lead = (kps >> 8) as u8;
            let trail = (kps & 0xFF) as u8;
            assert_eq!(
                encoded,
                &[lead, trail],
                "encode(U+{unicode:04X} '{ch}') → [{lead:#04x},{trail:#04x}]"
            );
        }
    }

    // --- one-shot ---

    #[test]
    fn roundtrip_ascii() {
        let s = "Hello, world!";
        assert_eq!(decode(&encode(s).unwrap()).unwrap(), s);
    }

    #[test]
    fn decode_known() {
        // 0x8141 → U+AC03 갃
        assert_eq!(decode(&[0x81, 0x41]).unwrap(), "갃");
    }

    #[test]
    fn encode_known() {
        // U+AC03 갃 → 0x8141
        assert_eq!(encode("갃").unwrap(), [0x81, 0x41]);
    }

    #[test]
    fn decode_unexpected_eof() {
        // offset = 1: lead byte was consumed; EOF detected after it
        assert!(matches!(
            decode(&[0x81]),
            Err(DecodeError::UnexpectedEof { offset: 1 })
        ));
    }

    #[test]
    fn encode_unmappable() {
        assert!(matches!(
            encode("😀"),
            Err(EncodeError::UnmappableChar { .. })
        ));
    }

    // --- streaming Decoder ---

    #[test]
    fn streaming_decode_split_across_chunks() {
        // Feed lead byte and trail byte in separate chunks.
        let mut dec = Decoder::new();
        let mut out = String::new();
        let (n, err) = dec.decode_to_string(&[0x81], &mut out, false);
        assert_eq!(n, 1);
        assert!(!err);
        assert!(out.is_empty()); // lead stashed, nothing emitted yet

        let (n, err) = dec.decode_to_string(&[0x41], &mut out, true);
        assert_eq!(n, 1);
        assert!(!err);
        assert_eq!(out, "갃");
    }

    #[test]
    fn streaming_decode_replacement_on_invalid_trail() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        // Lead 0x81 followed by 0x20 (< TRAIL_MIN): lead replaced, 0x20 kept as ASCII.
        let (_, had_errors) = dec.decode_to_string(&[0x81, 0x20], &mut out, true);
        assert!(had_errors);
        assert_eq!(out, "\u{FFFD} "); // FFFD for bad lead, then ASCII space
    }

    #[test]
    fn streaming_decode_dangling_lead_at_eof() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        let (_, had_errors) = dec.decode_to_string(&[0x81], &mut out, true);
        assert!(had_errors);
        assert_eq!(out, "\u{FFFD}");
    }

    #[test]
    fn streaming_without_replacement_split() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        dec.decode_to_string_without_replacement(&[0x81], &mut out, false)
            .unwrap();
        dec.decode_to_string_without_replacement(&[0x41], &mut out, true)
            .unwrap();
        assert_eq!(out, "갃");
    }

    #[test]
    fn streaming_without_replacement_eof_error() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        dec.decode_to_string_without_replacement(&[0x81], &mut out, false)
            .unwrap();
        assert!(matches!(
            dec.decode_to_string_without_replacement(&[], &mut out, true),
            Err(DecodeError::UnexpectedEof { .. })
        ));
    }

    // --- streaming Encoder ---

    #[test]
    fn streaming_encode_with_replacement() {
        let mut dst = Vec::new();
        let (_, had_errors) = Encoder.encode_to_vec("갃😀", &mut dst);
        assert!(had_errors);
        assert_eq!(&dst[..2], [0x81, 0x41]); // 갃 encoded
        assert_eq!(dst[2], b'?'); // 😀 replaced
    }

    #[test]
    fn streaming_encode_without_replacement_error() {
        let mut dst = Vec::new();
        assert!(matches!(
            Encoder.encode_to_vec_without_replacement("😀", &mut dst),
            Err(EncodeError::UnmappableChar { .. })
        ));
    }
}
