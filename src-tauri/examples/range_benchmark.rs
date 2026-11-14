#[allow(dead_code)]
#[path = "../src/files.rs"]
mod files;
#[allow(dead_code)]
#[path = "../src/scoped_file.rs"]
mod scoped_file;

use std::time::Instant;

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .expect("usage: range_benchmark FILE");
    let docs = files::Documents::default();
    let start = Instant::now();
    let descriptor = docs
        .grant_with_policy(std::path::Path::new(&path), false, |_| {})
        .expect("open fixture");
    let grant_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let range = docs
        .read_range(&descriptor.id, 0, files::MAX_RANGE)
        .expect("read range");
    let range_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(docs.status(&descriptor.id).unwrap(), "unchanged");
    docs.release(&descriptor.id).unwrap();
    println!(
        "{}",
        serde_json::json!({"fileBytes": descriptor.byte_length, "rangeBytes": range.text.len(), "grantMs": grant_ms, "firstRangeMs": range_ms, "watching": descriptor.watching})
    );
}
