#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ::kps9566::kps9566::decode(data);
    let _ = ::kps9566::euckp::decode(data);
    let _ = ::kps9566::iso2022kp::decode(data);

    let mut out = String::new();
    let mut dec = ::kps9566::kps9566::Decoder::new();
    let _ = dec.decode_to_string(data, &mut out, true);

    let mut out = String::new();
    let mut dec = ::kps9566::euckp::Decoder::new();
    let _ = dec.decode_to_string(data, &mut out, true);

    let mut out = String::new();
    let mut dec = ::kps9566::iso2022kp::Decoder::new();
    let _ = dec.decode_to_string(data, &mut out, true);

    let text = String::from_utf8_lossy(data);
    let _ = ::kps9566::kps9566::encode(&text);
    let _ = ::kps9566::euckp::encode(&text);
    let _ = ::kps9566::iso2022kp::encode(&text);

    let mut dst = Vec::new();
    let enc = ::kps9566::kps9566::Encoder::new();
    let _ = enc.encode_to_vec(&text, &mut dst);

    dst.clear();
    let enc = ::kps9566::euckp::Encoder::new();
    let _ = enc.encode_to_vec(&text, &mut dst);

    dst.clear();
    let mut enc = ::kps9566::iso2022kp::Encoder::new();
    let _ = enc.encode_to_vec(&text, &mut dst);
    let _ = enc.finish(&mut dst);
});
