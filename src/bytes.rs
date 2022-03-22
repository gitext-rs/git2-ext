// From git2 crate
#[cfg(unix)]
pub(crate) fn bytes2path(b: &[u8]) -> &std::path::Path {
    use std::os::unix::prelude::*;
    std::path::Path::new(std::ffi::OsStr::from_bytes(b))
}

// From git2 crate
#[cfg(windows)]
pub(crate) fn bytes2path(b: &[u8]) -> &std::path::Path {
    use std::str;
    std::path::Path::new(str::from_utf8(b).unwrap())
}
