# kps9566

Rust library for KPS 9566, EUC-KP, and ISO-2022-KP encoding/decoding.

## Features

- One-shot encode/decode APIs
- Streaming encoder/decoder APIs
- Error-reporting and replacement modes
- Exhaustive table-based tests

## Supported Encodings

- `kps9566`
- `euckp`
- `iso2022kp`

## Install

Add this crate to your `Cargo.toml`:

```toml
[dependencies]
kps9566 = "0.1"
```

## Quick Start

```rust
use kps9566::{euckp, iso2022kp, kps9566};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // KPS 9566
    let kps_bytes = kps9566::encode("안녕하십니까?")?;
    let kps_text = kps9566::decode(&kps_bytes)?;
    assert_eq!(kps_text, "안녕하십니까?");

    // EUC-KP
    let euc_bytes = euckp::encode("안녕하십니까?")?;
    let euc_text = euckp::decode(&euc_bytes)?;
    assert_eq!(euc_text, "안녕하십니까?");

    // ISO-2022-KP
    let iso_bytes = iso2022kp::encode("안녕하십니까?")?;
    let iso_text = iso2022kp::decode(&iso_bytes)?;
    assert_eq!(iso_text, "안녕하십니까?");

    Ok(())
}
```

## API Overview

Each encoding module provides:

- One-shot:
  - `decode(bytes: &[u8]) -> Result<String, DecodeError>`
  - `encode(s: &str) -> Result<Vec<u8>, EncodeError>`
- Streaming:
  - `Decoder::decode_to_string(...)` (replacement mode)
  - `Decoder::decode_to_string_without_replacement(...)` (fail-fast mode)
  - `Encoder::encode_to_vec(...)` (replacement mode)
  - `Encoder::encode_to_vec_without_replacement(...)` (fail-fast mode)

`iso2022kp::Encoder` also has:

- `finish(&mut Vec<u8>)` to emit final SI when needed.

## Streaming Example

```rust
use kps9566::kps9566::Decoder;

fn main() {
    let mut dec = Decoder::new();
    let mut out = String::new();

    // First chunk might end in the middle of a multibyte sequence.
    let (_read, had_errors) = dec.decode_to_string(&[0x81], &mut out, false);
    assert!(!had_errors);
    assert!(out.is_empty());

    // Continue with next chunk.
    let (_read, had_errors) = dec.decode_to_string(&[0x41], &mut out, true);
    assert!(!had_errors);
    assert_eq!(out, "\u{AC03}");
}
```

## Error Semantics

- Decode errors:
  - `InvalidByte { offset, byte }`
  - `UnmappedBytes { offset }`
  - `UnexpectedEof { offset }`
- Encode errors:
  - `UnmappableChar { char_idx, ch }`

Use replacement mode APIs if you want decoding/encoding to continue with replacement output.
