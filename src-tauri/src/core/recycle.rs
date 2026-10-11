//! Delete a file to the Recycle Bin (recoverable), as 1.2 did through PowerShell/VisualBasic, here with the
//! Windows shell API directly. Elsewhere: a plain delete.
use super::error::{CoreError, CoreResult};
use std::path::Path;

/// True if the drive holding `file` has a Recycle Bin (network and many removable drives do not: there a "delete
/// to the bin" would delete permanently).
#[cfg(windows)]
pub fn has_recycle_bin(file: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};
    let Ok(abs) = std::path::absolute(file) else { return false };
    let Some(root) = abs.ancestors().last().map(|r| r.to_path_buf()) else { return false };
    let mut w: Vec<u16> = root.as_os_str().encode_wide().collect();
    w.push(0);
    let mut info: SHQUERYRBINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHQUERYRBINFO>() as u32;
    unsafe { SHQueryRecycleBinW(w.as_ptr(), &mut info) == 0 }
}

/// Move `file` to the Recycle Bin. Never deletes permanently on its own: a drive without a bin is refused up front,
/// and a file the bin cannot take (too big) brings up Windows' own "delete permanently?" question
/// (FOF_WANTNUKEWARNING); if the user says no, the file stays and an error is returned.
#[cfg(windows)]
pub fn recycle(file: &Path) -> CoreResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE,
        SHFILEOPSTRUCTW,
    };
    let abs = std::path::absolute(file)?;
    if !has_recycle_bin(&abs) {
        return Err(CoreError::new(
            "This drive has no Recycle Bin, so the file was not deleted. Delete it yourself in Explorer if you want it gone.",
        ));
    }
    // pFrom is a list of NUL-terminated paths ending with an extra NUL.
    let mut from: Vec<u16> = abs.as_os_str().encode_wide().collect();
    from.push(0);
    from.push(0);
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE;
    op.pFrom = from.as_ptr();
    op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING | FOF_SILENT | FOF_NOERRORUI) as _;
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 || op.fAnyOperationsAborted != 0 || abs.exists() {
        return Err(CoreError::new(format!("The file was not moved to the Recycle Bin (error {r}); it was left in place.")));
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
    fn system_drive_has_a_bin() {
        let d = tempfile::tempdir().unwrap();
        assert!(super::has_recycle_bin(&d.path().join("x.mp4")));
        assert!(!super::has_recycle_bin(std::path::Path::new(r"\\127.0.0.1\nonexistent-share-cs\x.mp4")));
    }

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
