//! Compartir al teléfono por Wi-Fi: servidor HTTP local + QR. El teléfono abre una página
//! con el video (soporta Range, necesario en iOS) y un botón de descarga.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::net::Ipv4Addr;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use qrcode::QrCode;
use qrcode::render::svg;
use serde::Serialize;
use tiny_http::{Header, Request, Response, Server, StatusCode};

const TTL: Duration = Duration::from_secs(24 * 3600);

struct Entry {
    path: PathBuf,
    title: String,
    created: Instant,
}

#[derive(Clone)]
pub struct ShareServer {
    port: u16,
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

#[derive(Serialize)]
pub struct ShareLink {
    pub url: String,
    pub qr_svg: String,
    /// Direcciones de esta PC en la red local (solo en el enlace por Wi-Fi); la primera es
    /// la del enlace si no se pidió otra.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<LanAddress>,
}

/// Una dirección IPv4 de un adaptador de red físico (cable o Wi-Fi).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LanAddress {
    pub ip: String,
    /// Nombre del adaptador en Windows ("Ethernet", "Wi-Fi").
    pub name: String,
    /// Tiene puerta de enlace: es la red del router, no una red virtual.
    pub gateway: bool,
}

/// Deja las candidatas útiles, primero los rangos de red doméstica.
fn rank(mut v: Vec<LanAddress>) -> Vec<LanAddress> {
    let home = |ip: &str| match ip.parse::<Ipv4Addr>().map(|a| a.octets()) {
        Ok([192, 168, ..]) => 0,
        Ok([10, ..]) => 1,
        Ok([172, b, ..]) if (16..32).contains(&b) => 2,
        _ => 3,
    };
    v.sort_by_key(|a| (!a.gateway, home(&a.ip)));
    // Las redes sin puerta de enlace (conmutadores virtuales) solo sirven si no hay otra.
    if v.first().is_some_and(|a| a.gateway) {
        v.retain(|a| a.gateway);
    }
    v
}

/// Direcciones por las que un teléfono de la misma red puede llegar a esta PC. Las VPN
/// (Windscribe, Tailscale…) no son adaptadores de cable ni Wi-Fi y quedan afuera: con una
/// VPN activa, la ruta por defecto de Windows apunta a la VPN y no a la red de la casa.
pub fn lan_addresses() -> Vec<LanAddress> {
    use windows::Win32::NetworkManagement::IpHelper::{GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IF_TYPE_ETHERNET_CSMACD, IF_TYPE_IEEE80211, IP_ADAPTER_ADDRESSES_LH};
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size = 32 * 1024u32;
    let mut out = Vec::new();
    for _ in 0..3 {
        // u64 para que el búfer quede alineado como la estructura.
        let mut buf = vec![0u64; size as usize / 8 + 1];
        let head = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        // SAFETY: `buf` tiene al menos `size` bytes y vive mientras se recorre la lista.
        let rc = unsafe { GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(head), &mut size) };
        if rc == 111 {
            // ERROR_BUFFER_OVERFLOW: `size` trae el tamaño necesario.
            continue;
        }
        if rc != 0 {
            break;
        }
        let mut p = head as *const IP_ADAPTER_ADDRESSES_LH;
        while !p.is_null() {
            // SAFETY: Windows llenó la lista enlazada dentro de `buf`.
            let a = unsafe { &*p };
            p = a.Next;
            if a.OperStatus != IfOperStatusUp || !(a.IfType == IF_TYPE_ETHERNET_CSMACD || a.IfType == IF_TYPE_IEEE80211) {
                continue;
            }
            let name = unsafe { a.FriendlyName.to_string() }.unwrap_or_default();
            let mut u = a.FirstUnicastAddress;
            while !u.is_null() {
                let ua = unsafe { &*u };
                u = ua.Next;
                let sa = ua.Address.lpSockaddr as *const SOCKADDR_IN;
                if sa.is_null() || unsafe { (*sa).sin_family } != AF_INET {
                    continue;
                }
                let ip = Ipv4Addr::from(u32::from_be(unsafe { (*sa).sin_addr.S_un.S_addr }));
                if !ip.is_link_local() && !ip.is_loopback() {
                    out.push(LanAddress { ip: ip.to_string(), name: name.clone(), gateway: !a.FirstGatewayAddress.is_null() });
                }
            }
        }
        break;
    }
    rank(out)
}

