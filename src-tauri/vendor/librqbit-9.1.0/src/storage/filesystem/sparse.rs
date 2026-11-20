// Modified by Kiwix Tauri. Supply the Windows byte-count output.

#[cfg(windows)]
#[allow(unsafe_code)]
pub(super) fn mark_file_sparse(file: &std::fs::File) -> bool {
    use std::os::windows::io::{AsHandle, AsRawHandle};
    use windows::{
        Win32::Foundation::HANDLE, Win32::System::IO::DeviceIoControl,
        Win32::System::Ioctl::FSCTL_SET_SPARSE,
    };

    let borrowed = file.as_handle();
    let handle = HANDLE(borrowed.as_raw_handle());
    let mut bytes_returned = 0;

    // SAFETY: The caller owns the synchronous OpenOptions file for the
    // duration of this call. This operation has no input or output buffer.
    // Synchronous DeviceIoControl requires a valid bytes-returned pointer.
    unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_SPARSE,
            None,
            0,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        )
        .is_ok()
    }
}
