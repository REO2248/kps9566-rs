use crate::{DecodeError, EncodeError};

include!(concat!(env!("OUT_DIR"), "/euckp_tables.rs"));

const REPLACEMENT_CHARACTER: char = '\u{FFFD}';

#[inline]
fn is_euckp_2byte_lead(byte: u8) -> bool {
    (EUCKP_L2_MIN..=EUCKP_L2_MAX).contains(&byte)
}

#[inline]
fn is_euckp_ss3_b2(byte: u8) -> bool {
    (EUCKP_SS3_B2_MIN..=EUCKP_SS3_B2_MAX).contains(&byte)
}

#[inline]
fn is_euckp_ss3_b3(byte: u8) -> bool {
    (EUCKP_SS3_B3_MIN..=EUCKP_SS3_B3_MAX).contains(&byte)
}

#[inline]
fn decode_euckp_2byte(lead: u8, trail: u8) -> Option<char> {
    let ptr = (lead - EUCKP_L2_MIN) as usize * EUCKP_T2_COUNT + (trail - EUCKP_T2_MIN) as usize;
    let code = EUCKP_DECODE_2BYTE[ptr];
    (code != 0xFFFF)
        .then_some(code as u32)
        .and_then(char::from_u32)
}

#[inline]
fn decode_euckp_ss3(b2: u8, b3: u8) -> Option<char> {
    let ptr =
        (b2 - EUCKP_SS3_B2_MIN) as usize * EUCKP_SS3_B3_COUNT + (b3 - EUCKP_SS3_B3_MIN) as usize;
    let code = EUCKP_DECODE_SS3[ptr];
    (code != 0xFFFF)
        .then_some(code as u32)
        .and_then(char::from_u32)
}

