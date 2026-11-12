use base64::{engine::general_purpose as b64, Engine};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use url::Url;

pub const MAX_INPUT: usize = 4 * 1024 * 1024;
pub const MAX_PROFILES: usize = 512;

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub kind: String,
    pub outbounds: Vec<Value>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    Invalid,
    Unsupported,
    Protected,
    SubscriptionUrl,
}

#[derive(Default)]
pub struct Imported {
    pub profiles: Vec<Profile>,
    pub skipped: usize,
    pub protected: usize,
    pub urls: usize,
}

pub fn import(input: &str) -> Imported {
    let mut result = Imported::default();
    if input.len() > MAX_INPUT {
        result.skipped = 1;
        return result;
    }
    let input = input.trim().trim_start_matches('\u{feff}').trim();
    if hidden(input) {
        result.protected = 1;
        return result;
    }
    if input
        .lines()
        .any(|l| l.trim().eq_ignore_ascii_case("[interface]"))
    {
        match wireguard_conf(input) {
            Ok(p) => result.profiles.push(p),
            Err(_) => result.skipped = 1,
        }
        return result;
    }
    if input.starts_with('{') || input.starts_with('[') {
        let items = match serde_json::from_str::<Value>(input) {
            Ok(Value::Array(a)) => a,
            Ok(v) => vec![v],
            Err(_) => {
                result.skipped = 1;
                return result;
            }
        };
        for item in items {
            if result.profiles.len() == MAX_PROFILES {
                result.skipped += 1;
            } else if let Some(p) = from_json(item) {
                result.profiles.push(p);
            } else {
                result.skipped += 1;
            }
        }
        return result;
    }
    let mut decoded = None;
    if !input.contains("://") && input.len() >= 8 {
        let joined: String = input.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        decoded = decode64(&joined).and_then(|b| String::from_utf8(b).ok());
        if let Some(s) = &decoded {
            if hidden(s) {
                result.protected = 1;
                return result;
            }
            if s.trim_start().starts_with(['{', '[']) {
                return import(s);
            }
        }
    }
    for line in decoded.as_deref().unwrap_or(input).lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if result.profiles.len() == MAX_PROFILES {
            result.skipped += 1;
            continue;
        }
        match from_link(line) {
            Ok(p) => result.profiles.push(p),
            Err(ParseError::Protected) => result.protected += 1,
            Err(ParseError::SubscriptionUrl) => result.urls += 1,
            Err(_) => result.skipped += 1,
        }
    }
    result
}

fn hidden(s: &str) -> bool {
    s.lines().any(|line| {
        let Some((k, v)) = line
            .trim_start()
            .strip_prefix('#')
            .and_then(|l| l.split_once(':'))
        else {
            return false;
        };
        k.trim().eq_ignore_ascii_case("hide-settings")
            && (v.trim() == "1" || v.trim().eq_ignore_ascii_case("true"))
    })
}

fn decode64(s: &str) -> Option<Vec<u8>> {
    [
        b64::STANDARD,
        b64::STANDARD_NO_PAD,
        b64::URL_SAFE,
        b64::URL_SAFE_NO_PAD,
    ]
    .iter()
    .find_map(|e| e.decode(s).ok())
}

