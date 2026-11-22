//! macOS: the cwd of a process through `proc_pidinfo`.

use std::mem::MaybeUninit;
use std::path::PathBuf;

/// The working directory of the process `pid`, `None` on any failure and for a path that
/// is not valid UTF-8.
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let size = libc::c_int::try_from(size_of::<libc::proc_vnodepathinfo>()).ok()?;
    let mut info = MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
    // SAFETY: `info` is a zeroed buffer of exactly `size` bytes, which `proc_pidinfo`
    // fills; the struct is plain data, so all-zero is a valid value.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: the call filled the whole struct (it returned `size`), and zero bytes were
    // valid before it.
    let info = unsafe { info.assume_init() };
    let bytes: Vec<u8> = info
        .pvi_cdir
        .vip_path
        .iter()
        .flatten()
        .map(|&c| c.cast_unsigned())
        .take_while(|&b| b != 0)
        .collect();
    let path = String::from_utf8(bytes).ok()?;
    (!path.is_empty()).then(|| PathBuf::from(path))
}
