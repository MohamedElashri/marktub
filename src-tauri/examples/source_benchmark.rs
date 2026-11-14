//! Isolated byte-provenance cost; this is not a WebView/input-paint benchmark.
use marktub_lib::document::Provenance;
use sha2::{Digest, Sha256};
use std::time::Instant;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .expect("usage: source_benchmark FILE");
    let bytes = std::fs::read(path).expect("read owned UTF-8 fixture");
    let input_bytes = bytes.len();
    assert!(input_bytes > 0);
    let input_hash = format!("{:x}", Sha256::digest(&bytes));
    let start = Instant::now();
    let source = Provenance::new(&bytes).expect("validated source");
    let create_ms = start.elapsed().as_secs_f64() * 1000.0;
    drop(bytes);
    let mut seed = 71_u64;
    let mut digest = Sha256::new();
    let start = Instant::now();
    for _ in 0..1000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let byte = (seed as usize) % input_bytes;
        let from = source.utf16_at_byte(byte);
        let to = source.utf16_at_byte((byte + 8192).min(input_bytes));
        digest.update(
            source
                .logical_range(from, to)
                .expect("scalar-aligned range"),
        );
    }
    let ranges_ms = start.elapsed().as_secs_f64() * 1000.0;
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) }, 0);
    let peak_rss_bytes = if cfg!(target_os = "macos") {
        usage.ru_maxrss as u64
    } else {
        usage.ru_maxrss as u64 * 1024
    };
    println!(
        "{}",
        serde_json::json!({"scope":"isolated native provenance, uncontrolled page cache; RSS high-water is not physical footprint or app memory","inputBytes":input_bytes,"inputSha256":input_hash,"logicalUnits":source.len_utf16(),"createMs":create_ms,"thousandRangesMs":ranges_ms,"rangeSha256":format!("{:x}",digest.finalize()),"peakRssBytes":peak_rss_bytes})
    );
}