fn pct(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

fn label(s: &str, fallback: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(96)
        .collect();
    if clean.trim().is_empty() {
        fallback.to_string()
    } else {
        clean.trim().to_string()
    }
}

fn from_json(mut input: Value) -> Option<Profile> {
    let obj = input.as_object_mut()?;
    let name = obj
        .remove("remarks")
        .or_else(|| obj.remove("name"))
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut outbounds = match obj.remove("outbounds") {
        Some(Value::Array(items)) => items,
        Some(_) => return None,
        None if obj.contains_key("protocol") => vec![input],
        None => return None,
    };
    if outbounds.is_empty() || outbounds.len() > 32 {
        return None;
    }
    let protocol = outbounds[0].get("protocol")?.as_str()?.to_string();
    if !supported(&protocol) {
        return None;
    }
    for outbound in &mut outbounds {
        let o = outbound.as_object_mut()?;
        let p = o.get("protocol")?.as_str()?;
        if !supported(p) && p != "freedom" && p != "blackhole" && p != "dns" {
            return None;
        }
        if has_external_resource(&Value::Object(o.clone())) {
            return None;
        }
        // Never inherit socket binding to a Happ-specific or hardcoded address.
        o.remove("sendThrough");
    }
    Some(Profile {
        name: label(&name, &protocol.to_uppercase()),
        kind: protocol,
        outbounds,
    })
}

fn has_external_resource(v: &Value) -> bool {
    match v {
        Value::Object(o) => o.iter().any(|(k, v)| {
            let key = k.to_ascii_lowercase();
            key.ends_with("file")
                || key == "masterkeylog"
                || key.ends_with("path")
                    && v.as_str()
                        .is_some_and(|s| s.contains(':') || s.contains('\\'))
                || has_external_resource(v)
        }),
        Value::Array(a) => a.iter().any(has_external_resource),
        _ => false,
    }
}

fn supported(s: &str) -> bool {
    matches!(
        s,
        "vless"
            | "vmess"
            | "trojan"
            | "shadowsocks"
            | "socks"
            | "http"
            | "wireguard"
            | "hysteria"
            | "masque"
    )
}

pub fn export(profiles: &[Profile]) -> Option<Vec<u8>> {
    let configs: Vec<_> = profiles
        .iter()
        .map(|p| json!({"remarks":p.name,"outbounds":p.outbounds}))
        .collect();
    serde_json::to_vec(&configs)
        .ok()
        .filter(|s| s.len() <= MAX_INPUT)
}

type Params = BTreeMap<String, String>;
fn param<'a>(p: &'a Params, key: &str) -> &'a str {
    p.get(key).map(String::as_str).unwrap_or("")
}
fn def<'a>(s: &'a str, default: &'a str) -> &'a str {
    if s.is_empty() {
        default
    } else {
        s
    }
}
fn put(obj: &mut Map<String, Value>, key: &str, val: &str) {
    if !val.is_empty() {
        obj.insert(key.into(), json!(val));
    }
}

fn stream(p: &Params, addr: &str) -> Result<Value, ParseError> {
    let n = match param(p, "type") {
        "" | "tcp" | "raw" => "raw",
        "ws" | "grpc" | "httpupgrade" | "xhttp" | "kcp" => param(p, "type"),
        "splithttp" => "xhttp",
        _ => return Err(ParseError::Unsupported),
    };
    let sec = def(param(p, "security"), "none");
    let mut res = json!({"network": n, "security": sec});
    let mut settings = Map::new();
    let host = param(p, "host");
    let path = param(p, "path");
    let field = match n {
        "ws" | "httpupgrade" | "xhttp" => {
            put(&mut settings, "host", host);
            put(&mut settings, "path", path);
            if n == "xhttp" {
                put(&mut settings, "mode", param(p, "mode"));
                if let Some(extra) = p.get("extra") {
                    let x: Value = serde_json::from_str(extra).map_err(|_| ParseError::Invalid)?;
                    if !x.is_object() || has_external_resource(&x) {
                        return Err(ParseError::Invalid);
                    }
                    settings.insert("extra".into(), x);
                }
            }
            match n {
                "ws" => "wsSettings",
                "xhttp" => "xhttpSettings",
                _ => "httpupgradeSettings",
            }
        }
        "grpc" => {
            put(
                &mut settings,
                "serviceName",
                def(param(p, "serviceName"), path),
            );
            put(&mut settings, "authority", def(param(p, "authority"), host));
            if param(p, "mode") == "multi" {
                settings.insert("multiMode".into(), json!(true));
            }
            "grpcSettings"
        }
        "kcp" => {
            put(&mut settings, "seed", def(param(p, "seed"), path));
            if !matches!(param(p, "headerType"), "" | "none") {
                settings.insert("header".into(), json!({"type":param(p,"headerType")}));
            }
            "kcpSettings"
        }
        _ => {
            match param(p, "headerType") {
                "" | "none" => {}
                "http" => {
                    let paths: Vec<_> = def(path, "/").split(',').collect();
                    let hosts: Vec<_> = def(host, addr).split(',').collect();
                    settings.insert(
                        "header".into(),
                        json!({"type":"http","request":{
                            "path":paths, "headers":{"Host":hosts}
                        }}),
                    );
                }
                _ => return Err(ParseError::Unsupported),
            }
            "rawSettings"
        }
    };
    if !settings.is_empty() {
        res[field] = Value::Object(settings);
    }
    let mut s = Map::new();
    match sec {
        "none" => {}
        "tls" => {
            put(
                &mut s,
                "serverName",
                def(
                    param(p, "sni"),
                    def(host.split(',').next().unwrap_or(""), addr),
                ),
            );
            put(&mut s, "fingerprint", param(p, "fp"));
            if !param(p, "alpn").is_empty() {
                s.insert(
                    "alpn".into(),
                    json!(param(p, "alpn").split(',').collect::<Vec<_>>()),
                );
            }
            // An imported link cannot silently disable TLS verification.
            if matches!(param(p, "allowInsecure"), "1" | "true") {
                return Err(ParseError::Unsupported);
            }
            put(&mut s, "pinnedPeerCertSha256", param(p, "pcs"));
            put(&mut s, "echConfigList", param(p, "ech"));
            put(&mut s, "verifyPeerCertByName", param(p, "vcn"));
            res["tlsSettings"] = Value::Object(s);
        }
        "reality" => {
            if param(p, "pbk").is_empty() || param(p, "sni").is_empty() {
                return Err(ParseError::Invalid);
            }
            put(&mut s, "serverName", param(p, "sni"));
            put(&mut s, "fingerprint", def(param(p, "fp"), "chrome"));
            put(&mut s, "password", param(p, "pbk"));
            put(&mut s, "shortId", param(p, "sid"));
            put(&mut s, "spiderX", param(p, "spx"));
            put(&mut s, "mldsa65Verify", param(p, "pqv"));
            res["realitySettings"] = Value::Object(s);
        }
        _ => return Err(ParseError::Unsupported),
    }
    Ok(res)
}

