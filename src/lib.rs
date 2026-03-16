pub mod euckp;
pub mod iso2022kp;
pub mod kps9566;

/// Indicates whether a streaming coder operation consumed all input or filled the output.
///
/// Mirrors [`encoding_rs::CoderResult`](https://docs.rs/encoding_rs/latest/encoding_rs/enum.CoderResult.html).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoderResult {
    /// All input was consumed. Provide more input (or call again with `last = true`).
    InputEmpty,
    /// The output buffer is full. Provide more output space, then resume with the same input position.
    OutputFull,
}

/// Error returned when decoding fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// A byte (or lead byte) is not valid in this encoding.
    InvalidByte { offset: usize, byte: u8 },
    /// A valid byte sequence has no Unicode mapping.
    UnmappedBytes { offset: usize },
    /// Input ended in the middle of a multi-byte sequence.
    UnexpectedEof { offset: usize },
}

/// Error returned when encoding fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// A character has no mapping in the target encoding.
    UnmappableChar { char_idx: usize, ch: char },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::InvalidByte { offset, byte } => {
                write!(f, "invalid byte 0x{byte:02X} at offset {offset}")
            }
            DecodeError::UnmappedBytes { offset } => {
                write!(f, "no Unicode mapping for byte sequence at offset {offset}")
            }
            DecodeError::UnexpectedEof { offset } => {
                write!(
                    f,
                    "unexpected end of input in multi-byte sequence at offset {offset}"
                )
            }
        }
    }
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::UnmappableChar { char_idx, ch } => write!(
                f,
                "character U+{:04X} '{}' at index {char_idx} has no mapping",
                *ch as u32, ch
            ),
        }
    }
}

impl std::error::Error for DecodeError {}
impl std::error::Error for EncodeError {}
