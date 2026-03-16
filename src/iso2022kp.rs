use crate::{DecodeError, EncodeError};

// Reuse the EUC-KP 2-byte decode/encode tables.
// ISO-2022-KP 2-byte characters are the 7-bit GL (0x21–0x7E) forms of EUC-KP 2-byte
// characters: iso_byte = euc_byte & 0x7F (≡ euc_byte − 0x80).
include!(concat!(env!("OUT_DIR"), "/euckp_tables.rs"));

const ESC: u8 = 0x1B;
const SO: u8 = 0x0E; // Shift-Out:  invoke G1 (KPS 9566)
const SI: u8 = 0x0F; // Shift-In:   invoke G0 (ASCII)
const GL_MIN: u8 = 0x21;
const GL_MAX: u8 = 0x7E;

// The G1 designation sequence: ESC $ ) N  (0x1B 0x24 0x29 0x4E)
const DESIG_2: u8 = 0x24; // '$'
const DESIG_3: u8 = 0x29; // ')'
const DESIG_4: u8 = 0x4E; // 'N'
const REPLACEMENT_CHARACTER: char = '\u{FFFD}';

#[inline]
fn is_gl(byte: u8) -> bool {
    (GL_MIN..=GL_MAX).contains(&byte)
}

#[inline]
fn decode_gl_pair(gl1: u8, gl2: u8) -> Option<char> {
    let euc_lead = gl1 | 0x80; // 0xA1–0xFE
    let euc_trail = gl2 | 0x80; // 0xA1–0xFE
    let ptr =
        (euc_lead - EUCKP_L2_MIN) as usize * EUCKP_T2_COUNT + (euc_trail - EUCKP_T2_MIN) as usize;
    let code = EUCKP_DECODE_2BYTE[ptr];
    (code != 0xFFFF)
        .then_some(code as u32)
        .and_then(char::from_u32)
}

// ---------------------------------------------------------------------------
// One-shot convenience functions
// ---------------------------------------------------------------------------

/// Decode ISO-2022-KP encoded bytes into a Unicode string.
///
/// Expects well-formed ISO-2022-KP: the stream must contain `ESC $ ) N` before
/// any SO-invoked KPS 9566 characters.
pub fn decode(bytes: &[u8]) -> Result<String, DecodeError> {
    let mut out = String::with_capacity(bytes.len());
    Decoder::new().decode_to_string_without_replacement(bytes, &mut out, true)?;
    Ok(out)
}

/// Encode a Unicode string into ISO-2022-KP bytes.
///
/// Emits `ESC $ ) N` once before the first KPS 9566 character; uses SO/SI to
/// bracket KPS 9566 runs. The stream always ends in ASCII (G0) state.
pub fn encode(s: &str) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::with_capacity(s.len() * 2);
    let mut enc = Encoder::new();
    enc.encode_to_vec_without_replacement(s, &mut out)?;
    enc.finish(&mut out);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Streaming Decoder
// ---------------------------------------------------------------------------

/// Pending state for the ISO-2022-KP decoder.
///
/// The ESC-sequence states track partial consumption of the 4-byte designation
/// `ESC $ ) N`. The `G1Lead` state buffers the first byte of a G1 2-byte pair.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Pending {
    #[default]
    None,
    /// Saw ESC (0x1B); waiting for 0x24 (`$`).
    Esc,
    /// Saw ESC 0x24; waiting for 0x29 (`)`).
    EscDollar,
    /// Saw ESC 0x24 0x29; waiting for 0x4E (`N`).
    EscDollarParen,
    /// In G1 mode; buffered first GL byte (0x21–0x7E), waiting for second.
    G1Lead(u8),
}

