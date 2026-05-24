//! Integration test for the 256 KB import-colors size cap (Codex review #8).
//!
//! Spawns the actual vibeflow binary with --import-colors against a stub
//! file >256 KB and asserts non-zero exit + the expected stderr message.

use std::io::Write;
use std::process::Command;

#[test]
fn import_rejects_oversize_itermcolors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let oversize_path = temp.path().join("oversize.itermcolors");
    let mut f = std::fs::File::create(&oversize_path).expect("create");
    // 300 KB of zero bytes — easily over the 262_144 byte cap.
    let blob = vec![0u8; 300 * 1024];
    f.write_all(&blob).expect("write blob");
    f.sync_all().expect("sync");

    let output = Command::new(env!("CARGO_BIN_EXE_vibeflow"))
        .args(["--import-colors", oversize_path.to_str().unwrap()])
        .output()
        .expect("run vibeflow");

    assert!(
        !output.status.success(),
        "expected non-zero exit; got {}",
        output.status
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("exceeds cap"),
        "expected 'exceeds cap' in stderr; got: {stderr}"
    );
}
