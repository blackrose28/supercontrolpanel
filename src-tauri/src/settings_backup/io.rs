use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

pub fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not inspect settings file".into()),
        Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
            return Err("Settings and backup paths must be regular files, not symlinks".into())
        }
        _ => {}
    }
    let file = File::open(path).map_err(|_| "Could not open settings file")?;
    let mut data = Vec::new();
    file.take(crypto_limit() + 1)
        .read_to_end(&mut data)
        .map_err(|_| "Could not read settings file")?;
    if data.len() > super::crypto::MAX_BYTES {
        return Err("Settings file exceeds size limit".into());
    }
    Ok(Some(data))
}
fn crypto_limit() -> u64 {
    super::crypto::MAX_BYTES as u64
}

pub fn atomic_write(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), String> {
    reject_unsafe_target(path, overwrite)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("An absolute destination path is required")?;
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "Could not stage settings file")?;
    restrict(temp.path())?;
    temp.write_all(bytes)
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|_| "Could not flush settings file")?;
    reject_unsafe_target(path, overwrite)?;
    persist(temp, path, overwrite)?;
    sync_directory(parent)
}

fn persist(temp: tempfile::NamedTempFile, path: &Path, overwrite: bool) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        if overwrite {
            temp.persist(path)
                .map_err(|_| "Could not replace settings file; original retained")?;
        } else {
            temp.persist_noclobber(path)
                .map_err(|_| "Destination exists or could not be created")?;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let temp = temp.into_temp_path();
        let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let flags = MOVEFILE_WRITE_THROUGH
            | if overwrite {
                MOVEFILE_REPLACE_EXISTING
            } else {
                0
            };
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
            return Err("Could not replace settings file; original retained".into());
        }
    }
    Ok(())
}

fn reject_unsafe_target(path: &Path, overwrite: bool) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => {
            Err("Refusing unsafe or symlink destination".into())
        }
        Ok(_) if !overwrite => Err("Destination exists; confirm overwrite".into()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not inspect destination".into()),
    }
}

pub(super) fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| "Could not durably flush settings directory")?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn restrict(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| "Could not restrict settings file permissions")?;
    }
    #[cfg(windows)]
    {
        use std::{os::windows::ffi::OsStrExt, ptr};
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{
                Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
            },
        };
        // Protected owner-only DACL: never inherit a permissive destination ACL.
        let sddl: Vec<u16> = "D:P(A;;FA;;;OW)\0".encode_utf16().collect();
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut descriptor = ptr::null_mut();
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            ) == 0
            {
                return Err("Could not create private settings ACL".into());
            }
            let ok = SetFileSecurityW(
                wide.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor,
            );
            LocalFree(descriptor);
            if ok == 0 {
                return Err("Could not restrict settings ACL".into());
            }
        }
    }
    Ok(())
}