pub fn qr_svg(data: &str) -> Result<String> {
    let code = QrCode::new(data.as_bytes())?;
    Ok(code.render::<svg::Color>().min_dimensions(220, 220).quiet_zone(true).dark_color(svg::Color("#000")).light_color(svg::Color("#fff")).build())
}

impl ShareServer {
    pub fn start(port: u16) -> Result<Self> {
        let server = Server::http(("0.0.0.0", port)).map_err(|e| anyhow::anyhow!("puerto {port}: {e}"))?;
        let me = Self { port, entries: Arc::new(Mutex::new(HashMap::new())) };
        let entries = me.entries.clone();
        let server = Arc::new(server);
        for _ in 0..4 {
            let server = server.clone();
            let entries = entries.clone();
            std::thread::spawn(move || {
                for req in server.incoming_requests() {
                    if let Err(e) = handle(req, &entries) {
                        log::debug!("share: {e:#}");
                    }
                }
            });
        }
        Ok(me)
    }

    /// Publica el archivo y arma el enlace con `ip` (una de `lan_addresses`) o, si no se
    /// pide ninguna, con la mejor candidata.
    pub fn share(&self, path: PathBuf, title: String, ip: Option<&str>) -> Result<ShareLink> {
        use rand::Rng;
        let token: String = (0..16).map(|_| format!("{:x}", rand::rng().random_range(0..16u8))).collect();
        let mut addresses = lan_addresses();
        if addresses.is_empty() {
            // Sin adaptador de cable o Wi-Fi reconocible: lo que Windows use para salir.
            let ip = local_ip_address::local_ip().context("Esta PC no está conectada a una red local")?;
            addresses.push(LanAddress { ip: ip.to_string(), name: String::new(), gateway: true });
        }
        let ip = match ip {
            Some(ip) => &addresses.iter().find(|a| a.ip == ip).context("Esa dirección ya no está disponible")?.ip,
            None => &addresses[0].ip,
        };
        let url = format!("http://{ip}:{}/s/{token}", self.port);
        let mut map = self.entries.lock().unwrap();
        map.retain(|_, e| e.created.elapsed() < TTL);
        map.insert(token, Entry { path, title, created: Instant::now() });
        Ok(ShareLink { qr_svg: qr_svg(&url)?, url, addresses })
    }
}

// ---------- firewall de Windows ----------

/// Si el firewall de Windows deja entrar conexiones a esta app desde la red local.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Firewall {
    Allowed,
    /// Hay una regla que bloquea la app (queda así si se cancela el aviso de Windows).
    Blocked,
    /// Sin regla para la app: Windows pregunta la primera vez, o bloquea sin avisar.
    Unknown,
}

fn powershell(script: &str) -> Result<std::process::Output> {
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .creation_flags(crate::capture::video::CREATE_NO_WINDOW)
        .output()
        .context("no se pudo ejecutar PowerShell")
}

/// Texto entre comillas simples de PowerShell.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn exe_path() -> Result<String> {
    Ok(std::env::current_exe()?.display().to_string())
}

/// Decide con las reglas de entrada de la app (`acción|perfiles`) y la categoría de la red
/// (`Public`, `Private`, `DomainAuthenticated`). Una regla de bloqueo le gana a una de permiso.
fn firewall_verdict(rules: &[(String, String)], category: Option<&str>) -> Firewall {
    let applies = |profiles: &str| match category {
        _ if profiles == "Any" => true,
        Some("DomainAuthenticated") => profiles.contains("Domain"),
        Some(c) => profiles.contains(c),
        None => true,
    };
    let has = |action: &str| rules.iter().any(|(a, p)| a == action && applies(p));
    if has("Block") {
        Firewall::Blocked
    } else if has("Allow") {
        Firewall::Allowed
    } else {
        Firewall::Unknown
    }
}

