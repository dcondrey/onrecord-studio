use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use tokio::io::AsyncReadExt;

const HASH_BUFFER_BYTES: usize = 64 * 1024;

pub(crate) fn sha256_hex(value: &[u8]) -> String {
    digest_hex(Sha256::digest(value))
}

/// Formats a digest without relying on the optional `LowerHex` implementation from a specific
/// digest crate version. Keeping this local avoids adding a second hex-encoding dependency.
pub(crate) fn digest_hex<D: AsRef<[u8]>>(digest: D) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Ok(digest_hex(digest.finalize()));
        }
        digest.update(&buffer[..count]);
    }
}

pub(crate) async fn sha256_file_async(path: &Path) -> std::io::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut digest = Sha256::new();
    // Keep the async state machine compact; embedding 64 KiB directly would inflate every caller.
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            return Ok(digest_hex(digest.finalize()));
        }
        digest.update(&buffer[..count]);
    }
}

pub(crate) fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::{constant_time_equal, sha256_hex};

    #[test]
    fn hashes_and_compares_without_early_byte_exit() {
        let digest = sha256_hex(b"forensic evidence");
        assert_eq!(digest.len(), 64);
        assert!(constant_time_equal(digest.as_bytes(), digest.as_bytes()));
        assert!(!constant_time_equal(digest.as_bytes(), b"different"));
    }
}
