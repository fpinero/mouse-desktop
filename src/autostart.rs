//! Starting with Windows, without Administrator privileges.
//!
//! `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` is writable by a
//! standard user, so no scheduled task, service or machine-wide registry key is needed.
//! Nothing is written outside the user's own profile.

use std::path::PathBuf;
use std::ptr;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "mouse-desktop";

/// Whether Windows is set to start this exact executable at sign-in.
///
/// A value left behind by a copy that has since been moved or deleted counts as off, so
/// the tray menu never claims a stale entry is working.
pub fn is_enabled() -> bool {
    match (read_value(), command_line()) {
        (Some(stored), Some(expected)) => stored.eq_ignore_ascii_case(&expected),
        _ => false,
    }
}

/// Add or remove the sign-in entry.
pub fn set(enabled: bool) -> Result<(), String> {
    let key = open_run_key(KEY_WRITE)?;
    let name = wide(VALUE_NAME);

    let status = if enabled {
        let command = command_line().ok_or("could not determine the path of this executable")?;
        let data = wide(&command);
        let bytes = std::mem::size_of_val(data.as_slice()) as u32;
        unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast::<u8>(),
                bytes,
            )
        }
    } else {
        unsafe { RegDeleteValueW(key, name.as_ptr()) }
    };

    unsafe { RegCloseKey(key) };

    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("registry write failed with status {status}"))
    }
}

/// The command Windows should run, which is the executable path in quotes so that spaces
/// in the folder name do not split it.
fn command_line() -> Option<String> {
    let exe: PathBuf = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

fn read_value() -> Option<String> {
    let key = open_run_key(KEY_READ).ok()?;
    let name = wide(VALUE_NAME);

    // Ask for the size first: the path length is not known in advance.
    let mut bytes: u32 = 0;
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        unsafe { RegCloseKey(key) };
        return None;
    }

    let mut buffer = vec![0u16; bytes as usize / 2 + 1];
    let mut bytes_out = bytes;
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
            buffer.as_mut_ptr().cast::<u8>(),
            &mut bytes_out,
        )
    };
    unsafe { RegCloseKey(key) };

    if status != ERROR_SUCCESS {
        return None;
    }

    let text: Vec<u16> = buffer.into_iter().take_while(|unit| *unit != 0).collect();
    Some(String::from_utf16_lossy(&text))
}

fn open_run_key(access: u32) -> Result<HKEY, String> {
    let path = wide(RUN_KEY);
    let mut key: HKEY = ptr::null_mut();

    // The Run key always exists on a normal Windows installation, so opening is enough
    // and there is no need to ask for creation rights.
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, access, &mut key) };

    if status == ERROR_SUCCESS {
        Ok(key)
    } else {
        Err(format!("could not open the Run key, status {status}"))
    }
}

/// A null terminated UTF-16 string, the form every wide Win32 entry point expects.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_strings_are_null_terminated() {
        assert_eq!(wide("ab"), vec![b'a' as u16, b'b' as u16, 0]);
        assert_eq!(wide(""), vec![0]);
    }

    #[test]
    fn the_command_line_is_quoted_so_spaces_do_not_split_it() {
        let command = command_line().expect("the test binary has a path");
        assert!(
            command.starts_with('"') && command.ends_with('"'),
            "{command}"
        );
    }

    #[test]
    fn reading_the_current_state_never_panics() {
        // Whatever the machine happens to have configured, this must answer rather than
        // fail: the tray menu calls it every time it opens.
        let _ = is_enabled();
    }
}