/// Estado del firewall para esta app en la red del adaptador `adapter`.
pub fn firewall_state(adapter: Option<&str>) -> Result<Firewall> {
    let script = format!(
        "$ErrorActionPreference='SilentlyContinue'; $exe={}; \
         Get-NetConnectionProfile | ForEach-Object {{ \"net=$($_.InterfaceAlias)|$($_.NetworkCategory)\" }}; \
         Get-NetFirewallApplicationFilter | Where-Object {{ $_.Program -eq $exe }} | Get-NetFirewallRule | \
         Where-Object {{ \"$($_.Enabled)\" -eq 'True' -and \"$($_.Direction)\" -eq 'Inbound' }} | ForEach-Object {{ \"rule=$($_.Action)|$($_.Profile)\" }}",
        ps_quote(&exe_path()?)
    );
    let out = powershell(&script)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let pairs = |prefix: &str| -> Vec<(String, String)> {
        text.lines().filter_map(|l| l.trim().strip_prefix(prefix)).filter_map(|l| l.split_once('|')).map(|(a, b)| (a.to_string(), b.to_string())).collect()
    };
    let nets = pairs("net=");
    let category = adapter.and_then(|n| nets.iter().find(|(alias, _)| alias == n)).map(|(_, c)| c.as_str());
    Ok(firewall_verdict(&pairs("rule="), category))
}

/// Reemplaza las reglas de entrada de la app por una que permite conexiones TCP desde la
/// red local. Cambiar el firewall pide permiso de administrador (aviso de Windows).
pub fn firewall_allow() -> Result<()> {
    use base64::Engine;
    let utf16: Vec<u8> = firewall_allow_script(&exe_path()?).encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let outer = format!(
        "try {{ $p = Start-Process powershell -Verb RunAs -WindowStyle Hidden -Wait -PassThru \
         -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand','{encoded}'; exit $p.ExitCode }} catch {{ exit 1223 }}"
    );
    match powershell(&outer)?.status.code() {
        Some(0) => Ok(()),
        Some(1223) => bail!("No se dio el permiso de administrador"),
        code => bail!("No se pudo cambiar el firewall (código {code:?})"),
    }
}

/// Lo que corre con permiso de administrador.
fn firewall_allow_script(exe: &str) -> String {
    format!(
        "$exe={}; Get-NetFirewallApplicationFilter | Where-Object {{ $_.Program -eq $exe }} | Get-NetFirewallRule | \
         Where-Object {{ \"$($_.Direction)\" -eq 'Inbound' }} | Remove-NetFirewallRule; \
         New-NetFirewallRule -DisplayName 'ggswp (compartir por Wi-Fi)' -Direction Inbound -Action Allow -Program $exe \
         -Protocol TCP -RemoteAddress LocalSubnet -Profile Any -ErrorAction Stop | Out-Null",
        ps_quote(exe)
    )
}

