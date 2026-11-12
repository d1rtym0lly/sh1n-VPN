use crate::{profile::MAX_INPUT, ui::wide};
use std::{
    ffi::c_void,
    mem::size_of,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use url::Url;
use windows_sys::Win32::Networking::WinHttp::*;

struct Http(*mut c_void);
impl Drop for Http {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
}

pub fn parse(input: &str) -> Option<Url> {
    let s = input.trim();
    if s.len() > 4096 || s.chars().any(char::is_whitespace) {
        return None;
    }
    let u = Url::parse(s).ok()?;
    (u.scheme() == "https"
        && u.host_str().is_some()
        && u.username().is_empty()
        && u.password().is_none()
        && u.fragment().is_none())
    .then_some(u)
}

pub fn fetch(u: &Url) -> Result<String, String> {
    const FAIL: &str = "Не удалось получить подписку по HTTPS. Проверьте доступ к провайдеру.";
    let host = u.host_str().ok_or(FAIL)?;
    let port = u.port_or_known_default().ok_or(FAIL)?;
    let path = match u.query() {
        Some(q) => format!("{}?{q}", u.path()),
        None => u.path().to_owned(),
    };
    unsafe {
        let session = Http(WinHttpOpen(
            wide("sh1n-vpn/0.2").as_ptr(),
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            null(),
            null(),
            0,
        ));
        if session.0.is_null() {
            return Err(FAIL.into());
        }
        WinHttpSetTimeouts(session.0, 7000, 7000, 7000, 7000);
        let conn = Http(WinHttpConnect(session.0, wide(host).as_ptr(), port, 0));
        if conn.0.is_null() {
            return Err(FAIL.into());
        }
        let req = Http(WinHttpOpenRequest(
            conn.0,
            wide("GET").as_ptr(),
            wide(&path).as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err(FAIL.into());
        }
        let redirects = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
        let disabled = WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION;
        if WinHttpSetOption(
            req.0,
            WINHTTP_OPTION_REDIRECT_POLICY,
            (&redirects as *const u32).cast(),
            4,
        ) == 0
            || WinHttpSetOption(
                req.0,
                WINHTTP_OPTION_DISABLE_FEATURE,
                (&disabled as *const u32).cast(),
                4,
            ) == 0
        {
            return Err(FAIL.into());
        }
        let headers = wide("Accept: application/json, text/plain, */*\r\n");
        if WinHttpSendRequest(
            req.0,
            headers.as_ptr(),
            (headers.len() - 1) as u32,
            null(),
            0,
            0,
            0,
        ) == 0
            || WinHttpReceiveResponse(req.0, null_mut()) == 0
        {
            return Err(FAIL.into());
        }
        let mut status = 0u32;
        let mut sz = size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut sz,
            null_mut(),
        ) == 0
        {
            return Err(FAIL.into());
        }
        if (300..400).contains(&status) {
            return Err("Провайдер вернул перенаправление. Нужна прямая HTTPS-ссылка.".into());
        }
        if status != 200 {
            return Err(format!("Провайдер не выдал профили (HTTP {status})."));
        }
        let mut hidden = [0u16; 64];
        let mut bytes = size_of::<[u16; 64]>() as u32;
        if WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_CUSTOM,
            wide("hide-settings").as_ptr(),
            hidden.as_mut_ptr().cast(),
            &mut bytes,
            null_mut(),
        ) != 0
        {
            let s = String::from_utf16_lossy(&hidden[..(bytes as usize / 2).min(hidden.len())]);
            if s.trim() == "1" || s.trim().eq_ignore_ascii_case("true") {
                return Err(
                    "Провайдер пометил настройки подписки как скрытые. Профили не перенесены."
                        .into(),
                );
            }
        }
        let mut out = Vec::new();
        let mut buffer = [0u8; 16384];
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            let mut got = 0;
            if Instant::now() >= deadline
                || WinHttpReadData(
                    req.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut got,
                ) == 0
            {
                return Err(FAIL.into());
            }
            if got == 0 {
                break;
            }
            if out.len() + got as usize > MAX_INPUT {
                return Err("Ответ провайдера превышает 4 МБ.".into());
            }
            out.extend_from_slice(&buffer[..got as usize]);
        }
        String::from_utf8(out).map_err(|_| "Провайдер не вернул текстовую подписку UTF-8.".into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn restricts_subscription_scheme_and_credentials() {
        assert!(super::parse("https://example.invalid/path?token=fixture").is_some());
        assert!(super::parse("http://example.invalid/path").is_none());
        assert!(super::parse("https://name:secret@example.invalid").is_none());
        assert!(super::parse("https://example.invalid/one\nhttps://example.invalid/two").is_none());
    }
}
