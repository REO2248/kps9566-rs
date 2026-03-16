use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

type DynError = Box<dyn Error>;

#[inline]
fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn parse_mapping(path: &str) -> Result<Vec<(u32, u32)>, DynError> {
    println!("cargo:rerun-if-changed={path}");
    let f = File::open(path)?;
    let mut mappings = Vec::new();

    for (line_no, line) in BufReader::new(f).lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        let mut parts = line.splitn(3, '\t');
        let src_raw = parts
            .next()
            .ok_or_else(|| invalid_data(format!("{path}:{line_no}: missing source code point")))?;
        let dst_raw = parts.next().ok_or_else(|| {
            invalid_data(format!("{path}:{line_no}: missing destination code point"))
        })?;

        let src = u32::from_str_radix(src_raw.trim_start_matches("0x"), 16).map_err(|e| {
            invalid_data(format!(
                "{path}:{line_no}: invalid source hex '{src_raw}': {e}"
            ))
        })?;
        let dst = u32::from_str_radix(dst_raw.trim_start_matches("0x"), 16).map_err(|e| {
            invalid_data(format!(
                "{path}:{line_no}: invalid destination hex '{dst_raw}': {e}"
            ))
        })?;

        mappings.push((src, dst));
    }

    Ok(mappings)
}

fn write_u16_array(f: &mut File, name: &str, doc: &str, table: &[u16]) -> io::Result<()> {
    writeln!(f, "/// {doc}")?;
    writeln!(f, "pub static {name}: [u16; {}] = [", table.len())?;
    for chunk in table.chunks(16) {
        let s: Vec<_> = chunk.iter().map(|v| format!("0x{v:04X}")).collect();
        writeln!(f, "    {},", s.join(", "))?;
    }
    writeln!(f, "];")?;
    writeln!(f)?;
    Ok(())
}

