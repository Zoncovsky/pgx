//! Storage capability probes: does this directory's filesystem clone blocks?
use std::path::Path;

/// Probes block cloning by actually cloning a small file inside `dir`
/// (`clonefile` on macOS, `FICLONE` on Linux). True means `file_copy_method = clone`
/// shares blocks between a template and its clones on this filesystem.
pub fn supports_block_clone(dir: &Path) -> bool {
    let src = dir.join(format!(".pgx-clone-probe-{}.src", std::process::id()));
    let dst = dir.join(format!(".pgx-clone-probe-{}.dst", std::process::id()));
    let _ = std::fs::remove_file(&dst);
    if std::fs::write(&src, b"pgx block clone probe").is_err() {
        return false;
    }
    let ok = clone_file(&src, &dst);
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst);
    ok
}

#[cfg(target_os = "macos")]
fn clone_file(src: &Path, dst: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let s = std::ffi::CString::new(src.as_os_str().as_bytes()).unwrap();
    let d = std::ffi::CString::new(dst.as_os_str().as_bytes()).unwrap();
    unsafe { libc::clonefile(s.as_ptr(), d.as_ptr(), 0) == 0 }
}

#[cfg(target_os = "linux")]
fn clone_file(src: &Path, dst: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(s) = std::fs::File::open(src) else {
        return false;
    };
    let Ok(d) = std::fs::File::create(dst) else {
        return false;
    };
    unsafe { libc::ioctl(d.as_raw_fd(), libc::FICLONE as _, s.as_raw_fd()) == 0 }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn clone_file(_src: &Path, _dst: &Path) -> bool {
    false
}

/// Filesystem type name of the directory, for `pgx doctor` (best effort).
pub fn fs_type_name(dir: &Path) -> String {
    let out = std::process::Command::new("df")
        .args(["-P", "-T"])
        .arg(dir)
        .output();
    if let Ok(o) = out {
        if o.status.success() {
            if let Some(line) = String::from_utf8_lossy(&o.stdout).lines().nth(1) {
                if let Some(t) = line.split_whitespace().nth(1) {
                    return t.to_string();
                }
            }
        }
    }
    // macOS df has no -T; use mount output.
    if let Ok(o) = std::process::Command::new("mount").output() {
        let dir_s = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        let mut best: Option<(usize, String)> = None;
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            // "/dev/disk3s1 on / (apfs, local, journaled)"
            if let Some((_, rest)) = line.split_once(" on ") {
                if let Some((mp, kind)) = rest.split_once(" (") {
                    if dir_s.starts_with(mp)
                        && best.as_ref().map(|(l, _)| mp.len() > *l).unwrap_or(true)
                    {
                        best = Some((
                            mp.len(),
                            kind.split([',', ')']).next().unwrap_or("").to_string(),
                        ));
                    }
                }
            }
        }
        if let Some((_, k)) = best {
            return k;
        }
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_cleans_up() {
        let dir = std::env::temp_dir();
        let _ = supports_block_clone(&dir);
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".pgx-clone-probe")
            })
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn fsdetect_apfs_is_clone_capable() {
        let dir = std::env::temp_dir();
        assert_eq!(fs_type_name(&dir), "apfs");
        assert!(supports_block_clone(&dir));
    }
}