fn from_link(s: &str) -> Result<Profile, ParseError> {
    if s.len() > 96 * 1024 {
        return Err(ParseError::Invalid);
    }
    let (scheme, body) = s.split_once("://").ok_or(ParseError::Invalid)?;
    if scheme.eq_ignore_ascii_case("happ") {
        return Err(ParseError::Protected);
    }
    if matches!(scheme, "http" | "https") {
        return Err(ParseError::SubscriptionUrl);
    }
    if scheme == "v2rayn" {
        return v2rayn(body);
    }
    let (kind, name, settings, st) = match scheme {
        "vmess" if !body.contains('@') => {
            let body = body.split('#').next().unwrap_or(body);
            let bytes = decode64(body).ok_or(ParseError::Invalid)?;
            let j: Value = serde_json::from_slice(&bytes).map_err(|_| ParseError::Invalid)?;
            let get = |k| j.get(k).and_then(Value::as_str).unwrap_or("");
            let num = |k| {
                j.get(k)
                    .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
            };
            let addr = get("add");
            let id = get("id");
            let port = num("port")
                .and_then(|p| u16::try_from(p).ok())
                .filter(|p| *p > 0)
                .ok_or(ParseError::Invalid)?;
            if addr.is_empty() || id.is_empty() {
                return Err(ParseError::Invalid);
            }
            let mut p = Params::new();
            for key in ["host", "path", "sni", "fp", "alpn", "allowInsecure"] {
                p.insert(key.into(), get(key).into());
            }
            p.insert("type".into(), get("net").into());
            p.insert("headerType".into(), get("type").into());
            p.insert("security".into(), get("tls").into());
            let o = json!({"vnext":[{"address":addr,"port":port,"users":[{
                "id":id,"alterId":num("aid").unwrap_or(0),"security":def(get("scy"),"auto")
            }]}]});
            ("vmess", label(get("ps"), "VMess"), o, stream(&p, addr)?)
        }
        "ss" => {
            let (body, frag) = body.split_once('#').unwrap_or((body, ""));
            let (bare, query) = body.split_once('?').unwrap_or((body, ""));
            if url::form_urlencoded::parse(query.as_bytes())
                .any(|(k, v)| k == "plugin" && !v.is_empty())
            {
                return Err(ParseError::Unsupported);
            }
            let decoded;
            let bare = if bare.contains('@') {
                bare
            } else {
                decoded = String::from_utf8(
                    decode64(bare.trim_end_matches('/')).ok_or(ParseError::Invalid)?,
                )
                .map_err(|_| ParseError::Invalid)?;
                &decoded
            };
            let (user, ep) = bare.rsplit_once('@').ok_or(ParseError::Invalid)?;
            let user = pct(user);
            let decoded_user;
            let user = if user.contains(':') {
                &user
            } else {
                decoded_user = String::from_utf8(decode64(&user).ok_or(ParseError::Invalid)?)
                    .map_err(|_| ParseError::Invalid)?;
                &decoded_user
            };
            let (method, password) = user.split_once(':').ok_or(ParseError::Invalid)?;
            if method.is_empty() || password.is_empty() {
                return Err(ParseError::Invalid);
            }
            let u = Url::parse(&format!("ss://{ep}")).map_err(|_| ParseError::Invalid)?;
            let (addr, port) = endpoint(&u)?;
            (
                "shadowsocks",
                label(&pct(frag), "Shadowsocks"),
                json!({"servers":[{"address":addr,"port":port,"method":method,"password":password}]}),
                Value::Null,
            )
        }
        "wireguard" => {
            let u = Url::parse(s).map_err(|_| ParseError::Invalid)?;
            let (host, port) = endpoint(&u)?;
            let q: Params = u.query_pairs().into_owned().collect();
            let ep = format!("{}:{port}", bracket(&host));
            let peer = wg_peer(
                &ep,
                param(&q, "publickey"),
                param(&q, "presharedkey"),
                "0.0.0.0/0,::/0",
                "",
            )?;
            let o = wg_settings(
                &pct(u.username()),
                param(&q, "address"),
                param(&q, "mtu"),
                param(&q, "dns"),
                param(&q, "reserved"),
                vec![peer],
            )?;
            let mut st = Value::Null;
            if let Some(f) = q.get("fm") {
                let m: Value = serde_json::from_str(f).map_err(|_| ParseError::Unsupported)?;
                if !m.is_object() || has_external_resource(&m) {
                    return Err(ParseError::Unsupported);
                }
                st = json!({"finalmask":m});
            }
            (
                "wireguard",
                label(&pct(u.fragment().unwrap_or("")), "WireGuard"),
                o,
                st,
            )
        }
        "hysteria2" | "hy2" => {
            let u = Url::parse(s).map_err(|_| ParseError::Invalid)?;
            let addr = u
                .host_str()
                .filter(|h| !h.is_empty())
                .ok_or(ParseError::Invalid)?
                .trim_matches(['[', ']']);
            let port = u.port().unwrap_or(443);
            if port == 0 {
                return Err(ParseError::Invalid);
            }
            let mut q: Params = u.query_pairs().into_owned().collect();
            if ["insecure", "allowInsecure"]
                .iter()
                .any(|k| matches!(param(&q, k), "1" | "true"))
                || ["pinSHA256", "ports", "mport", "realm"]
                    .iter()
                    .any(|k| !param(&q, k).is_empty())
            {
                return Err(ParseError::Unsupported);
            }
            q.insert("security".into(), "tls".into());
            q.remove("type");
            let mut st = stream(&q, addr)?;
            st["network"] = json!("hysteria");
            let mut auth = pct(u.username());
            if let Some(p) = u.password() {
                auth.push(':');
                auth.push_str(&pct(p));
            }
            st["hysteriaSettings"] = json!({"version":2,"auth":auth});
            match param(&q, "obfs") {
                "" => {}
                "salamander" if !param(&q, "obfs-password").is_empty() => {
                    st["finalmask"] = json!({"udp":[{"type":"salamander","settings":{"password":param(&q,"obfs-password")}}]});
                }
                _ => return Err(ParseError::Unsupported),
            }
            (
                "hysteria",
                label(&pct(u.fragment().unwrap_or("")), "Hysteria2"),
                json!({"version":2,"address":addr,"port":port}),
                st,
            )
        }
        "vless" | "vmess" | "trojan" | "socks" | "socks5" => {
            if matches!(scheme, "socks" | "socks5")
                && !body.split('#').next().unwrap_or("").contains(':')
            {
                let (data, tag) = body.split_once('#').unwrap_or((body, ""));
                let decoded = decode64(data)
                    .and_then(|d| String::from_utf8(d).ok())
                    .ok_or(ParseError::Invalid)?;
                if !decoded.contains('@')
                    || !decoded.contains(':')
                    || decoded.contains(['\n', '\r'])
                {
                    return Err(ParseError::Invalid);
                }
                return from_link(&format!("socks://{decoded}#{tag}"));
            }
            let u = Url::parse(s).map_err(|_| ParseError::Invalid)?;
            let (addr, port) = endpoint(&u)?;
            let pass = pct(u.username());
            let q: Params = u.query_pairs().into_owned().collect();
            let (k, o) = match scheme {
                "vless" => {
                    if pass.is_empty() {
                        return Err(ParseError::Invalid);
                    }
                    (
                        "vless",
                        json!({"vnext":[{"address":addr,"port":port,"users":[{
                            "id":pass,"encryption":def(param(&q,"encryption"),"none"),"flow":param(&q,"flow")
                        }]}]}),
                    )
                }
                "vmess" => {
                    if pass.is_empty() {
                        return Err(ParseError::Invalid);
                    }
                    (
                        "vmess",
                        json!({"vnext":[{"address":addr,"port":port,"users":[{
                            "id":pass,"alterId":0,"security":def(param(&q,"encryption"),"auto")
                        }]}]}),
                    )
                }
                "trojan" => {
                    if pass.is_empty() {
                        return Err(ParseError::Invalid);
                    }
                    (
                        "trojan",
                        json!({"servers":[{"address":addr,"port":port,"password":pass}]}),
                    )
                }
                _ => {
                    let mut srv = json!({"address":addr,"port":port});
                    let decoded = if u.password().is_none() {
                        decode64(&pass).and_then(|b| String::from_utf8(b).ok())
                    } else {
                        None
                    };
                    let literal = pct(u.password().unwrap_or(""));
                    let (username, password) = decoded
                        .as_deref()
                        .and_then(|d| d.split_once(':'))
                        .unwrap_or((&pass, &literal));
                    if !username.is_empty() {
                        srv["users"] = json!([{"user":username,"pass":password}]);
                    }
                    ("socks", json!({"servers":[srv]}))
                }
            };
            let mut q = q;
            if scheme == "trojan" && !q.contains_key("security") {
                q.insert("security".into(), "tls".into());
            }
            (
                k,
                label(&pct(u.fragment().unwrap_or("")), &k.to_uppercase()),
                o,
                stream(&q, &addr)?,
            )
        }
        _ => return Err(ParseError::Unsupported),
    };
    let mut outbound = json!({"protocol":kind,"settings":settings,"tag":"proxy"});
    if !st.is_null() {
        outbound["streamSettings"] = st;
    }
    Ok(Profile {
        name,
        kind: kind.into(),
        outbounds: vec![outbound],
    })
}