fn main() -> Result<(), DynError> {
    let out_dir = env::var("OUT_DIR")?;
    generate_kps9566(&out_dir)?;
    generate_euckp(&out_dir)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// KPS 9566
// ---------------------------------------------------------------------------

fn generate_kps9566(out_dir: &str) -> Result<(), DynError> {
    let entries = parse_mapping("data/KPS9566.TXT")?;

    // Lead: 0x81..=0xFE (126 values), Trail: 0x41..=0xFE (190 values)
    const LEAD_MIN: u8 = 0x81;
    const LEAD_MAX: u8 = 0xFE;
    const TRAIL_MIN: u8 = 0x41;
    const TRAIL_MAX: u8 = 0xFF;
    const TRAIL_COUNT: usize = (TRAIL_MAX - TRAIL_MIN + 1) as usize; // 191
    const LEAD_COUNT: usize = (LEAD_MAX - LEAD_MIN + 1) as usize; // 126
    const TABLE_SIZE: usize = LEAD_COUNT * TRAIL_COUNT; // 24066

    let mut decode = vec![0xFFFFu16; TABLE_SIZE];

    // Collect (unicode, kps_bytes); dedup later by sorting and taking first
    let mut encode: Vec<(u16, u16)> = Vec::with_capacity(entries.len());

    for &(src, dst) in &entries {
        let lead = (src >> 8) as u8;
        let trail = (src & 0xFF) as u8;
        if !(LEAD_MIN..=LEAD_MAX).contains(&lead) {
            return Err(invalid_data(format!(
                "KPS9566: unexpected lead byte {lead:#04x} in entry {src:#06x}"
            ))
            .into());
        }
        if trail < TRAIL_MIN {
            return Err(invalid_data(format!(
                "KPS9566: unexpected trail byte {trail:#04x} in entry {src:#06x}"
            ))
            .into());
        }
        let ptr = (lead - LEAD_MIN) as usize * TRAIL_COUNT + (trail - TRAIL_MIN) as usize;
        decode[ptr] = dst as u16;
        encode.push((dst as u16, src as u16));
    }

    // Sort by (unicode, kps_bytes); dedup keeping first (arbitrary but deterministic)
    encode.sort_unstable_by_key(|&(u, k)| (u, k));
    encode.dedup_by_key(|e| e.0);

    let dest = Path::new(out_dir).join("kps9566_tables.rs");
    let mut f = File::create(&dest)?;

    writeln!(f, "pub const KPS9566_LEAD_MIN: u8 = 0x{LEAD_MIN:02X};")?;
    writeln!(f, "pub const KPS9566_LEAD_MAX: u8 = 0x{LEAD_MAX:02X};")?;
    writeln!(f, "pub const KPS9566_TRAIL_MIN: u8 = 0x{TRAIL_MIN:02X};")?;
    writeln!(f, "pub const KPS9566_TRAIL_MAX: u8 = 0x{TRAIL_MAX:02X};")?;
    writeln!(f, "pub const KPS9566_TRAIL_COUNT: usize = {TRAIL_COUNT};")?;
    writeln!(f)?;

    write_u16_array(
        &mut f,
        "KPS9566_DECODE",
        "Decode: index = (lead - LEAD_MIN) * TRAIL_COUNT + (trail - TRAIL_MIN). 0xFFFF = unmapped.",
        &decode,
    )?;

    writeln!(
        f,
        "/// Encode: sorted (unicode, kps_bytes) pairs for binary search."
    )?;
    writeln!(
        f,
        "pub static KPS9566_ENCODE: [(u16, u16); {}] = [",
        encode.len()
    )?;
    for &(u, k) in &encode {
        writeln!(f, "    (0x{u:04X}, 0x{k:04X}),")?;
    }
    writeln!(f, "];")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// EUC-KP
// ---------------------------------------------------------------------------

fn generate_euckp(out_dir: &str) -> Result<(), DynError> {
    let all_entries = parse_mapping("data/EUCKP.TXT")?;

    // 2-byte entries: src <= 0xFFFF  (lead 0xA1..=0xFE, trail 0xA1..=0xFE)
    // SS3  entries:  src >  0xFFFF  (0x8F prefix + 2 bytes: 0x8FXXYY)
    let mut entries_2byte: Vec<(u32, u32)> = Vec::new();
    let mut entries_ss3: Vec<(u32, u32)> = Vec::new();
    for &(src, dst) in &all_entries {
        if src > 0xFFFF {
            entries_ss3.push((src, dst));
        } else {
            entries_2byte.push((src, dst));
        }
    }

    // --- 2-byte decode table ---
    const L2_MIN: u8 = 0xA1;
    const L2_MAX: u8 = 0xFE;
    const T2_MIN: u8 = 0xA1;
    const T2_MAX: u8 = 0xFF;
    const T2_COUNT: usize = (T2_MAX - T2_MIN + 1) as usize; // 95
    const L2_COUNT: usize = (L2_MAX - L2_MIN + 1) as usize; // 94
    const TABLE_2_SIZE: usize = L2_COUNT * T2_COUNT; // 8930

    let mut decode_2 = vec![0xFFFFu16; TABLE_2_SIZE];
    for &(src, dst) in &entries_2byte {
        let lead = (src >> 8) as u8;
        let trail = (src & 0xFF) as u8;
        if !(L2_MIN..=L2_MAX).contains(&lead) || trail < T2_MIN {
            return Err(
                invalid_data(format!("EUCKP 2byte: unexpected bytes in entry {src:#06x}")).into(),
            );
        }
        let ptr = (lead - L2_MIN) as usize * T2_COUNT + (trail - T2_MIN) as usize;
        decode_2[ptr] = dst as u16;
    }

    // --- SS3 decode table: src = 0x8FXXYY; b2=XX (0xA1..=0xFE), b3=YY (0xA1..=0xFE) ---
    const SS3_B2_MIN: u8 = 0xA1;
    const SS3_B2_MAX: u8 = 0xFE;
    const SS3_B3_MIN: u8 = 0xA1;
    const SS3_B3_MAX: u8 = 0xFE;
    const SS3_B3_COUNT: usize = (SS3_B3_MAX - SS3_B3_MIN + 1) as usize; // 94
    const SS3_B2_COUNT: usize = (SS3_B2_MAX - SS3_B2_MIN + 1) as usize; // 94
    const TABLE_SS3_SIZE: usize = SS3_B2_COUNT * SS3_B3_COUNT; // 8836

    let mut decode_ss3 = vec![0xFFFFu16; TABLE_SS3_SIZE];
    for &(src, dst) in &entries_ss3 {
        // src = 0x00_8F_XX_YY; high byte is always 0x00, next is 0x8F
        let b2 = ((src >> 8) & 0xFF) as u8;
        let b3 = (src & 0xFF) as u8;
        if !(SS3_B2_MIN..=SS3_B2_MAX).contains(&b2) || !(SS3_B3_MIN..=SS3_B3_MAX).contains(&b3) {
            return Err(
                invalid_data(format!("EUCKP SS3: unexpected bytes in entry {src:#08x}")).into(),
            );
        }
        let ptr = (b2 - SS3_B2_MIN) as usize * SS3_B3_COUNT + (b3 - SS3_B3_MIN) as usize;
        decode_ss3[ptr] = dst as u16;
    }

    // --- Encode table ---
    // Collect all entries; sort by (unicode, src) so 2-byte (src <= 0xFFFF) sorts before SS3.
    // Dedup by unicode keeping first = prefer 2-byte.
    let mut encode: Vec<(u16, u32)> = all_entries
        .iter()
        .map(|&(src, dst)| (dst as u16, src))
        .collect();
    encode.sort_unstable_by_key(|&(u, k)| (u, k));
    encode.dedup_by_key(|e| e.0);

    // Output
    let dest = Path::new(out_dir).join("euckp_tables.rs");
    let mut f = File::create(&dest)?;

    writeln!(f, "pub const EUCKP_L2_MIN: u8 = 0x{L2_MIN:02X};")?;
    writeln!(f, "pub const EUCKP_L2_MAX: u8 = 0x{L2_MAX:02X};")?;
    writeln!(f, "pub const EUCKP_T2_MIN: u8 = 0x{T2_MIN:02X};")?;
    writeln!(f, "pub const EUCKP_T2_MAX: u8 = 0x{T2_MAX:02X};")?;
    writeln!(f, "pub const EUCKP_T2_COUNT: usize = {T2_COUNT};")?;
    writeln!(f, "pub const EUCKP_SS3_B2_MIN: u8 = 0x{SS3_B2_MIN:02X};")?;
    writeln!(f, "pub const EUCKP_SS3_B2_MAX: u8 = 0x{SS3_B2_MAX:02X};")?;
    writeln!(f, "pub const EUCKP_SS3_B2_COUNT: usize = {SS3_B2_COUNT};")?;
    writeln!(f, "pub const EUCKP_SS3_B3_MIN: u8 = 0x{SS3_B3_MIN:02X};")?;
    writeln!(f, "pub const EUCKP_SS3_B3_MAX: u8 = 0x{SS3_B3_MAX:02X};")?;
    writeln!(f, "pub const EUCKP_SS3_B3_COUNT: usize = {SS3_B3_COUNT};")?;
    writeln!(f)?;

    let decode_2_doc = format!(
        "EUC-KP 2-byte decode: index = (lead - 0xA1) * {T2_COUNT} + (trail - 0xA1). 0xFFFF = unmapped."
    );

    write_u16_array(&mut f, "EUCKP_DECODE_2BYTE", &decode_2_doc, &decode_2)?;

    let decode_ss3_doc = format!(
        "EUC-KP SS3 3-byte decode: index = (b2 - 0xA1) * {SS3_B3_COUNT} + (b3 - 0xA1). 0xFFFF = unmapped."
    );

    write_u16_array(&mut f, "EUCKP_DECODE_SS3", &decode_ss3_doc, &decode_ss3)?;

    writeln!(
        f,
        "/// EUC-KP encode: sorted (unicode, encoded_value) pairs."
    )?;
    writeln!(
        f,
        "/// encoded_value <= 0xFFFF → 2-byte EUC-KP; > 0xFFFF → SS3 (first byte 0x8F, then next 2 bytes)."
    )?;
    writeln!(
        f,
        "pub static EUCKP_ENCODE: [(u16, u32); {}] = [",
        encode.len()
    )?;
    for &(u, k) in &encode {
        writeln!(f, "    (0x{u:04X}, 0x{k:08X}),")?;
    }
    writeln!(f, "];")?;

    Ok(())
}
