//! Delete a file to the Recycle Bin (recoverable), as 1.2 did through PowerShell/VisualBasic, here with the
//! Windows shell API directly. Elsewhere: a plain delete.
use super::error::{CoreError, CoreResult};
use std::path::Path;

#[cfg(windows)]
pub fn recycle(file: &Path) -> CoreResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE, SHFILEOPSTRUCTW,
    };
    let abs = std::path::absolute(file)?;
    // pFrom is a list of NUL-terminated paths ending with an extra NUL.
    let mut from: Vec<u16> = abs.as_os_str().encode_wide().collect();
    from.push(0);
    from.push(0);
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE;
    op.pFrom = from.as_ptr();
    op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as _;
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 || op.fAnyOperationsAborted != 0 || abs.exists() {
        return Err(CoreError::new(format!("Could not move the file to the Recycle Bin (error {r}).")));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn recycle(file: &Path) -> CoreResult<()> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(windows)]
    #[ignore = "puts a file in the real Recycle Bin; run manually"]
    fn recycles_a_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("cs-recycle-test.txt");
        std::fs::write(&f, b"bye").unwrap();
        super::recycle(&f).unwrap();
        assert!(!f.exists());
    }
}