fn endpoint(u: &Url) -> Result<(String, u16), ParseError> {
    let host = u
        .host_str()
        .filter(|s| !s.is_empty())
        .ok_or(ParseError::Invalid)?;
    let host = host.trim_matches(['[', ']']);
    let port = u.port().filter(|p| *p > 0).ok_or(ParseError::Invalid)?;
    Ok((host.into(), port))
}

fn bracket(host: &str) -> String {
    let host = host.trim_matches(['[', ']']);
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.into()
    }
}

fn csv(s: &str) -> Vec<&str> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

fn key(s: &str) -> Result<String, ParseError> {
    let b = decode64(s)
        .filter(|b| b.len() == 32)
        .ok_or(ParseError::Invalid)?;
    Ok(b64::STANDARD.encode(b))
}

fn wg_peer(ep: &str, public: &str, psk: &str, ips: &str, keep: &str) -> Result<Value, ParseError> {
    let u = Url::parse(&format!("wireguard://{ep}")).map_err(|_| ParseError::Invalid)?;
    if !u.username().is_empty()
        || !u.path().is_empty()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(ParseError::Invalid);
    }
    let (host, port) = endpoint(&u)?;
    let ips = csv(ips);
    if ips.is_empty() || ips.iter().any(|ip| !valid_ip(ip)) {
        return Err(ParseError::Invalid);
    }
    let mut peer = json!({"endpoint":format!("{}:{port}",bracket(&host)),"publicKey":key(public)?,"allowedIPs":ips});
    if !psk.is_empty() {
        peer["preSharedKey"] = json!(key(psk)?);
    }
    if !keep.is_empty() {
        peer["keepAlive"] = json!(keep.parse::<u16>().map_err(|_| ParseError::Invalid)?);
    }
    Ok(peer)
}

