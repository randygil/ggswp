//! Compartir al teléfono por Wi-Fi: servidor HTTP local + QR. El teléfono abre una página
//! con el video (soporta Range, necesario en iOS) y un botón de descarga.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
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

    pub fn share(&self, path: PathBuf, title: String) -> Result<ShareLink> {
        use rand::Rng;
        let token: String = (0..16).map(|_| format!("{:x}", rand::rng().random_range(0..16u8))).collect();
        let ip = local_ip_address::local_ip().context("no se encontró la IP de la red local")?;
        let url = format!("http://{ip}:{}/s/{token}", self.port);
        let mut map = self.entries.lock().unwrap();
        map.retain(|_, e| e.created.elapsed() < TTL);
        map.insert(token, Entry { path, title, created: Instant::now() });
        Ok(ShareLink { qr_svg: qr_svg(&url)?, url })
    }
}

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
<a href="/f/{token}?dl=1"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="M7 10l5 5 5-5"/><path d="M12 15V3"/></svg>Descargar video</a>
<p class="note">El enlace funciona en tu misma red Wi-Fi y vence en 24 horas.</p>
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
        let safe: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        headers.push(header("Content-Disposition", &format!("attachment; filename=\"{safe}.mp4\"")));
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