// ---------- servidor ----------

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn handle(req: Request, entries: &Mutex<HashMap<String, Entry>>) -> Result<()> {
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    let mut parts = path.trim_start_matches('/').splitn(2, '/');
    let (kind, token) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let found = {
        let map = entries.lock().unwrap();
        map.get(token).filter(|e| e.created.elapsed() < TTL).map(|e| (e.path.clone(), e.title.clone()))
    };
    let Some((file, title)) = found else {
        return Ok(req.respond(Response::from_string("Enlace vencido").with_status_code(404))?);
    };
    match kind {
        "s" => {
            let t = html_escape(&title);
            let size = format!("{:.0} MB", (std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0) as f64 / 1e6).max(1.0));
            // Misma identidad que la app: negro, acento ácido, títulos condensados. Las fuentes de Google
            // son opcionales; sin internet cae a una condensada del sistema.
            let page = format!(
                r##"<!doctype html><html lang="es"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="theme-color" content="#07080a"><title>{t} · ggswp</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Archivo:wdth,wght@62..125,400..900&family=JetBrains+Mono:wght@400;500&display=swap">
<style>
*{{box-sizing:border-box}}
body{{margin:0;background:#07080a;color:#f2f4f1;font-family:Archivo,system-ui,sans-serif;-webkit-font-smoothing:antialiased}}
main{{max-width:720px;min-height:100vh;margin:0 auto;padding:28px 20px 32px;display:flex;flex-direction:column;gap:24px}}
.brand{{display:flex;align-items:center;gap:12px}}
.mark{{width:12px;height:12px;background:#39ff88;transform:rotate(45deg);margin:0 3px}}
.name{{font-size:26px;font-weight:800;font-stretch:62%;line-height:1}}
.from,.note,.tag{{font-family:"JetBrains Mono",ui-monospace,monospace}}
.from{{margin-left:auto;font-size:11px;letter-spacing:.08em;color:#8a918d}}
video{{width:100%;aspect-ratio:16/9;border-radius:4px;background:#0b0d10;border:1px solid rgba(255,255,255,.08);display:block}}
.tag{{align-self:flex-start;font-size:11px;letter-spacing:.08em;padding:3px 7px;border-radius:2px;background:#39ff88;color:#07080a}}
h1{{margin:0;font-size:clamp(48px,14vw,84px);font-weight:900;font-stretch:62%;text-transform:uppercase;line-height:.85;overflow-wrap:anywhere;
font-family:Archivo,"Arial Narrow","Roboto Condensed",sans-serif-condensed,sans-serif}}
.cta{{margin-top:auto;display:flex;flex-direction:column;gap:12px}}
a{{display:flex;align-items:center;justify-content:center;gap:10px;min-height:56px;border-radius:4px;background:#39ff88;color:#07080a;
font-size:17px;font-weight:700;text-decoration:none}}
a:active{{transform:scale(.98)}}
.note{{margin:0;text-align:center;font-size:12px;line-height:1.6;color:#8a918d}}
</style></head>
<body><main>
<div class="brand"><span class="mark"></span><span class="name">ggswp</span><span class="from">DESDE TU PC</span></div>
<video src="/f/{token}" controls playsinline preload="metadata"></video>
<span class="tag">VIDEO</span>
<h1>{t}</h1>
<div class="cta">
<a href="/f/{token}?dl=1"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="M7 10l5 5 5-5"/><path d="M12 15V3"/></svg>Descargar video · {size}</a>
<p class="note">Queda guardado en el teléfono: desde ahí lo compartes por WhatsApp, Instagram o donde quieras. El enlace funciona en tu misma red Wi-Fi y vence en 24 horas.</p>
</div>
</main></body></html>"##
            );
            req.respond(Response::from_string(page).with_header(header("Content-Type", "text/html; charset=utf-8")))?;
        }
        "f" => serve_file(req, &file, &title, query.contains("dl=1"))?,
        _ => req.respond(Response::empty(404))?,
    }
    Ok(())
}

/// Cabecera de descarga con el título como nombre de archivo. Las cabeceras HTTP solo
/// admiten ASCII: el nombre con tildes va aparte, codificado (RFC 5987).
fn content_disposition(title: &str) -> String {
    let safe: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let ascii: String = safe.chars().map(|c| if c.is_ascii() { c } else { '_' }).collect();
    format!("attachment; filename=\"{ascii}.mp4\"; filename*=UTF-8''{}.mp4", urlencoding::encode(&safe))
}

fn serve_file(req: Request, path: &PathBuf, title: &str, download: bool) -> Result<()> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let range = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .and_then(|h| h.value.as_str().strip_prefix("bytes=").map(str::to_string))
        .and_then(|r| {
            let (a, b) = r.split_once('-')?;
            let start: u64 = a.parse().ok()?;
            let end: u64 = if b.is_empty() { len - 1 } else { b.parse().ok()? };
            (start <= end && end < len).then_some((start, end))
        });
    let mut headers = vec![header("Content-Type", "video/mp4"), header("Accept-Ranges", "bytes")];
    if download {
        headers.push(header("Content-Disposition", &content_disposition(title)));
    }
    let (status, start, count) = match range {
        Some((s, e)) => {
            headers.push(header("Content-Range", &format!("bytes {s}-{e}/{len}")));
            (206, s, e - s + 1)
        }
        None => (200, 0, len),
    };
    f.seek(SeekFrom::Start(start))?;
    let reader = f.take(count);
    let resp = Response::new(StatusCode(status), headers, reader, Some(count as usize), None);
    req.respond(resp)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(ip: &str, gateway: bool) -> LanAddress {
        LanAddress { ip: ip.into(), name: String::new(), gateway }
    }

    #[test]
    fn the_router_network_goes_first() {
        // Conmutador virtual de Hyper-V (sin puerta de enlace) y la red de la casa.
        let v = rank(vec![addr("172.20.112.1", false), addr("10.0.0.5", true), addr("192.168.7.200", true)]);
        assert_eq!(v.iter().map(|a| a.ip.as_str()).collect::<Vec<_>>(), ["192.168.7.200", "10.0.0.5"]);
        assert_eq!(rank(vec![addr("172.20.112.1", false)]).len(), 1);
    }

    #[test]
    fn a_block_rule_wins_and_profiles_must_match_the_network() {
        let rule = |a: &str, p: &str| (a.to_string(), p.to_string());
        assert_eq!(firewall_verdict(&[rule("Block", "Private, Public")], Some("Public")), Firewall::Blocked);
        assert_eq!(firewall_verdict(&[rule("Allow", "Any"), rule("Block", "Public")], Some("Public")), Firewall::Blocked);
        // Permitida solo en redes privadas, pero la red está marcada como pública.
        assert_eq!(firewall_verdict(&[rule("Allow", "Private")], Some("Public")), Firewall::Unknown);
        assert_eq!(firewall_verdict(&[rule("Allow", "Private")], Some("Private")), Firewall::Allowed);
        assert_eq!(firewall_verdict(&[rule("Allow", "Domain")], Some("DomainAuthenticated")), Firewall::Allowed);
        assert_eq!(firewall_verdict(&[], Some("Private")), Firewall::Unknown);
    }

    #[test]
    fn download_names_with_accents_are_valid_headers() {
        let v = content_disposition("Doble eliminación · ×2");
        assert_eq!(v, "attachment; filename=\"Doble eliminaci_n _ _2.mp4\"; filename*=UTF-8''Doble%20eliminaci%C3%B3n%20_%20_2.mp4");
        assert!(Header::from_bytes("Content-Disposition".as_bytes(), v.as_bytes()).is_ok());
    }

    #[test]
    fn powershell_quoting() {
        assert_eq!(ps_quote(r"C:\Users\O'Brien\ggswp.exe"), r"'C:\Users\O''Brien\ggswp.exe'");
    }

    /// PowerShell analiza el script del firewall sin ejecutarlo.
    #[test]
    #[ignore]
    fn firewall_script_parses() {
        let script = firewall_allow_script(r"C:\Users\O'Brien\AppData\Local\ggswp\ggswp.exe");
        let check = format!("$e = $null; [void][System.Management.Automation.Language.Parser]::ParseInput({}, [ref]$null, [ref]$e); $e.Count; $e", ps_quote(&script));
        let out = powershell(&check).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "0", "{script}");
    }

    /// En esta PC: `cargo test --lib share -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn this_pc() {
        println!("{:#?}", lan_addresses());
        println!("firewall: {:?}", firewall_state(lan_addresses().first().map(|a| a.name.as_str())));
    }
}