/// Stateful streaming decoder for ISO-2022-KP.
///
/// ISO-2022-KP is a 7-bit modal encoding:
/// - `ESC $ ) N` designates KPS 9566 as the G1 character set (must appear first).
/// - SO (0x0E) invokes G1; SI (0x0F) returns to G0 (ASCII).
/// - In G1 mode, each character is two bytes in the GL range (0x21–0x7E); adding
///   0x80 to each byte yields the corresponding EUC-KP 2-byte sequence.
///
/// Mirrors the design of [`encoding_rs::Decoder`](https://docs.rs/encoding_rs/latest/encoding_rs/struct.Decoder.html).
#[derive(Debug, Default, Clone)]
pub struct Decoder {
    pending: Pending,
    g1_designated: bool,
    in_g1: bool,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the decoder to its initial state (G0, no designation).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Streaming decode with **U+FFFD replacement** on errors.
    ///
    /// Returns `(bytes_read, had_errors)`. Set `last = true` on the final chunk.
    pub fn decode_to_string(&mut self, src: &[u8], dst: &mut String, last: bool) -> (usize, bool) {
        let mut had_errors = false;
        let mut i = 0;

        while i < src.len() {
            let b = src[i];

            match std::mem::replace(&mut self.pending, Pending::None) {
                Pending::None => {
                    if self.in_g1 {
                        if is_gl(b) {
                            self.pending = Pending::G1Lead(b);
                            i += 1;
                        } else if b == SI {
                            self.in_g1 = false;
                            i += 1;
                        } else if b == SO {
                            // Already in G1; no-op.
                            i += 1;
                        } else if b == ESC {
                            self.pending = Pending::Esc;
                            i += 1;
                        } else {
                            // Invalid byte in G1 mode.
                            dst.push(REPLACEMENT_CHARACTER);
                            had_errors = true;
                            i += 1;
                        }
                    } else {
                        // G0 (ASCII) mode.
                        if b == ESC {
                            self.pending = Pending::Esc;
                            i += 1;
                        } else if b == SO {
                            if self.g1_designated {
                                self.in_g1 = true;
                            } else {
                                // SO before designation.
                                dst.push(REPLACEMENT_CHARACTER);
                                had_errors = true;
                            }
                            i += 1;
                        } else if b == SI {
                            // Already in G0; no-op.
                            i += 1;
                        } else if b < 0x80 {
                            dst.push(b as char);
                            i += 1;
                        } else {
                            dst.push(REPLACEMENT_CHARACTER);
                            had_errors = true;
                            i += 1;
                        }
                    }
                }

                Pending::Esc => {
                    if b == DESIG_2 {
                        self.pending = Pending::EscDollar;
                        i += 1;
                    } else {
                        // Unrecognised ESC sequence: replace ESC, re-process b.
                        dst.push(REPLACEMENT_CHARACTER);
                        had_errors = true;
                        // Do NOT increment i.
                    }
                }

                Pending::EscDollar => {
                    if b == DESIG_3 {
                        self.pending = Pending::EscDollarParen;
                        i += 1;
                    } else {
                        // ESC was the bad byte; '$' (0x24) is a valid ASCII/G1 character
                        // that must be re-emitted, then re-process b.
                        dst.push(REPLACEMENT_CHARACTER); // for the lone ESC
                        had_errors = true;
                        dst.push(DESIG_2 as char); // '$' was valid, put it back
                                                   // Do NOT increment i; re-process b.
                    }
                }

                Pending::EscDollarParen => {
                    if b == DESIG_4 {
                        // Complete: ESC $ ) N — KPS 9566 is now designated as G1.
                        self.g1_designated = true;
                        // Shift state unchanged.
                        i += 1;
                    } else {
                        // ESC was the bad byte; '$' and ')' were valid characters.
                        dst.push(REPLACEMENT_CHARACTER); // for the lone ESC
                        had_errors = true;
                        dst.push(DESIG_2 as char); // '$'
                        dst.push(DESIG_3 as char); // ')'
                                                   // Do NOT increment i; re-process b.
                    }
                }

                Pending::G1Lead(lead) => {
                    if !is_gl(b) {
                        // Invalid second byte: replace the partial pair, re-process b.
                        dst.push(REPLACEMENT_CHARACTER);
                        had_errors = true;
                        // Do NOT increment i.
                    } else {
                        match decode_gl_pair(lead, b) {
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
    /// Returns `Ok(bytes_read)` or `Err` describing the first error.
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
                    if self.in_g1 {
                        if is_gl(b) {
                            self.pending = Pending::G1Lead(b);
                            i += 1;
                        } else if b == SI {
                            self.in_g1 = false;
                            i += 1;
                        } else if b == SO {
                            i += 1; // no-op, already in G1
                        } else if b == ESC {
                            self.pending = Pending::Esc;
                            i += 1;
                        } else {
                            return Err(DecodeError::InvalidByte { offset: i, byte: b });
                        }
                    } else if b == ESC {
                        self.pending = Pending::Esc;
                        i += 1;
                    } else if b == SO {
                        if self.g1_designated {
                            self.in_g1 = true;
                            i += 1;
                        } else {
                            return Err(DecodeError::InvalidByte { offset: i, byte: b });
                        }
                    } else if b == SI {
                        i += 1; // no-op, already in G0
                    } else if b < 0x80 {
                        dst.push(b as char);
                        i += 1;
                    } else {
                        return Err(DecodeError::InvalidByte { offset: i, byte: b });
                    }
                }

                Pending::Esc => {
                    if b == DESIG_2 {
                        self.pending = Pending::EscDollar;
                        i += 1;
                    } else {
                        return Err(DecodeError::InvalidByte { offset: i, byte: b });
                    }
                }

                Pending::EscDollar => {
                    if b == DESIG_3 {
                        self.pending = Pending::EscDollarParen;
                        i += 1;
                    } else {
                        return Err(DecodeError::InvalidByte { offset: i, byte: b });
                    }
                }

                Pending::EscDollarParen => {
                    if b == DESIG_4 {
                        self.g1_designated = true;
                        i += 1;
                    } else {
                        return Err(DecodeError::InvalidByte { offset: i, byte: b });
                    }
                }

                Pending::G1Lead(lead) => {
                    if !is_gl(b) {
                        return Err(DecodeError::InvalidByte { offset: i, byte: b });
                    }
                    match decode_gl_pair(lead, b) {
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

/// Stateful streaming encoder for ISO-2022-KP.
///
/// Tracks whether the G1 designation (`ESC $ ) N`) has been emitted and the
/// current shift state (G0/G1) to insert SO/SI at the right places.
///
/// Unlike the stateless EUC-KP [`Encoder`](crate::euckp::Encoder), this struct
/// carries state because ISO-2022-KP has inter-character shift state.
/// Call [`finish`](Encoder::finish) after the last chunk to ensure the stream
/// ends in G0 (ASCII) state.
#[derive(Debug, Default, Clone)]
pub struct Encoder {
    g1_designated: bool,
    in_g1: bool,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the encoder to its initial state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Encode with **`'?'` (0x3F) substitution** for unmappable characters.
    ///
    /// Returns `(bytes_written, had_errors)`.
    pub fn encode_to_vec(&mut self, src: &str, dst: &mut Vec<u8>) -> (usize, bool) {
        let mut had_errors = false;
        let start = dst.len();

        for ch in src.chars() {
            let cp = ch as u32;
            if cp < 0x80 {
                if self.in_g1 {
                    dst.push(SI);
                    self.in_g1 = false;
                }
                dst.push(cp as u8);
            } else {
                match self.find_gl_pair(ch) {
                    Some((gl1, gl2)) => {
                        if !self.g1_designated {
                            dst.extend_from_slice(&[ESC, DESIG_2, DESIG_3, DESIG_4]);
                            self.g1_designated = true;
                        }
                        if !self.in_g1 {
                            dst.push(SO);
                            self.in_g1 = true;
                        }
                        dst.push(gl1);
                        dst.push(gl2);
                    }
                    None => {
                        if self.in_g1 {
                            dst.push(SI);
                            self.in_g1 = false;
                        }
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
        &mut self,
        src: &str,
        dst: &mut Vec<u8>,
    ) -> Result<usize, EncodeError> {
        let start = dst.len();

        for (char_idx, ch) in src.chars().enumerate() {
            let cp = ch as u32;
            if cp < 0x80 {
                if self.in_g1 {
                    dst.push(SI);
                    self.in_g1 = false;
                }
                dst.push(cp as u8);
            } else {
                match self.find_gl_pair(ch) {
                    Some((gl1, gl2)) => {
                        if !self.g1_designated {
                            dst.extend_from_slice(&[ESC, DESIG_2, DESIG_3, DESIG_4]);
                            self.g1_designated = true;
                        }
                        if !self.in_g1 {
                            dst.push(SO);
                            self.in_g1 = true;
                        }
                        dst.push(gl1);
                        dst.push(gl2);
                    }
                    None => return Err(EncodeError::UnmappableChar { char_idx, ch }),
                }
            }
        }

        Ok(dst.len() - start)
    }

    /// Finalize the stream: if currently in G1, emits SI (0x0F) to return to ASCII.
    ///
    /// Returns the number of bytes written (0 or 1).
    /// Call this after the last [`encode_to_vec`](Encoder::encode_to_vec) /
    /// [`encode_to_vec_without_replacement`](Encoder::encode_to_vec_without_replacement) call.
    pub fn finish(&mut self, dst: &mut Vec<u8>) -> usize {
        if self.in_g1 {
            dst.push(SI);
            self.in_g1 = false;
            1
        } else {
            0
        }
    }

    /// Look up a Unicode codepoint and return its ISO-2022-KP GL byte pair
    /// `(gl1, gl2)` (both in 0x21–0x7E), or `None` if unmappable.
    ///
    /// Unmappable cases:
    /// - No EUC-KP mapping at all.
    /// - EUC-KP mapping is SS3-only (would require 0x8F prefix, outside GL).
    /// - EUC-KP trail byte is 0xFF (GL form would be 0x7F = DEL, not a graphic).
    fn find_gl_pair(&self, ch: char) -> Option<(u8, u8)> {
        let cp = ch as u32;
        if cp > u16::MAX as u32 {
            return None;
        }
        let cp = cp as u16;
        let pos = EUCKP_ENCODE.binary_search_by_key(&cp, |&(u, _)| u).ok()?;
        let enc = EUCKP_ENCODE[pos].1;
        if enc > 0xFFFF {
            return None; // SS3-only character
        }
        let euc_lead = (enc >> 8) as u8; // 0xA1–0xFE always (from table construction)
        let euc_trail = (enc & 0xFF) as u8;
        if euc_trail > 0xFE {
            return None; // trail 0xFF → GL 0x7F = DEL, invalid
        }
        // euc_lead 0xA1–0xFE → GL 0x21–0x7E ✓
        // euc_trail 0xA1–0xFE → GL 0x21–0x7E ✓
        Some((euc_lead & 0x7F, euc_trail & 0x7F))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Convenience: wrap bytes into a full ISO-2022-KP stream and decode.
    fn decode_g1(gl_bytes: &[u8]) -> Result<String, DecodeError> {
        let mut stream = vec![ESC, DESIG_2, DESIG_3, DESIG_4, SO];
        stream.extend_from_slice(gl_bytes);
        stream.push(SI);
        decode(&stream)
    }

    // --- exhaustive table correctness ---

    /// Every non-FFFF slot in the EUC-KP 2-byte table whose trail ≤ 0xFE is
    /// reachable from ISO-2022-KP via its GL form. Verify round-trip for all.
    #[test]
    fn exhaustive_decode_table() {
        for lead_idx in 0..94usize {
            // Only trail indices that map to bytes ≤ 0xFE (GL valid)
            for trail_idx in 0..94usize {
                let ptr = lead_idx * EUCKP_T2_COUNT + trail_idx;
                let code = EUCKP_DECODE_2BYTE[ptr];
                if code == 0xFFFF {
                    continue;
                }
                let gl1 = (EUCKP_L2_MIN + lead_idx as u8) & 0x7F;
                let gl2 = (EUCKP_T2_MIN + trail_idx as u8) & 0x7F;
                let result = decode_g1(&[gl1, gl2]).unwrap();
                let expected = char::from_u32(code as u32).unwrap();
                assert_eq!(
                    result.chars().next().unwrap(),
                    expected,
                    "GL({gl1:#04x},{gl2:#04x}) should decode to U+{code:04X}"
                );
            }
        }
    }

    /// Every entry in the EUC-KP encode table that is 2-byte and has trail ≤ 0xFE
    /// should encode successfully and round-trip.
    #[test]
    fn exhaustive_encode_table() {
        for &(unicode, enc) in EUCKP_ENCODE.iter() {
            if enc > 0xFFFF {
                continue; // SS3-only: not encodable in ISO-2022-KP
            }
            let trail = (enc & 0xFF) as u8;
            if trail > 0xFE {
                continue; // GL 0x7F = DEL: not valid
            }
            let ch = char::from_u32(unicode as u32).unwrap();
            let encoded = encode(&ch.to_string()).unwrap();
            // Stream must contain ESC $ ) N, SO, gl1, gl2, SI (8 bytes total for one char)
            assert_eq!(encoded.len(), 8, "U+{unicode:04X}: expected 8-byte stream");
            assert_eq!(&encoded[..5], &[ESC, DESIG_2, DESIG_3, DESIG_4, SO]);
            let gl1 = (enc >> 8) as u8 & 0x7F;
            let gl2 = (enc & 0xFF) as u8 & 0x7F;
            assert_eq!(encoded[5], gl1, "U+{unicode:04X}: gl1 mismatch");
            assert_eq!(encoded[6], gl2, "U+{unicode:04X}: gl2 mismatch");
            assert_eq!(encoded[7], SI, "U+{unicode:04X}: missing SI");
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
        // U+3000 (ideographic space): EUC-KP 0xA1A1 → GL 0x21 0x21
        assert_eq!(decode_g1(&[0x21, 0x21]).unwrap(), "\u{3000}");
    }

    #[test]
    fn encode_known() {
        // U+3000 → ESC $ ) N SO 0x21 0x21 SI
        assert_eq!(
            encode("\u{3000}").unwrap(),
            [ESC, DESIG_2, DESIG_3, DESIG_4, SO, 0x21, 0x21, SI]
        );
    }

    #[test]
    fn encode_ascii_only_no_designation() {
        // Pure ASCII: no ESC $ ) N or SO/SI should be emitted.
        let encoded = encode("abc").unwrap();
        assert_eq!(encoded, b"abc");
    }

    #[test]
    fn encode_mixed_ascii_and_kps() {
        // "A" + U+3000 + "B" → A, ESC $ ) N, SO, 0x21, 0x21, SI, B
        let encoded = encode("A\u{3000}B").unwrap();
        assert_eq!(
            encoded,
            [b'A', ESC, DESIG_2, DESIG_3, DESIG_4, SO, 0x21, 0x21, SI, b'B']
        );
    }

    #[test]
    fn encode_consecutive_kps_single_so() {
        // Two consecutive KPS chars should share one SO/SI pair.
        let encoded = encode("\u{3000}\u{3000}").unwrap();
        // ESC $ ) N, SO, 0x21, 0x21, 0x21, 0x21, SI
        assert_eq!(
            encoded,
            [ESC, DESIG_2, DESIG_3, DESIG_4, SO, 0x21, 0x21, 0x21, 0x21, SI]
        );
    }

    #[test]
    fn decode_unexpected_eof_in_g1_pair() {
        // Dangling first GL byte at EOF.
        let stream = [ESC, DESIG_2, DESIG_3, DESIG_4, SO, 0x21];
        assert!(matches!(
            decode(&stream),
            Err(DecodeError::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn decode_unexpected_eof_in_esc_sequence() {
        // Partial ESC sequence at EOF.
        assert!(matches!(
            decode(&[ESC]),
            Err(DecodeError::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn decode_so_before_designation_error() {
        // SO without prior ESC $ ) N is an error.
        assert!(matches!(
            decode(&[SO]),
            Err(DecodeError::InvalidByte { byte: 0x0E, .. })
        ));
    }

    #[test]
    fn encode_ss3_only_char_unmappable() {
        // 갃 (U+AC03) is SS3-only in EUC-KP; must fail in ISO-2022-KP.
        assert!(matches!(
            encode("갃"),
            Err(EncodeError::UnmappableChar { .. })
        ));
    }

    #[test]
    fn encode_unmappable_emoji() {
        assert!(matches!(
            encode("😀"),
            Err(EncodeError::UnmappableChar { .. })
        ));
    }

    // --- streaming Decoder ---

    #[test]
    fn streaming_decode_designation_split() {
        // Feed ESC $ ) N one byte at a time.
        let mut dec = Decoder::new();
        let mut out = String::new();

        for &b in &[ESC, DESIG_2, DESIG_3, DESIG_4] {
            let (n, err) = dec.decode_to_string(&[b], &mut out, false);
            assert_eq!(n, 1);
            assert!(!err);
        }
        assert!(dec.g1_designated);
        assert!(out.is_empty());

        // Now SO + two GL bytes split across chunks.
        dec.decode_to_string(&[SO, 0x21], &mut out, false);
        assert!(out.is_empty()); // GL lead stashed
        dec.decode_to_string(&[0x21, SI], &mut out, true);
        assert_eq!(out, "\u{3000}");
    }

    #[test]
    fn streaming_decode_g1_pair_split() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        // Pre-designate and shift via the first chunk.
        dec.decode_to_string(&[ESC, DESIG_2, DESIG_3, DESIG_4, SO, 0x21], &mut out, false);
        assert!(out.is_empty()); // waiting for second GL byte
        let (n, err) = dec.decode_to_string(&[0x21], &mut out, true);
        assert_eq!(n, 1);
        assert!(!err);
        assert_eq!(out, "\u{3000}");
    }

    #[test]
    fn streaming_decode_invalid_g1_second_byte_replaced() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        // 0x21 (valid GL first) followed by 0x20 (< GL_MIN): replace pair, re-process 0x20.
        // 0x20 is a space (ASCII) but we're in G1 mode — it's < GL_MIN so also invalid in G1.
        let (_, had_errors) = dec.decode_to_string(
            &[ESC, DESIG_2, DESIG_3, DESIG_4, SO, 0x21, 0x20],
            &mut out,
            true,
        );
        assert!(had_errors);
        // U+FFFD for the bad pair, then 0x20 is re-processed in G1 mode (invalid → FFFD again)
        assert!(out.starts_with('\u{FFFD}'));
    }

    #[test]
    fn streaming_without_replacement_eof_in_esc() {
        let mut dec = Decoder::new();
        let mut out = String::new();
        dec.decode_to_string_without_replacement(&[ESC], &mut out, false)
            .unwrap();
        assert!(matches!(
            dec.decode_to_string_without_replacement(&[], &mut out, true),
            Err(DecodeError::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn streaming_without_replacement_full_roundtrip() {
        let original = "\u{3000}";
        let encoded = encode(original).unwrap();
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    // --- streaming Encoder ---

    #[test]
    fn streaming_encoder_finish_emits_si() {
        let mut dst = Vec::new();
        let mut enc = Encoder::new();
        enc.encode_to_vec("\u{3000}", &mut dst);
        // Before finish: stream ends in G1 (no SI yet)
        assert_ne!(*dst.last().unwrap(), SI);
        let written = enc.finish(&mut dst);
        assert_eq!(written, 1);
        assert_eq!(*dst.last().unwrap(), SI);
        // Second finish: already in G0, nothing to do.
        assert_eq!(enc.finish(&mut dst), 0);
    }

    #[test]
    fn streaming_encoder_with_replacement() {
        let mut dst = Vec::new();
        let mut enc = Encoder::new();
        let (_, had_errors) = enc.encode_to_vec("\u{3000}😀", &mut dst);
        enc.finish(&mut dst);
        assert!(had_errors);
        // Check: ESC $ ) N, SO, 0x21, 0x21, SI (returned to G0 before '?'), '?'
        assert!(dst.contains(&b'?'));
    }

    #[test]
    fn streaming_encoder_without_replacement_error() {
        let mut dst = Vec::new();
        let mut enc = Encoder::new();
        assert!(matches!(
            enc.encode_to_vec_without_replacement("😀", &mut dst),
            Err(EncodeError::UnmappableChar { .. })
        ));
    }

    // --- intermediate-byte recovery in replacement mode ---

    #[test]
    fn replacement_partial_esc_dollar_invalid() {
        // ESC $ X (X ≠ ')') → FFFD for lone ESC, then '$' re-emitted, then X as ASCII.
        let mut dec = Decoder::new();
        let mut out = String::new();
        let (_, had_errors) = dec.decode_to_string(&[ESC, DESIG_2, b'X'], &mut out, true);
        assert!(had_errors);
        assert_eq!(out, "\u{FFFD}$X");
    }

    #[test]
    fn replacement_partial_esc_dollar_paren_invalid() {
        // ESC $ ) X (X ≠ 'N') → FFFD for ESC, then '$' and ')' re-emitted, then X.
        let mut dec = Decoder::new();
        let mut out = String::new();
        let (_, had_errors) = dec.decode_to_string(&[ESC, DESIG_2, DESIG_3, b'X'], &mut out, true);
        assert!(had_errors);
        assert_eq!(out, "\u{FFFD}$)X");
    }
}
