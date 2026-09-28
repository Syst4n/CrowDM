//! Deterministic content generator shared by the server and the tests.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

#[inline]
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Fills `buf` with the bytes of the resource `seed` starting at `offset`.
pub fn fill(seed: u64, offset: u64, buf: &mut [u8]) {
    let mut pos = offset;
    let mut i = 0usize;
    while i < buf.len() {
        let word = splitmix64((pos / 8) ^ seed.wrapping_mul(0xA24B_AED4_963E_E407)).to_le_bytes();
        let start = (pos % 8) as usize;
        let take = (8 - start).min(buf.len() - i);
        buf[i..i + take].copy_from_slice(&word[start..start + take]);
        i += take;
        pos += take as u64;
    }
}

/// Checks that `path` holds exactly `size` bytes of resource `seed`.
/// Returns the offset of the first mismatch, if any.
pub fn verify_file(path: &Path, seed: u64, size: u64) -> io::Result<Option<u64>> {
    let mut f = File::open(path)?;
    let actual = f.metadata()?.len();
    let mut got = vec![0u8; 1 << 20];
    let mut want = vec![0u8; 1 << 20];
    let mut pos = 0u64;
    loop {
        let n = f.read(&mut got)?;
        if n == 0 {
            break;
        }
        fill(seed, pos, &mut want[..n]);
        if got[..n] != want[..n] {
            let idx = got[..n].iter().zip(&want[..n]).position(|(a, b)| a != b).unwrap_or(0);
            return Ok(Some(pos + idx as u64));
        }
        pos += n as u64;
    }
    if actual != size {
        return Ok(Some(actual.min(size)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_is_offset_consistent() {
        let mut whole = vec![0u8; 1000];
        fill(7, 0, &mut whole);
        for start in [0usize, 1, 7, 8, 9, 333] {
            let mut part = vec![0u8; 100];
            fill(7, start as u64, &mut part);
            assert_eq!(&part[..], &whole[start..start + 100]);
        }
    }
}