#[inline]
fn lookup_euckp_encoding(ch: char) -> Option<u32> {
    let cp = ch as u32;
    if cp >= 0x80 && cp <= u16::MAX as u32 {
        EUCKP_ENCODE
            .binary_search_by_key(&(cp as u16), |&(u, _)| u)
            .ok()
            .map(|pos| EUCKP_ENCODE[pos].1)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// One-shot convenience functions
// ---------------------------------------------------------------------------

/// Decode EUC-KP encoded bytes into a Unicode string.
///
/// Byte classes:
/// - 0x00–0x7F : ASCII, pass through.
/// - 0x8F      : SS3 prefix; the following two bytes (0xA1–0xFE each) form a 3-byte sequence.
/// - 0xA1–0xFE : Lead byte of a 2-byte sequence; trail byte must also be 0xA1–0xFE.
/// - Everything else is invalid.
pub fn decode(bytes: &[u8]) -> Result<String, DecodeError> {
    let mut out = String::with_capacity(bytes.len());
    Decoder::new().decode_to_string_without_replacement(bytes, &mut out, true)?;
    Ok(out)
}

/// Encode a Unicode string into EUC-KP bytes.
///
/// ASCII characters (U+0000–U+007F) are written as single bytes.
/// Characters covered by the 2-byte range are preferred over the SS3 3-byte form.
/// Returns an error if any character has no EUC-KP mapping.
pub fn encode(s: &str) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::with_capacity(s.len() * 2);
    Encoder.encode_to_vec_without_replacement(s, &mut out)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Streaming Decoder
// ---------------------------------------------------------------------------

/// Pending decoder state for EUC-KP, which has up to 3-byte sequences (SS3).
///
/// Mirrors the `EucJpPending` pattern in encoding_rs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum Pending {
    #[default]
    None,
    /// Saw 0x8F (SS3 prefix); waiting for b2.
    Ss3,
    /// Saw 0x8F + b2; waiting for b3.
    Ss3Lead(u8),
    /// Saw a 2-byte lead (0xA1–0xFE); waiting for the trail byte.
    Lead(u8),
}

/// Stateful streaming decoder for EUC-KP.
///
/// Handles sequences split across chunk boundaries, including the 3-byte SS3
/// sequences that require two bytes after the 0x8F prefix.
///
/// Mirrors the design of [`encoding_rs::Decoder`](https://docs.rs/encoding_rs/latest/encoding_rs/struct.Decoder.html).
#[derive(Debug, Default, Clone)]
pub struct Decoder {
    pending: Pending,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Discard any pending state and return to the initial state.
    pub fn reset(&mut self) {
        self.pending = Pending::None;
    }

    /// Streaming decode with **U+FFFD replacement** on errors.
    ///
    /// Appends decoded text to `dst`. Set `last = true` on the final chunk.
    ///
    /// Returns `(bytes_read, had_errors)`.
    pub fn decode_to_string(&mut self, src: &[u8], dst: &mut String, last: bool) -> (usize, bool) {
        let mut had_errors = false;
        let mut i = 0;

        while i < src.len() {
            let b = src[i];

            match std::mem::replace(&mut self.pending, Pending::None) {
                Pending::None => {
                    if b < 0x80 {
                        dst.push(b as char);
                        i += 1;
                    } else if b == 0x8F {
                        self.pending = Pending::Ss3;
                        i += 1;
                    } else if is_euckp_2byte_lead(b) {
                        self.pending = Pending::Lead(b);
                        i += 1;
                    } else {
                        dst.push(REPLACEMENT_CHARACTER);
                        had_errors = true;
                        i += 1;
                    }
                }

                Pending::Ss3 => {
                    if !is_euckp_ss3_b2(b) {
                        // Invalid b2: replace the SS3 prefix, re-process b.
                        dst.push(REPLACEMENT_CHARACTER);
                        had_errors = true;
                        // pending already reset to None; do NOT increment i.
                    } else {
                        self.pending = Pending::Ss3Lead(b);
                        i += 1;
                    }
                }

                Pending::Ss3Lead(b2) => {
                    if !is_euckp_ss3_b3(b) {
                        // Invalid b3: replace the partial SS3 sequence, re-process b.
                        dst.push(REPLACEMENT_CHARACTER);
                        had_errors = true;
                        // pending already reset to None; do NOT increment i.
                    } else {
                        match decode_euckp_ss3(b2, b) {
                            Some(ch) => dst.push(ch),
                            None => {
                                dst.push(REPLACEMENT_CHARACTER);
                                had_errors = true;
                            }
                        }
                        i += 1;
                    }
                }

                Pending::Lead(lead) => {
                    if b < EUCKP_T2_MIN {
                        // Invalid trail: replace the lead, re-process b.
                        dst.push(REPLACEMENT_CHARACTER);
                        had_errors = true;
                        // pending already reset to None; do NOT increment i.
                    } else {
                        match decode_euckp_2byte(lead, b) {
                            Some(ch) => dst.push(ch),
                            None => {
                                dst.push(REPLACEMENT_CHARACTER);
                                had_errors = true;
                            }
                        }
                        i += 1;
                    }
                }
            }
        }

        if last && self.pending != Pending::None {
            self.pending = Pending::None;
            dst.push(REPLACEMENT_CHARACTER);
            had_errors = true;
        }

        (i, had_errors)
    }

    /// Streaming decode that **stops at the first error** instead of replacing.
    ///
    /// Returns `Ok(bytes_read)` on success, or `Err` describing the first error.
    pub fn decode_to_string_without_replacement(
        &mut self,
        src: &[u8],
        dst: &mut String,
        last: bool,
    ) -> Result<usize, DecodeError> {
        let mut i = 0;

        while i < src.len() {
            let b = src[i];

            match std::mem::replace(&mut self.pending, Pending::None) {
                Pending::None => {
                    if b < 0x80 {
                        dst.push(b as char);
                        i += 1;
                    } else if b == 0x8F {
                        self.pending = Pending::Ss3;
                        i += 1;
                    } else if is_euckp_2byte_lead(b) {
                        self.pending = Pending::Lead(b);
                        i += 1;
                    } else {
                        return Err(DecodeError::InvalidByte { offset: i, byte: b });
                    }
                }

                Pending::Ss3 => {
                    if !is_euckp_ss3_b2(b) {
                        return Err(DecodeError::InvalidByte { offset: i, byte: b });
                    }
                    self.pending = Pending::Ss3Lead(b);
                    i += 1;
                }

                Pending::Ss3Lead(b2) => {
                    if !is_euckp_ss3_b3(b) {
                        return Err(DecodeError::InvalidByte { offset: i, byte: b });
                    }
                    match decode_euckp_ss3(b2, b) {
                        Some(ch) => dst.push(ch),
                        None => return Err(DecodeError::UnmappedBytes { offset: i }),
                    }
                    i += 1;
                }

                Pending::Lead(lead) => {
                    if b < EUCKP_T2_MIN {
                        return Err(DecodeError::InvalidByte { offset: i, byte: b });
                    }
                    match decode_euckp_2byte(lead, b) {
                        Some(ch) => dst.push(ch),
                        None => return Err(DecodeError::UnmappedBytes { offset: i }),
                    }
                    i += 1;
                }
            }
        }

        if last && self.pending != Pending::None {
            self.pending = Pending::None;
            return Err(DecodeError::UnexpectedEof { offset: i });
        }

        Ok(i)
    }
}

// ---------------------------------------------------------------------------
// Streaming Encoder
// ---------------------------------------------------------------------------

/// Stateless streaming encoder for EUC-KP.
///
/// EUC-KP encoding has no inter-character state, so this struct carries no
/// fields — it exists for API symmetry with [`Decoder`].
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
                match lookup_euckp_encoding(ch) {
                    Some(enc) => write_euckp_encoded(enc, dst),
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
                match lookup_euckp_encoding(ch) {
                    Some(enc) => write_euckp_encoded(enc, dst),
                    None => return Err(EncodeError::UnmappableChar { char_idx, ch }),
                }
            }
        }

        Ok(dst.len() - start)
    }
}