fn valid_ip(s: &str) -> bool {
    let (ip, cidr) = s.split_once('/').map_or((s, None), |(a, b)| (a, Some(b)));
    let Ok(ip) = ip.parse::<std::net::IpAddr>() else {
        return false;
    };
    cidr.is_none_or(|n| {
        n.parse::<u8>()
            .is_ok_and(|n| n <= if ip.is_ipv4() { 32 } else { 128 })
    })
}

fn wg_settings(
    secret: &str,
    addr: &str,
    mtu: &str,
    dns: &str,
    reserved: &str,
    peers: Vec<Value>,
) -> Result<Value, ParseError> {
    let addrs = csv(addr);
    if addrs.is_empty()
        || addrs.iter().any(|ip| !valid_ip(ip))
        || peers.is_empty()
        || peers.len() > 32
    {
        return Err(ParseError::Invalid);
    }
    let mut o = json!({"secretKey":key(secret)?,"address":addrs,"noKernelTun":true,"peers":peers});
    if !mtu.is_empty() {
        let n = mtu
            .parse::<u16>()
            .ok()
            .filter(|n| *n >= 576 && *n <= 9000)
            .ok_or(ParseError::Invalid)?;
        o["mtu"] = json!(n);
    }
    if !dns.is_empty() {
        let servers = csv(dns);
        if servers
            .iter()
            .any(|s| s.parse::<std::net::IpAddr>().is_err())
        {
            return Err(ParseError::Unsupported);
        }
        o["remoteDNS"] = json!(servers);
    }
    if !reserved.is_empty() {
        let n: Vec<u8> = csv(reserved)
            .iter()
            .map(|s| s.parse::<u8>())
            .collect::<Result<_, _>>()
            .map_err(|_| ParseError::Invalid)?;
        if n.len() != 3 {
            return Err(ParseError::Invalid);
        }
        o["reserved"] = json!(n);
    }
    Ok(o)
}

