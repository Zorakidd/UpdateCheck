//! Web requests through WinHTTP, the HTTP client that comes with Windows. It checks certificates against
//! Windows' own store and follows the PC's proxy settings, and it keeps a TLS library out of the app.

use std::ffi::c_void;
use std::time::Duration;

use windows_sys::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
    WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_ADDREQ_FLAG_REPLACE, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
};

use crate::system::{USER_AGENT, last_error, wide};

pub struct Response {
    pub status: u32,
    pub body: Vec<u8>,
}

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// What went wrong, in words: WinHTTP's errors aren't in Windows' general list of messages.
fn describe(code: u32) -> String {
    let text = match code {
        12002 => "the server didn't answer in time",
        12005 | 12006 => "the address isn't valid",
        12007 => "the server's name couldn't be found; is the PC online?",
        12029 => "couldn't connect to the server; is the PC online?",
        12030 => "the connection was closed before the answer arrived",
        12152 => "the server's answer couldn't be read",
        12175 | 12038 | 12037 | 12045 | 12169 => "the server's certificate couldn't be trusted",
        _ => return format!("network error {code}"),
    };
    format!("{text} (network error {code})")
}

fn check(ok: i32) -> Result<(), String> {
    if ok != 0 { Ok(()) } else { Err(describe(last_error())) }
}

/// Fetch `url` with GET, giving up after `timeout` for each step of the exchange, and on answers larger than
/// `limit` bytes. Redirects are followed; any HTTP status comes back as a response.
pub fn get(url: &str, headers: &[(&str, &str)], timeout: Duration, limit: usize) -> Result<Response, String> {
    let url_text = wide(url);
    let mut parts: URL_COMPONENTS = unsafe { std::mem::zeroed() };
    parts.dwStructSize = std::mem::size_of::<URL_COMPONENTS>() as u32;
    // Lengths without buffers: WinHTTP points into the address instead of copying.
    parts.dwHostNameLength = u32::MAX;
    parts.dwUrlPathLength = u32::MAX;
    parts.dwExtraInfoLength = u32::MAX;
    check(unsafe { WinHttpCrackUrl(url_text.as_ptr(), 0, 0, &mut parts) })?;
    let slice = |start: *mut u16, length: u32| {
        if start.is_null() { String::new() } else { String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(start, length as usize) }) }
    };
    let host = slice(parts.lpszHostName, parts.dwHostNameLength);
    let mut path = slice(parts.lpszUrlPath, parts.dwUrlPathLength);
    path.push_str(&slice(parts.lpszExtraInfo, parts.dwExtraInfoLength));
    if path.is_empty() {
        path.push('/');
    }
    // INTERNET_SCHEME_HTTPS.
    let secure = parts.nScheme == 2;

    let agent = wide(USER_AGENT);
    let mut session = Handle(unsafe {
        WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, std::ptr::null(), std::ptr::null(), 0)
    });
    if session.0.is_null() {
        session = Handle(unsafe {
            WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, std::ptr::null(), std::ptr::null(), 0)
        });
    }
    if session.0.is_null() {
        return Err(describe(last_error()));
    }
    let milliseconds = timeout.as_millis().min(i32::MAX as u128) as i32;
    check(unsafe { WinHttpSetTimeouts(session.0, milliseconds, milliseconds, milliseconds, milliseconds) })?;

    let connection = Handle(unsafe { WinHttpConnect(session.0, wide(&host).as_ptr(), parts.nPort, 0) });
    if connection.0.is_null() {
        return Err(describe(last_error()));
    }
    let request = Handle(unsafe {
        WinHttpOpenRequest(
            connection.0,
            wide("GET").as_ptr(),
            wide(&path).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            if secure { WINHTTP_FLAG_SECURE } else { 0 },
        )
    });
    if request.0.is_null() {
        return Err(describe(last_error()));
    }
    if !headers.is_empty() {
        let text: String = headers.iter().map(|(name, value)| format!("{name}: {value}\r\n")).collect();
        let text: Vec<u16> = text.encode_utf16().collect();
        check(unsafe {
            WinHttpAddRequestHeaders(
                request.0,
                text.as_ptr(),
                text.len() as u32,
                WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
            )
        })?;
    }
    check(unsafe { WinHttpSendRequest(request.0, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) })?;
    check(unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) })?;

    let mut status = 0u32;
    let mut size = 4u32;
    check(unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            std::ptr::null_mut(),
        )
    })?;

    let mut body = Vec::new();
    loop {
        let mut available = 0u32;
        check(unsafe { WinHttpQueryDataAvailable(request.0, &mut available) })?;
        if available == 0 {
            break;
        }
        if body.len() + available as usize > limit {
            return Err(format!("the answer is larger than the {} MB expected", limit.div_ceil(1024 * 1024)));
        }
        let start = body.len();
        body.resize(start + available as usize, 0);
        let mut read = 0u32;
        check(unsafe { WinHttpReadData(request.0, body[start..].as_mut_ptr().cast(), available, &mut read) })?;
        body.truncate(start + read as usize);
        if read == 0 {
            break;
        }
    }
    Ok(Response { status, body })
}