/// Write an encoded EUC-KP value (from the encode table) to `dst`.
/// `enc <= 0xFFFF` → 2-byte; `enc > 0xFFFF` → SS3 3-byte (0x8F prefix).
#[inline]
fn write_euckp_encoded(enc: u32, dst: &mut Vec<u8>) {
    if enc > 0xFFFF {
        dst.push(0x8F);
        dst.push(((enc >> 8) & 0xFF) as u8);
        dst.push((enc & 0xFF) as u8);
    } else {
        dst.push((enc >> 8) as u8);
        dst.push((enc & 0xFF) as u8);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- exhaustive table correctness ---

    /// Decode every non-FFFF slot in both decode tables and verify the result.
    #[test]
    fn exhaustive_decode_table() {
        let ss3_b2_count = (EUCKP_SS3_B2_MAX - EUCKP_SS3_B2_MIN + 1) as usize;

        // 2-byte table
        for lead_idx in 0..94usize {
            for trail_idx in 0..EUCKP_T2_COUNT {
                let ptr = lead_idx * EUCKP_T2_COUNT + trail_idx;
                let code = EUCKP_DECODE_2BYTE[ptr];
                if code == 0xFFFF {
                    continue;
                }
                let lead = EUCKP_L2_MIN + lead_idx as u8;
                let trail = EUCKP_T2_MIN + trail_idx as u8;
                let result = decode(&[lead, trail]).unwrap();
                let expected = char::from_u32(code as u32).unwrap();
                assert_eq!(
                    result.chars().next().unwrap(),
                    expected,
                    "2byte decode({lead:#04x},{trail:#04x}) should be U+{code:04X}"
                );
            }
        }
        // SS3 table
        for b2_idx in 0..ss3_b2_count {
            for b3_idx in 0..EUCKP_SS3_B3_COUNT {
                let ptr = b2_idx * EUCKP_SS3_B3_COUNT + b3_idx;
                let code = EUCKP_DECODE_SS3[ptr];
                if code == 0xFFFF {
                    continue;
                }
                let b2 = EUCKP_SS3_B2_MIN + b2_idx as u8;
                let b3 = EUCKP_SS3_B3_MIN + b3_idx as u8;
                let result = decode(&[0x8F, b2, b3]).unwrap();
                let expected = char::from_u32(code as u32).unwrap();
                assert_eq!(
                    result.chars().next().unwrap(),
                    expected,
                    "SS3 decode(8F,{b2:#04x},{b3:#04x}) should be U+{code:04X}"
                );
            }
        }
    }

    /// Encode every entry in the encode table and verify the result.
    #[test]
    fn exhaustive_encode_table() {
        for &(unicode, enc) in EUCKP_ENCODE.iter() {
            let ch = char::from_u32(unicode as u32).unwrap();
            let encoded = encode(&ch.to_string()).unwrap();
            if enc > 0xFFFF {
                // SS3 form
                let b2 = ((enc >> 8) & 0xFF) as u8;
                let b3 = (enc & 0xFF) as u8;
                assert_eq!(
                    encoded,
                    &[0x8F, b2, b3],
                    "encode(U+{unicode:04X}) should give SS3 [8F,{b2:#04x},{b3:#04x}]"
                );
            } else {
                let b1 = (enc >> 8) as u8;
                let b2 = (enc & 0xFF) as u8;
                assert_eq!(
                    encoded,
                    &[b1, b2],
                    "encode(U+{unicode:04X}) should give 2-byte [{b1:#04x},{b2:#04x}]"
                );
            }
        }
    }

    /// Characters in the 2-byte table must be encoded with 2 bytes, not SS3.
    #[test]
    fn two_byte_preferred_over_ss3() {
        // U+3000 is in the 2-byte table (0xA1A1); encode must NOT produce 0x8F prefix.
        let encoded = encode("\u{3000}").unwrap();
        assert_eq!(encoded.len(), 2);
        assert_ne!(encoded[0], 0x8F);
    }

    // --- one-shot ---

    #[test]
    fn roundtrip_ascii() {
        let s = "Hello, world!";
        assert_eq!(decode(&encode(s).unwrap()).unwrap(), s);
    }

    #[test]
    fn decode_2byte_known() {
        // 0xA1A1 → U+3000 (ideographic space)
        assert_eq!(decode(&[0xA1, 0xA1]).unwrap(), "\u{3000}");
    }

    #[test]
    fn encode_2byte_known() {
        // U+3000 → 0xA1A1
        assert_eq!(encode("\u{3000}").unwrap(), [0xA1, 0xA1]);
    }

    #[test]
    fn decode_ss3_known() {
        // 0x8F A1 A1 → U+AC03 갃
        assert_eq!(decode(&[0x8F, 0xA1, 0xA1]).unwrap(), "갃");
    }

    #[test]
    fn encode_ss3_roundtrip() {
        let original = "갃";
        let encoded = encode(original).unwrap();
        assert_eq!(decode(&encoded).unwrap(), original);
    }

    #[test]
    fn decode_unexpected_eof_2byte() {
        // offset = 1: lead byte was consumed; EOF detected after it
        assert!(matches!(
            decode(&[0xA1]),
            Err(DecodeError::UnexpectedEof { offset: 1 })
        ));
    }

    #[test]
    fn decode_unexpected_eof_ss3() {
        // offset = 2: 0x8F and b2 were consumed; EOF detected after them
        assert!(matches!(
            decode(&[0x8F, 0xA1]),
            Err(DecodeError::UnexpectedEof { offset: 2 })
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
    fn streaming_decode_ss3_split_three_ways() {
        // Feed 0x8F, then b2, then b3 in separate chunks.
        let mut dec = Decoder::new();
        let mut out = String::new();

        let (n, err) = dec.decode_to_string(&[0x8F], &mut out, false);
        assert_eq!(n, 1);
        assert!(!err);
        assert!(out.is_empty());

        let (n, err) = dec.decode_to_string(&[0xA1], &mut out, false);
        assert_eq!(n, 1);
        assert!(!err);
        assert!(out.is_empty());

        let (n, err) = dec.decode_to_string(&[0xA1], &mut out, true);
        assert_eq!(n, 1);
        assert!(!err);
        assert_eq!(out, "갃");
    }

    #[test]
    fn streaming_decode_2byte_split() {
        let mut dec = Decoder::new();
        let mut out = String::new();

        dec.decode_to_string(&[0xA1], &mut out, false);
        dec.decode_to_string(&[0xA1], &mut out, true);
        assert_eq!(out, "\u{3000}");
    }

    #[test]
    fn streaming_decode_invalid_ss3_b2_replaced() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        // 0x8F followed by 0x20 (< SS3_B2_MIN): replace SS3 prefix, re-process 0x20 as ASCII.
        let (_, had_errors) = dec.decode_to_string(&[0x8F, 0x20], &mut out, true);
        assert!(had_errors);
        assert_eq!(out, "\u{FFFD} ");
    }

    #[test]
    fn streaming_decode_dangling_ss3_at_eof() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        let (_, had_errors) = dec.decode_to_string(&[0x8F], &mut out, true);
        assert!(had_errors);
        assert_eq!(out, "\u{FFFD}");
    }

    #[test]
    fn streaming_without_replacement_ss3_split() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        dec.decode_to_string_without_replacement(&[0x8F, 0xA1], &mut out, false)
            .unwrap();
        dec.decode_to_string_without_replacement(&[0xA1], &mut out, true)
            .unwrap();
        assert_eq!(out, "갃");
    }

    #[test]
    fn streaming_without_replacement_eof_error() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        dec.decode_to_string_without_replacement(&[0x8F], &mut out, false)
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
        let (_, had_errors) = Encoder.encode_to_vec("\u{3000}😀", &mut dst);
        assert!(had_errors);
        assert_eq!(&dst[..2], [0xA1, 0xA1]); // U+3000 encoded as 2-byte
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
