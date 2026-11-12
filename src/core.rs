use crate::profile::Profile;
use serde_json::{json, Value};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::windows::{io::AsRawHandle, process::CommandExt};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::ptr::{null, null_mut};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::{
    CheckTokenMembership, CreateWellKnownSid, WinBuiltinAdministratorsSid,
};
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Proxy,
    Tun,
}

struct Job(HANDLE);
// An owned kernel handle may be used or closed from any thread.
unsafe impl Send for Job {}
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub struct Core {
    child: Child,
    _job: Job,
    pub socks: u16,
    pub http: u16,
    pub mode: Mode,
}

fn exe() -> io::Result<PathBuf> {
    let dir = std::env::current_exe()?
        .parent()
        .ok_or(io::ErrorKind::NotFound)?
        .to_owned();
    let path = dir.join("core").join("xray.exe");
    if path.is_file() {
        Ok(path)
    } else {
        Err(io::ErrorKind::NotFound.into())
    }
}

pub fn config(profile: &Profile, mode: Mode, socks: u16, http: u16) -> Value {
    let mut inbound = Vec::new();
    if mode == Mode::Tun {
        inbound.push(json!({"tag":"velum-tun","protocol":"tun","settings":{
            "name":"sh1nTunnel","desc":"sh1n VPN","mtu":1400,
            "gateway":["172.30.253.1/30","fd72:7665:6c75::1/64"],
            "dns":["1.1.1.1","2606:4700:4700::1111"],
            "autoSystemRoutingTable":["0.0.0.0/0","::/0"],
            "autoOutboundsInterface":"auto",
            "autoSystemWfpBlockLeak":["dns","misconfigtun"]
        }}));
    }
    inbound.extend([
        json!({"tag":"velum-socks","listen":"127.0.0.1","port":socks,"protocol":"socks","settings":{"auth":"noauth","udp":true}}),
        json!({"tag":"velum-http","listen":"127.0.0.1","port":http,"protocol":"http","settings":{}}),
    ]);
    json!({"log":{"loglevel":"none"},"inbounds":inbound,"outbounds":profile.outbounds})
}

fn in_use_by_happ() -> Result<bool, &'static str> {
    unsafe {
        let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if h == INVALID_HANDLE_VALUE {
            return Err("Не удалось проверить, запущен ли Happ.");
        }
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(h, &mut entry) == 0 {
            CloseHandle(h);
            return Err("Не удалось проверить, запущен ли Happ.");
        }
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|&x| x == 0)
                .unwrap_or(entry.szExeFile.len());
            let image = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if image.eq_ignore_ascii_case("happ.exe") || image.eq_ignore_ascii_case("xray.exe") {
                CloseHandle(h);
                return Ok(true);
            }
            if Process32NextW(h, &mut entry) == 0 {
                break;
            }
        }
        CloseHandle(h);
        Ok(false)
    }
}

fn is_admin() -> bool {
    unsafe {
        let mut sid = [0u64; 9];
        let mut len = std::mem::size_of_val(&sid) as u32;
        let sid = sid.as_mut_ptr().cast();
        let mut yes = 0;
        CreateWellKnownSid(WinBuiltinAdministratorsSid, null_mut(), sid, &mut len) != 0
            && CheckTokenMembership(null_mut(), sid, &mut yes) != 0
            && yes != 0
    }
}

fn reserve() -> io::Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
}
fn addr(p: u16) -> SocketAddr {
    (Ipv4Addr::LOCALHOST, p).into()
}

