use std::{io, os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Storage::FileSystem::{
        FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
        FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FindCloseChangeNotification,
        FindFirstChangeNotificationW, FindNextChangeNotification,
    },
    System::Threading::WaitForSingleObject,
};

/// Owned and used exclusively inside the filesystem worker.
pub(super) struct Notification(HANDLE);

impl Notification {
    pub(super) fn open(path: &Path, contents: bool) -> io::Result<Self> {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        wide.push(0);
        let filter = FILE_NOTIFY_CHANGE_DIR_NAME
            | if contents {
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
                    | FILE_NOTIFY_CHANGE_SIZE
                    | FILE_NOTIFY_CHANGE_ATTRIBUTES
            } else {
                0
            };
        // SAFETY: `wide` is NUL-terminated and lives through this call. The
        // successful handle is uniquely owned here and closed by Drop.
        let handle = unsafe { FindFirstChangeNotificationW(wide.as_ptr(), 0, filter) };
        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }

    pub(super) fn changed(&self) -> io::Result<bool> {
        // SAFETY: the worker owns a live change-notification handle. A zero
        // timeout checks readiness without blocking on a filesystem change.
        match unsafe { WaitForSingleObject(self.0, 0) } {
            WAIT_TIMEOUT => Ok(false),
            WAIT_OBJECT_0 => {
                // Re-arm before requesting a snapshot, so changes during the
                // read remain pending for the following check.
                // SAFETY: this is the same live change-notification handle.
                if unsafe { FindNextChangeNotification(self.0) } == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(true)
                }
            }
            _ => Err(io::Error::last_os_error()),
        }
    }
}

impl Drop for Notification {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the handle, and this is its only close.
        unsafe { FindCloseChangeNotification(self.0) };
    }
}
