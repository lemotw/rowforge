//! Durability test: kill -9 a child rowforge mid-run, verify outcomes.jsonl
//! contains all worker-acked outcomes (lines are atomic per per-line write).
//!
//! This requires:
//! - Spawning rowforge as a subprocess (assert_cmd or similar)
//! - Reliable timing to kill mid-run (after some lines are written but before complete)
//! - Reading the partial jsonl and verifying line count + content
//!
//! Hard to test reliably in CI due to timing flakiness. Manual run only.

#[test]
#[ignore = "manual durability test; requires subprocess infra + reliable mid-run kill timing"]
fn jsonl_soft_crash_durable() {
    // TODO: implement when CI infra supports subprocess kill -9 + partial jsonl verification.
    // Acceptance §16 / §12.7 expects: all bytes that reached SharedJsonlWriter::append_line
    // (i.e., went through write_all + flush) survive a kill -9.
    unimplemented!("see TODO at top of file");
}