impl Core {
    pub fn start(profile: &Profile, mode: Mode) -> Result<Self, &'static str> {
        if mode == Mode::Tun {
            if in_use_by_happ()? {
                return Err("Happ или другой Xray уже запущен. Выберите «Локальный прокси».");
            }
            if !is_admin() {
                return Err("Для TUN запустите sh1n VPN от имени администратора.");
            }
        }
        let path = exe().map_err(|_| "Не найден core\\xray.exe. Распакуйте архив целиком.")?;
        if mode == Mode::Tun && !path.with_file_name("wintun.dll").is_file() {
            return Err("Не найден core\\wintun.dll.");
        }
        let socks = reserve().map_err(|_| "Не удалось выделить порт SOCKS5.")?;
        let http = reserve().map_err(|_| "Не удалось выделить порт HTTP.")?;
        let sp = socks
            .local_addr()
            .map_err(|_| "Не удалось выделить порт SOCKS5.")?
            .port();
        let hp = http
            .local_addr()
            .map_err(|_| "Не удалось выделить порт HTTP.")?
            .port();
        let cfg = config(profile, mode, sp, hp);
        let mut bytes = serde_json::to_vec(&cfg).map_err(|_| "Ошибка подготовки профиля.")?;
        drop(socks);
        drop(http);
        let child = spawn(&bytes, false);
        bytes.fill(0);
        let (child, job) =
            child.map_err(|_| "Не удалось запустить Xray. Проверьте профиль и наличие ядра.")?;
        let mut result = Self {
            child,
            _job: job,
            socks: sp,
            http: hp,
            mode,
        };
        let end = Instant::now() + Duration::from_secs(7);
        while Instant::now() < end {
            if !result.running() {
                return Err("Xray не запустился. Проверьте совместимость профиля и права доступа.");
            }
            if TcpStream::connect_timeout(&addr(sp), Duration::from_millis(60)).is_ok()
                && TcpStream::connect_timeout(&addr(hp), Duration::from_millis(60)).is_ok()
            {
                thread::sleep(Duration::from_millis(60));
                if result.running() {
                    return Ok(result);
                }
            }
            thread::sleep(Duration::from_millis(40));
        }
        Err("Xray не открыл локальные порты за отведённое время.")
    }

    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        if !self.running() {
            return;
        }
        // Xray owns a private console. Notify it so it can release its
        // adapter, routes and Windows Filtering Platform session.
        signal(self.child.id());
        let end = Instant::now() + Duration::from_millis(1000);
        while self.running() && Instant::now() < end {
            thread::sleep(Duration::from_millis(25));
        }
        if self.running() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn signal(pid: u32) {
    unsafe {
        if AttachConsole(pid) == 0 {
            return;
        }
        let mut ids = [0u32; 3];
        let n = GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32);
        if n == 2 && ids[..2].contains(&pid) && ids[..2].contains(&std::process::id()) {
            SetConsoleCtrlHandler(None, 1);
            GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0);
            thread::sleep(Duration::from_millis(75));
        }
        FreeConsole();
    }
}

fn spawn(config: &[u8], test: bool) -> io::Result<(Child, Job)> {
    let path = exe()?;
    let job = Job(unsafe { CreateJobObjectW(null(), null()) });
    if job.0.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut cmd = Command::new(&path);
    cmd.current_dir(path.parent().ok_or(io::ErrorKind::NotFound)?)
        .args(["run", "-format", "json", "-config", "stdin:"])
        .env("GOMEMLIMIT", "60MiB")
        .env("GOGC", "70")
        .env_remove("XRAY_LOCATION_ASSET")
        .env_remove("XRAY_LOCATION_CONFIG")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if test {
        cmd.arg("-test");
    }
    let mut child = cmd.spawn()?;
    if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle().cast()) } == 0 {
        let err = io::Error::last_os_error();
        let _ = child.kill();
        let _ = child.wait();
        return Err(err);
    }
    child
        .stdin
        .take()
        .ok_or(io::ErrorKind::BrokenPipe)?
        .write_all(config)?;
    Ok((child, job))
}

fn fixture() -> Profile {
    Profile {
        name: "Fixture".into(),
        kind: "freedom".into(),
        outbounds: vec![json!({"protocol":"freedom","tag":"fixture"})],
    }
}

pub fn diagnostic() -> io::Result<()> {
    for mode in [Mode::Proxy, Mode::Tun] {
        let cfg = config(&fixture(), mode, 35171, 35172);
        let (mut child, _job) = spawn(&serde_json::to_vec(&cfg)?, true)?;
        if !child.wait()?.success() {
            return Err(io::Error::other("Invalid Xray configuration"));
        }
    }
    use base64::{engine::general_purpose::STANDARD, Engine};
    let private = STANDARD.encode([1u8; 32]);
    let public = STANDARD.encode([2u8; 32]);
    let wg=format!("[Interface]\nPrivateKey={private}\nAddress=10.12.0.2/32\n[Peer]\nPublicKey={public}\nEndpoint=127.0.0.1:51820\nAllowedIPs=0.0.0.0/0\n");
    let cases = [
        "hy2://fixture@127.0.0.1:443?obfs=salamander&obfs-password=fixture",
        "vmess://11111111-1111-4111-8111-111111111111@127.0.0.1:443?security=tls&type=ws",
        &wg,
    ];
    for text in cases {
        let p = crate::profile::import(text)
            .profiles
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::other("Import fixture rejected"))?;
        let cfg = config(&p, Mode::Proxy, 35171, 35172);
        let (mut child, _job) = spawn(&serde_json::to_vec(&cfg)?, true)?;
        if !child.wait()?.success() {
            return Err(io::Error::other(format!(
                "Invalid {} Xray configuration",
                p.kind
            )));
        }
    }
    Ok(())
}

