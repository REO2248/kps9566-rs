#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);

    if let Ok(encoded) = ::kps9566::kps9566::encode(&text) {
        assert!(::kps9566::kps9566::decode(&encoded).is_ok());
    }

    if let Ok(encoded) = ::kps9566::euckp::encode(&text) {
        assert!(::kps9566::euckp::decode(&encoded).is_ok());
    }

    // ISO-2022-KP uses ESC/SO/SI as modal control bytes.
    // Roundtrip property for arbitrary Unicode text is meaningful only when
    // the source text does not contain those control characters as data.
    let has_iso2022kp_controls = text
        .chars()
        .any(|ch| matches!(ch, '\u{001B}' | '\u{000E}' | '\u{000F}'));
    if !has_iso2022kp_controls {
        if let Ok(encoded) = ::kps9566::iso2022kp::encode(&text) {
            assert!(::kps9566::iso2022kp::decode(&encoded).is_ok());
        }
    }
});