fn wireguard_conf(s: &str) -> Result<Profile, ParseError> {
    let mut intf = Params::new();
    let mut peers: Vec<Params> = Vec::new();
    let mut which = 0;
    for line in s.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        match line.to_ascii_lowercase().as_str() {
            "[interface]" if which == 0 => {
                which = 1;
                continue;
            }
            "[peer]" if which != 0 && peers.len() < 32 => {
                which = 2;
                peers.push(Params::new());
                continue;
            }
            _ => {}
        }
        let (k, v) = line.split_once('=').ok_or(ParseError::Invalid)?;
        let k = k.trim().to_ascii_lowercase();
        let allowed = if which == 1 {
            ["privatekey", "address", "dns", "mtu"].contains(&k.as_str())
        } else if which == 2 {
            [
                "publickey",
                "presharedkey",
                "endpoint",
                "allowedips",
                "persistentkeepalive",
            ]
            .contains(&k.as_str())
        } else {
            false
        };
        if !allowed {
            return Err(ParseError::Unsupported);
        } // Never executes wg-quick commands.
        let map = if which == 1 {
            &mut intf
        } else {
            peers.last_mut().ok_or(ParseError::Invalid)?
        };
        if map.insert(k, v.trim().into()).is_some() {
            return Err(ParseError::Invalid);
        }
    }
    let mut parsed = Vec::new();
    for p in peers {
        parsed.push(wg_peer(
            param(&p, "endpoint"),
            param(&p, "publickey"),
            param(&p, "presharedkey"),
            param(&p, "allowedips"),
            param(&p, "persistentkeepalive"),
        )?);
    }
    let o = wg_settings(
        param(&intf, "privatekey"),
        param(&intf, "address"),
        param(&intf, "mtu"),
        param(&intf, "dns"),
        "",
        parsed,
    )?;
    Ok(Profile {
        name: "WireGuard".into(),
        kind: "wireguard".into(),
        outbounds: vec![json!({"protocol":"wireguard","tag":"proxy","settings":o})],
    })
}

