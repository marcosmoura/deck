//! macOS: the cwd of a process through `proc_pidinfo`, its arguments through `sysctl`.

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

/// The arguments of the process `pid` (`argv[0]` first), from `sysctl KERN_PROCARGS2`.
pub fn process_argv(pid: u32) -> Option<Vec<String>> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    // SAFETY: `mib` has three entries, as passed; a null buffer asks only for the size,
    // which `sysctl` writes to `size`.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    // SAFETY: `buf` is valid for `size` bytes, which `sysctl` fills and shortens `size` to
    // what it wrote; `mib` is as above.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(size);
    parse_procargs(&buf)
}

/// The `KERN_PROCARGS2` layout: an `int` argument count, the executable path, NUL padding,
/// then the arguments, NUL-terminated (the environment follows and is ignored).
fn parse_procargs(buf: &[u8]) -> Option<Vec<String>> {
    let count = usize::try_from(i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?)).ok()?;
    let rest = buf.get(4..)?;
    let path_end = rest.iter().position(|&b| b == 0)?;
    let rest = rest.get(path_end..)?;
    let start = rest.iter().position(|&b| b != 0)?;
    let args = rest
        .get(start..)?
        .split(|&b| b == 0)
        .take(count)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect::<Vec<_>>();
    (args.len() == count).then_some(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procargs_split_into_the_arguments() {
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/bin/node\0\0\0node\0/x/codex.js\0HOME=/h\0");
        assert_eq!(parse_procargs(&buf).unwrap(), ["node", "/x/codex.js"]);
        // More arguments announced than present.
        let mut short = 3i32.to_ne_bytes().to_vec();
        short.extend_from_slice(b"/bin/sh\0sh\0");
        assert_eq!(parse_procargs(&short), None);
        assert_eq!(parse_procargs(b"\x01"), None);
    }

    #[test]
    fn the_argv_of_this_process_is_readable() {
        let argv = process_argv(std::process::id()).unwrap();
        assert!(!argv.is_empty());
    }
}