// End-to-end, offline: local HTTP target -> local VLESS test server -> Velum
// client, using both loopback listeners. Never installs an adapter or routes.
pub fn smoke() -> io::Result<()> {
    let target = reserve()?;
    let target_port = target.local_addr()?.port();
    thread::spawn(move || {
        for _ in 0..2 {
            let Ok((mut conn, _)) = target.accept() else {
                break;
            };
            let _ = conn.set_read_timeout(Some(Duration::from_secs(3)));
            let mut header = Vec::new();
            let mut buf = [0u8; 256];
            while header.len() < 4096 && !header.ends_with(b"\r\n\r\n") {
                match conn.read(&mut buf) {
                    Ok(n) if n > 0 => header.extend_from_slice(&buf[..n]),
                    _ => break,
                }
            }
            let _ = conn.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nVELUM-OK",
            );
        }
    });
    let vless_listener = reserve()?;
    let vless_port = vless_listener.local_addr()?.port();
    drop(vless_listener);
    const ID: &str = "11111111-1111-4111-8111-111111111111";
    let server = json!({"log":{"loglevel":"none"},"inbounds":[{"listen":"127.0.0.1","port":vless_port,
    "protocol":"vless","settings":{"clients":[{"id":ID}],"decryption":"none"}}],
    "outbounds":[{"protocol":"freedom","settings":{"finalRules":[
        {"action":"allow","network":"tcp","ip":["127.0.0.1/32"],"port":target_port.to_string()}
    ]}}]});
    let (mut server, _job) = spawn(&serde_json::to_vec(&server)?, false)?;
    let end = Instant::now() + Duration::from_secs(4);
    while TcpStream::connect_timeout(&addr(vless_port), Duration::from_millis(40)).is_err() {
        if server.try_wait()?.is_some() || Instant::now() >= end {
            return Err(io::Error::other("fixture VLESS server did not start"));
        }
        thread::sleep(Duration::from_millis(30));
    }
    let link = format!("vless://{ID}@127.0.0.1:{vless_port}?encryption=none#Test");
    let profile = crate::profile::import(&link)
        .profiles
        .into_iter()
        .next()
        .ok_or(io::ErrorKind::InvalidData)?;
    let core = Core::start(&profile, Mode::Proxy).map_err(io::Error::other)?;
    let open = |p| -> io::Result<TcpStream> {
        let stream = TcpStream::connect_timeout(&addr(p), Duration::from_secs(2))?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        Ok(stream)
    };
    let check = |mut s: TcpStream, path: String| -> io::Result<()> {
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{target_port}\r\nConnection: close\r\n\r\n"
        )?;
        let mut out = Vec::new();
        let mut buf = [0u8; 512];
        while out.len() < 4096 {
            let n = s.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            if out.ends_with(b"VELUM-OK") {
                return Ok(());
            }
        }
        Err(io::Error::other("unexpected upstream response"))
    };
    let step = |name: &str, e: io::Error| io::Error::other(format!("{name}: {e}"));
    let mut conn = open(core.socks)?;
    conn.write_all(&[5, 1, 0])?;
    let mut hello = [0u8; 2];
    conn.read_exact(&mut hello)
        .map_err(|e| step("SOCKS greeting", e))?;
    if hello != [5, 0] {
        return Err(io::Error::other("SOCKS handshake"));
    }
    let [hi, lo] = target_port.to_be_bytes();
    conn.write_all(&[5, 1, 0, 1, 127, 0, 0, 1, hi, lo])?;
    let mut reply = [0u8; 10];
    conn.read_exact(&mut reply)
        .map_err(|e| step("SOCKS reply", e))?;
    if reply[..4] != [5, 0, 0, 1] {
        return Err(io::Error::other("SOCKS CONNECT"));
    }
    check(conn, "/".into()).map_err(|e| step("SOCKS upstream", e))?;
    check(open(core.http)?, format!("http://127.0.0.1:{target_port}/"))
        .map_err(|e| step("HTTP upstream", e))?;
    println!(
        "HTTP and SOCKS5 through VLESS: OK. App PID {}; client core PID {}.",
        std::process::id(),
        core.child.id()
    );
    // Short stable window for external memory sampling, with sockets verified.
    thread::sleep(Duration::from_secs(4));
    let ports = [core.socks, core.http];
    drop(core);
    if ports
        .into_iter()
        .any(|p| TcpStream::connect_timeout(&addr(p), Duration::from_millis(120)).is_ok())
    {
        return Err(io::Error::other("local proxy did not stop"));
    }
    println!("Both listeners stopped.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loopback_only_and_tun_explicit() {
        let p = config(&fixture(), Mode::Proxy, 10001, 10002);
        assert!(p["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["listen"] == "127.0.0.1"));
        assert_eq!(
            config(&fixture(), Mode::Tun, 10001, 10002)["inbounds"][0]["protocol"],
            "tun"
        );
    }
}
