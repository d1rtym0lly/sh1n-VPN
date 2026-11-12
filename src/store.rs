use crate::profile::{Profile, MAX_PROFILES};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub theme: usize,
    pub selected: usize,
    pub profiles: Vec<Profile>,
}

fn path() -> io::Result<PathBuf> {
    let root = std::env::var_os("LOCALAPPDATA").ok_or(io::ErrorKind::NotFound)?;
    Ok(PathBuf::from(root).join("sh1n VPN").join("profiles.dpapi"))
}

pub fn read() -> io::Result<Store> {
    let file = match fs::File::open(path()?) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Read the earlier beta's data as a fallback; never modify its file.
            let current = path()?;
            let base = current
                .parent()
                .and_then(Path::parent)
                .ok_or(io::ErrorKind::NotFound)?;
            let legacy = base.join("Velum").join("profiles.dpapi");
            match fs::File::open(legacy) {
                Ok(f) => f,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Store::default()),
                Err(e) => return Err(e),
            }
        }
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(64 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut data = crypt(&mut bytes, false)?;
    let parsed = serde_json::from_slice::<Store>(&data);
    data.fill(0);
    let store = parsed?;
    if store.profiles.len() > MAX_PROFILES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(store)
}

pub fn write(store: &Store) -> io::Result<()> {
    let file = path()?;
    let dir = file.parent().ok_or(io::ErrorKind::NotFound)?;
    fs::create_dir_all(dir)?;
    let mut clear = serde_json::to_vec(store)?;
    let encrypted = crypt(&mut clear, true);
    clear.fill(0);
    let encrypted = encrypted?;
    if encrypted.len() > 64 * 1024 * 1024 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let tmp = dir.join(format!("profiles-{}.new", std::process::id()));
    let mut out = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let result = out.write_all(&encrypted).and_then(|()| out.sync_all());
    drop(out);
    let result = result.and_then(|()| {
        if unsafe {
            MoveFileExW(
                wide(&tmp).as_ptr(),
                wide(&file).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    });
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn crypt(bytes: &mut [u8], protect: bool) -> io::Result<Vec<u8>> {
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_mut_ptr(),
    };
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input,
                null(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
    };
    if ok == 0 || out.pbData.is_null() {
        return Err(io::Error::last_os_error());
    }
    let slice = unsafe { std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize) };
    let result = slice.to_vec();
    if !protect {
        slice.fill(0);
    }
    unsafe {
        LocalFree(out.pbData.cast());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn dpapi_round_trip() {
        let mut input = b"synthetic fixture".to_vec();
        let mut encrypted = super::crypt(&mut input, true).unwrap();
        assert_ne!(input, encrypted);
        assert_eq!(super::crypt(&mut encrypted, false).unwrap(), input);
    }
}
