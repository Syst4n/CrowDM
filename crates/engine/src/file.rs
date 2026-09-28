//! Temporary ("part") file handling.
//!
//! Data is written with positioned writes at explicit offsets, so segments
//! never share a cursor. Known-size files are created *sparse* and extended
//! to their final length up front: on NTFS this reserves the length without
//! the multi-gigabyte zero-fill that a plain `SetEndOfFile` followed by a
//! write near the end would trigger.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub struct PartFile {
    file: Arc<File>,
    path: PathBuf,
}

impl PartFile {
    /// Creates (truncating) a part file. When `size` is known the file is
    /// marked sparse and extended to `size`.
    pub fn create(path: &Path, size: Option<u64>) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
        if let Some(size) = size {
            set_sparse(&file, true);
            file.set_len(size)?;
        }
        Ok(PartFile { file: Arc::new(file), path: path.to_path_buf() })
    }

    /// Opens an existing part file for resuming.
    pub fn open_existing(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(PartFile { file: Arc::new(file), path: path.to_path_buf() })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    pub fn set_len(&self, len: u64) -> io::Result<()> {
        self.file.set_len(len)
    }

    /// Writes all of `buf` at `offset` (blocking).
    pub fn write_all_at(&self, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
        while !buf.is_empty() {
            let n = write_at(&self.file, buf, offset)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "failed to write to part file"));
            }
            buf = &buf[n..];
            offset += n as u64;
        }
        Ok(())
    }

    /// Flushes written data to the device (blocking).
    pub fn sync(&self) -> io::Result<()> {
        self.file.sync_data()
    }

    /// Clears the sparse flag once every byte has been written.
    pub fn clear_sparse(&self) {
        set_sparse(&self.file, false);
    }

    pub fn file(&self) -> &File {
        &self.file
    }
}

#[cfg(windows)]
fn write_at(file: &File, buf: &[u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_write(buf, offset)
}

#[cfg(unix)]
fn write_at(file: &File, buf: &[u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.write_at(buf, offset)
}

#[cfg(windows)]
fn set_sparse(file: &File, sparse: bool) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Ioctl::{FILE_SET_SPARSE_BUFFER, FSCTL_SET_SPARSE};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    let mut input = FILE_SET_SPARSE_BUFFER { SetSparse: sparse as _ };
    let mut returned = 0u32;
    // Best effort: FAT32/exFAT volumes do not support sparse files.
    unsafe {
        DeviceIoControl(
            file.as_raw_handle() as _,
            FSCTL_SET_SPARSE,
            &mut input as *mut _ as *const _,
            std::mem::size_of::<FILE_SET_SPARSE_BUFFER>() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        );
    }
}

#[cfg(not(windows))]
fn set_sparse(_file: &File, _sparse: bool) {}

/// Bytes actually allocated on disk for `path` (differs from the length for
/// sparse files). Used by tests and diagnostics.
#[cfg(windows)]
pub fn allocated_size(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetCompressedFileSizeW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut high = 0u32;
    let low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut high) };
    if low == u32::MAX {
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(0) {
            return Err(err);
        }
    }
    Ok(((high as u64) << 32) | low as u64)
}

#[cfg(not(windows))]
pub fn allocated_size(path: &Path) -> io::Result<u64> {
    Ok(std::fs::metadata(path)?.len())
}

/// Free bytes available to the current user on the volume holding `dir`
/// (walking up to the nearest existing ancestor).
#[cfg(windows)]
pub fn available_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut probe = dir.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let wide: Vec<u16> = probe.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

#[cfg(not(windows))]
pub fn available_space(_dir: &Path) -> Option<u64> {
    None
}

/// Moves a finished part file to its final name. Never overwrites an
/// existing file unless `overwrite` is set; on a collision the next free
/// "name (n).ext" is used. Returns the final path.
pub fn finalize(part: &Path, target: &Path, overwrite: bool) -> io::Result<PathBuf> {
    let dir = target.parent().unwrap_or(Path::new("."));
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "download".into());
    if overwrite && target.exists() {
        std::fs::remove_file(target)?;
    }
    let mut dest = target.to_path_buf();
    for _ in 0..50 {
        if dest.exists() {
            dest = crate::names::unique_path(dir, &name, &Default::default());
        }
        match rename_no_replace(part, &dest) {
            Ok(()) => return Ok(dest),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not find a free file name"))
}

#[cfg(windows)]
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let a: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let b: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // Without MOVEFILE_REPLACE_EXISTING the call fails if `to` exists.
    let ok = unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if ok == 0 {
        let err = io::Error::last_os_error();
        // ERROR_ALREADY_EXISTS (183) / ERROR_FILE_EXISTS (80)
        if matches!(err.raw_os_error(), Some(183) | Some(80)) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, err));
        }
        return Err(err);
    }
    Ok(())
}

#[cfg(not(windows))]
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    if to.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "exists"));
    }
    std::fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_preallocation_is_cheap() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.crowdm-part");
        let size = 100u64 * 1024 * 1024 * 1024; // 100 GiB
        let f = PartFile::create(&p, Some(size)).unwrap();
        assert_eq!(f.len().unwrap(), size);
        // Write near the end: must not zero-fill 100 GiB.
        let t = std::time::Instant::now();
        f.write_all_at(&[7u8; 4096], size - 4096).unwrap();
        f.write_all_at(&[9u8; 4096], 0).unwrap();
        f.sync().unwrap();
        assert!(t.elapsed().as_secs() < 5, "tail write took {:?}", t.elapsed());
        if cfg!(windows) {
            let alloc = allocated_size(&p).unwrap();
            assert!(alloc < 64 * 1024 * 1024, "allocated {alloc} bytes");
        }
        drop(f);
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn finalize_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("f.bin");
        std::fs::write(&target, b"old").unwrap();
        let part = dir.path().join("f.bin.crowdm-part");
        std::fs::write(&part, b"new").unwrap();
        let out = finalize(&part, &target, false).unwrap();
        assert_eq!(out, dir.path().join("f (1).bin"));
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(std::fs::read(&out).unwrap(), b"new");
    }
}
