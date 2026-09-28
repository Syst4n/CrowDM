//! Streaming file digests for completion checks.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::model::ChecksumAlgo;

fn digest<D: Digest>(path: &Path) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = D::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Lowercase hex digest of the file at `path`, read in 1 MiB blocks.
pub fn file_digest(path: &Path, algo: ChecksumAlgo) -> io::Result<String> {
    match algo {
        ChecksumAlgo::Md5 => digest::<Md5>(path),
        ChecksumAlgo::Sha1 => digest::<Sha1>(path),
        ChecksumAlgo::Sha256 => digest::<Sha256>(path),
    }
}

/// Guesses the algorithm from the length of a hex digest.
pub fn guess_algo(hex_digest: &str) -> Option<ChecksumAlgo> {
    let h = hex_digest.trim();
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        32 => Some(ChecksumAlgo::Md5),
        40 => Some(ChecksumAlgo::Sha1),
        64 => Some(ChecksumAlgo::Sha256),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("abc");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(file_digest(&p, ChecksumAlgo::Sha256).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(file_digest(&p, ChecksumAlgo::Md5).unwrap(), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(guess_algo("900150983cd24fb0d6963f7d28e17f72"), Some(ChecksumAlgo::Md5));
    }
}