fn v2rayn(s: &str) -> Result<Profile, ParseError> {
    let (kind, raw) = s.split_once('/').ok_or(ParseError::Invalid)?;
    let bytes = decode64(raw).ok_or(ParseError::Invalid)?;
    let j: Value = serde_json::from_slice(&bytes).map_err(|_| ParseError::Invalid)?;
    let get = |k| j.get(k).and_then(Value::as_str).unwrap_or("");
    let extra = |k| {
        j["ProtoExtraObj"]
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
    };
    let tr = |k| {
        j["TransportExtraObj"]
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
    };
    let proto = match (kind, j.get("ConfigType").and_then(Value::as_u64)) {
        ("vmess", Some(1)) => "vmess",
        ("shadowsocks", Some(3)) => "ss",
        ("socks", Some(4)) => "socks",
        ("vless", Some(5)) => "vless",
        ("trojan", Some(6)) => "trojan",
        ("hysteria2", Some(7)) => "hy2",
        ("wireguard", Some(9)) => "wireguard",
        ("http", Some(10)) => "http",
        _ => return Err(ParseError::Unsupported),
    };
    if !get("Cert").is_empty()
        || !get("Finalmask").is_empty()
        || !get("Ports").is_empty()
        || !extra("Hy2RealmUrl").is_empty()
        || !extra("Ports").is_empty()
        || matches!(get("AllowInsecure"), "1" | "true")
    {
        return Err(ParseError::Unsupported);
    }
    let host = get("Address");
    let port = j
        .get("Port")
        .and_then(Value::as_u64)
        .filter(|p| (1..=65535).contains(p))
        .ok_or(ParseError::Invalid)?;
    let mut u = Url::parse(&format!("{proto}://{}:{port}", bracket(host)))
        .map_err(|_| ParseError::Invalid)?;
    if u.host_str().is_none()
        || !u.username().is_empty()
        || u.query().is_some()
        || !matches!(u.path(), "" | "/")
        || u.fragment().is_some()
    {
        return Err(ParseError::Invalid);
    }
    if proto == "http" {
        let mut srv = json!({"address":host,"port":port});
        if !get("Username").is_empty() {
            srv["users"] = json!([{"user":get("Username"),"pass":get("Password")}]);
        }
        let p = Params::from([
            ("security".into(), get("StreamSecurity").into()),
            ("sni".into(), get("Sni").into()),
        ]);
        return Ok(Profile {
            name: label(get("Remarks"), "HTTP"),
            kind: "http".into(),
            outbounds: vec![
                json!({"protocol":"http","tag":"proxy","settings":{"servers":[srv]},"streamSettings":stream(&p,host)?}),
            ],
        });
    }
    let password = def(get("Password"), get("Id"));
    let auth = match proto {
        "ss" => b64::URL_SAFE_NO_PAD.encode(format!(
            "{}:{password}",
            def(extra("SsMethod"), get("Security"))
        )),
        "socks" => get("Username").into(),
        _ => password.into(),
    };
    u.set_username(&auth).map_err(|_| ParseError::Invalid)?;
    if proto == "socks" && !get("Username").is_empty() {
        u.set_password(Some(password))
            .map_err(|_| ParseError::Invalid)?;
    }
    u.set_fragment(Some(get("Remarks")));
    {
        let mut q = u.query_pairs_mut();
        let mut add = |k: &str, v: &str| {
            if !v.is_empty() {
                q.append_pair(k, v);
            }
        };
        for (key, v) in [
            ("type", get("Network")),
            ("security", get("StreamSecurity")),
            ("sni", get("Sni")),
            ("fp", get("Fingerprint")),
            ("alpn", get("Alpn")),
            ("pbk", get("PublicKey")),
            ("sid", get("ShortId")),
            ("spx", get("SpiderX")),
            ("pqv", get("Mldsa65Verify")),
            ("pcs", get("CertSha")),
            ("ech", get("EchConfigList")),
            ("vcn", get("VerifyPeerCertByName")),
            ("host", def(tr("Host"), get("RequestHost"))),
            ("path", def(tr("Path"), get("Path"))),
            (
                "headerType",
                def(
                    tr("RawHeaderType"),
                    def(tr("KcpHeaderType"), get("HeaderType")),
                ),
            ),
            ("serviceName", tr("GrpcServiceName")),
            ("authority", tr("GrpcAuthority")),
            ("seed", tr("KcpSeed")),
            ("mode", def(tr("XhttpMode"), tr("GrpcMode"))),
            ("extra", tr("XhttpExtra")),
            ("flow", def(extra("Flow"), get("Flow"))),
        ] {
            add(key, v);
        }
        if proto == "vmess" {
            add("encryption", def(extra("VmessSecurity"), get("Security")));
        }
        if proto == "vless" {
            add("encryption", extra("VlessEncryption"));
        }
        if proto == "hy2" && !extra("SalamanderPass").is_empty() {
            add("obfs", "salamander");
            add("obfs-password", extra("SalamanderPass"));
        }
        if proto == "wireguard" {
            for (k, v) in [
                ("publickey", "WgPublicKey"),
                ("presharedkey", "WgPresharedKey"),
                ("address", "WgInterfaceAddress"),
                ("reserved", "WgReserved"),
                ("dns", "WgDns"),
            ] {
                add(k, extra(v));
            }
            if let Some(n) = j["ProtoExtraObj"]["WgMtu"].as_u64() {
                add("mtu", &n.to_string());
            }
        }
    }
    from_link(u.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handles_reality_and_ipv6() {
        let s = "vless://11111111-1111-4111-8111-111111111111@[::1]:443?security=reality&pbk=abc&sni=example.invalid&sid=abcd&flow=xtls-rprx-vision#%D0%A2%D0%B5%D1%81%D1%82";
        let r = from_link(s).unwrap();
        assert_eq!(r.name, "Тест");
        assert_eq!(r.outbounds[0]["settings"]["vnext"][0]["address"], "::1");
        assert_eq!(
            r.outbounds[0]["streamSettings"]["realitySettings"]["password"],
            "abc"
        );
    }
    #[test]
    fn base64_subscription_and_shadowsocks() {
        let auth = b64::URL_SAFE_NO_PAD.encode("aes-128-gcm:test-pass");
        let body =
            format!("# title\nss://{auth}@127.0.0.1:1001#One\ntrojan://test@127.0.0.1:443#Two\n");
        let r = import(&b64::STANDARD.encode(body));
        assert_eq!(r.profiles.len(), 2);
        assert_eq!(
            r.profiles[0].outbounds[0]["settings"]["servers"][0]["method"],
            "aes-128-gcm"
        );
        assert_eq!(
            r.profiles[1].outbounds[0]["streamSettings"]["security"],
            "tls"
        );
    }
    #[test]
    fn strips_full_config_inbounds_and_does_not_open_links() {
        let r = import(
            r#"{"remarks":"fixture","inbounds":[{"listen":"0.0.0.0"}],"outbounds":[{"protocol":"socks","settings":{}}]}"#,
        );
        assert_eq!(r.profiles.len(), 1);
        assert_eq!(r.profiles[0].name, "fixture");
        assert_eq!(
            import("happ://crypto-example\nhttps://example.invalid/").protected,
            1
        );
        assert_eq!(import("https://example.invalid/").urls, 1);
        assert_eq!(import(r#"{"outbounds":[{"protocol":"trojan","streamSettings":{"tlsSettings":{"keyFile":"key.pem"}}}]}"#).skipped,1);
    }
    #[test]
    fn wireguard_export_round_trip_and_no_scripts() {
        let private = b64::STANDARD.encode([1u8; 32]);
        let public = b64::STANDARD.encode([2u8; 32]);
        let data=format!("[Interface]\nPrivateKey={private}\nAddress=10.12.0.2/32\nDNS=10.12.0.1\n[Peer]\nPublicKey={public}\nEndpoint=[::1]:51820\nAllowedIPs=0.0.0.0/0\nPersistentKeepalive=25\n");
        let result = import(&data);
        assert_eq!(result.profiles.len(), 1);
        let out = &result.profiles[0].outbounds[0];
        assert_eq!(out["settings"]["peers"][0]["keepAlive"], 25);
        assert_eq!(out["settings"]["remoteDNS"][0], "10.12.0.1");
        let exported = String::from_utf8(export(&result.profiles).unwrap()).unwrap();
        assert_eq!(
            import(&exported).profiles[0].outbounds,
            result.profiles[0].outbounds
        );
        assert_eq!(
            import(&data.replace("[Peer]", "PostUp = fixture\n[Peer]")).skipped,
            1
        );
    }
    #[test]
    fn v2rayn_and_hysteria_public_formats() {
        let payload = json!({"ConfigType":5,"ConfigVersion":4,"Address":"example.invalid","Port":443,
            "Password":"11111111-1111-4111-8111-111111111111","Network":"ws","Remarks":"Fixture",
            "StreamSecurity":"tls","TransportExtraObj":{"Host":"example.invalid","Path":"/ws"}});
        let link = format!(
            "v2rayn://vless/{}",
            b64::URL_SAFE_NO_PAD.encode(payload.to_string())
        );
        let a = from_link(&link).unwrap();
        assert_eq!(
            a.outbounds[0]["streamSettings"]["wsSettings"]["path"],
            "/ws"
        );
        let h =
            from_link("hy2://user:demo@127.0.0.1?obfs=salamander&obfs-password=fixture").unwrap();
        assert_eq!(h.outbounds[0]["settings"]["port"], 443);
        assert_eq!(
            h.outbounds[0]["streamSettings"]["hysteriaSettings"]["auth"],
            "user:demo"
        );
        assert_eq!(import("hy2://fixture@127.0.0.1?insecure=1").skipped, 1);
        assert_eq!(
            import("#hide-settings: true\nsocks://127.0.0.1:1080").protected,
            1
        );
    }
    #[test]
    fn v2rayn_socks_base64_user_info_and_legacy() {
        for link in [
            format!(
                "socks://{}@127.0.0.1:1080",
                b64::URL_SAFE_NO_PAD.encode("tester:fixture:secret")
            ),
            format!(
                "socks://{}",
                b64::URL_SAFE_NO_PAD.encode("tester:fixture:secret@127.0.0.1:1080")
            ),
        ] {
            let p = from_link(&link).unwrap();
            assert_eq!(
                p.outbounds[0]["settings"]["servers"][0]["users"][0]["user"],
                "tester"
            );
            assert_eq!(
                p.outbounds[0]["settings"]["servers"][0]["users"][0]["pass"],
                "fixture:secret"
            );
        }
    }
}
