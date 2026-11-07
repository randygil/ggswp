//! Google Drive: OAuth de aplicación de escritorio (PKCE + redirección a 127.0.0.1) y subida
//! reanudable. Usa el permiso `drive.file`: la app solo ve los archivos que ella misma creó.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPES: &str = "https://www.googleapis.com/auth/drive.file openid email";
const CHUNK: usize = 16 * 1024 * 1024;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

pub struct Authorized {
    pub refresh_token: String,
    pub email: Option<String>,
}

fn random_string(n: usize) -> String {
    use rand::Rng;
    const CS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    (0..n).map(|_| CS[rand::rng().random_range(0..CS.len())] as char).collect()
}

fn email_from_id_token(t: &str) -> Option<String> {
    let payload = t.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("email")?.as_str().map(str::to_string)
}

/// Abre el navegador para que el usuario autorice y espera la redirección local.
pub async fn authorize(client_id: &str, client_secret: &str, open_url: impl FnOnce(&str)) -> Result<Authorized> {
    if client_id.trim().is_empty() {
        bail!("Falta el Client ID de Google (Ajustes → Google Drive)");
    }
    let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| anyhow!("{e}"))?;
    let port = server.server_addr().to_ip().context("puerto local")?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let verifier = random_string(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_string(24);
    let url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={challenge}&code_challenge_method=S256&access_type=offline&prompt=consent&state={state}",
        urlencoding::encode(client_id),
        urlencoding::encode(&redirect),
        urlencoding::encode(SCOPES),
    );
    open_url(&url);

    let code = tokio::task::spawn_blocking(move || -> Result<String> {
        loop {
            let req = server.recv_timeout(Duration::from_secs(300))?.ok_or_else(|| anyhow!("se agotó el tiempo de autorización"))?;
            let q = req.url().split_once('?').map(|(_, q)| q.to_string()).unwrap_or_default();
            let params: std::collections::HashMap<String, String> = q
                .split('&')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.to_string(), urlencoding::decode(v).map(|s| s.into_owned()).unwrap_or_default()))
                .collect();
            if params.get("state") != Some(&state) {
                let _ = req.respond(tiny_http::Response::empty(400));
                continue;
            }
            let ok = params.contains_key("code");
            let msg = if ok { "Listo. Ya puedes volver a ggswp." } else { "No se autorizó el acceso. Puedes cerrar esta pestaña." };
            let html = format!("<html><body style='font-family:system-ui;background:#0d0f14;color:#e8eaf0;display:grid;place-items:center;height:100vh'><h2>{msg}</h2></body></html>");
            let _ = req.respond(tiny_http::Response::from_string(html).with_header(tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()));
            return params.get("code").cloned().ok_or_else(|| anyhow!("Google rechazó la autorización: {}", params.get("error").cloned().unwrap_or_default()));
        }
    })
    .await??;

    let resp: TokenResponse = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", &redirect),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await?
        .error_for_status()
        .context("intercambiando el código")?
        .json()
        .await?;
    Ok(Authorized {
        refresh_token: resp.refresh_token.ok_or_else(|| anyhow!("Google no devolvió refresh_token"))?,
        email: resp.id_token.as_deref().and_then(email_from_id_token),
    })
}

pub struct Drive {
    http: reqwest::Client,
    token: String,
}

impl Drive {
    pub async fn connect(client_id: &str, client_secret: &str, refresh_token: &str) -> Result<Self> {
        let http = reqwest::Client::new();
        let resp: TokenResponse = http
            .post(TOKEN_URL)
            .form(&[("client_id", client_id), ("client_secret", client_secret), ("refresh_token", refresh_token), ("grant_type", "refresh_token")])
            .send()
            .await?
            .error_for_status()
            .context("renovando el acceso a Google Drive (vuelve a conectar la cuenta)")?
            .json()
            .await?;
        Ok(Self { http, token: resp.access_token })
    }

    pub async fn ensure_folder(&self, existing: Option<&str>, name: &str) -> Result<String> {
        if let Some(id) = existing {
            let r = self.http.get(format!("https://www.googleapis.com/drive/v3/files/{id}?fields=id,trashed")).bearer_auth(&self.token).send().await?;
            if r.status().is_success() {
                let v: serde_json::Value = r.json().await?;
                if v["trashed"] != serde_json::Value::Bool(true) {
                    return Ok(id.to_string());
                }
            }
        }
        let v: serde_json::Value = self
            .http
            .post("https://www.googleapis.com/drive/v3/files?fields=id")
            .bearer_auth(&self.token)
            .json(&serde_json::json!({"name": name, "mimeType": "application/vnd.google-apps.folder"}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        v["id"].as_str().map(str::to_string).ok_or_else(|| anyhow!("Drive no devolvió el id de la carpeta"))
    }

    /// Subida reanudable en bloques de 16 MB. `progress(bytes_enviados, total)`.
    /// `paused()` se consulta antes de cada bloque: mientras devuelva true, la subida espera
    /// (la sesión reanudable de Drive dura días, así que se puede retomar sin perder nada).
    pub async fn upload(&self, path: &Path, name: &str, folder: &str, mut progress: impl FnMut(u64, u64), paused: impl Fn() -> bool) -> Result<String> {
        let total = tokio::fs::metadata(path).await?.len();
        let session = self
            .http
            .post("https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable&fields=id")
            .bearer_auth(&self.token)
            .header("X-Upload-Content-Type", "video/mp4")
            .header("X-Upload-Content-Length", total.to_string())
            .json(&serde_json::json!({"name": name, "parents": [folder]}))
            .send()
            .await?
            .error_for_status()?
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("Drive no abrió la sesión de subida"))?;

        let mut file = tokio::fs::File::open(path).await?;
        let mut sent: u64 = 0;
        let mut buf = vec![0u8; CHUNK];
        loop {
            while paused() {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            let mut n = 0;
            while n < CHUNK {
                let r = file.read(&mut buf[n..]).await?;
                if r == 0 {
                    break;
                }
                n += r;
            }
            let end = sent + n as u64;
            let range = if total == 0 { "bytes */0".to_string() } else { format!("bytes {sent}-{}/{total}", end - 1) };
            let mut attempt = 0;
            let resp = loop {
                let r = self.http.put(&session).header("Content-Range", &range).body(buf[..n].to_vec()).send().await;
                match r {
                    Ok(r) if r.status().is_server_error() && attempt < 5 => {}
                    Ok(r) => break r,
                    Err(_) if attempt < 5 => {}
                    Err(e) => return Err(e.into()),
                }
                attempt += 1;
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
            };
            sent = end;
            progress(sent, total);
            match resp.status().as_u16() {
                308 => continue,
                200 | 201 => {
                    let v: serde_json::Value = resp.json().await?;
                    return v["id"].as_str().map(str::to_string).ok_or_else(|| anyhow!("Drive no devolvió el id"));
                }
                s => bail!("Drive respondió {s}: {}", resp.text().await.unwrap_or_default()),
            }
        }
    }

    /// Tamaño en bytes de un archivo de Drive (`None` si no existe o está en la papelera).
    pub async fn file_size(&self, file_id: &str) -> Result<Option<u64>> {
        let r = self.http.get(format!("https://www.googleapis.com/drive/v3/files/{file_id}?fields=size,trashed")).bearer_auth(&self.token).send().await?;
        if r.status().as_u16() == 404 {
            return Ok(None);
        }
        let v: serde_json::Value = r.error_for_status()?.json().await?;
        if v["trashed"] == serde_json::Value::Bool(true) {
            return Ok(None);
        }
        Ok(v["size"].as_str().and_then(|s| s.parse().ok()))
    }

    /// Cambia metadatos (nombre, estrella, papelera). Un archivo que ya no existe no es error.
    pub async fn patch(&self, file_id: &str, body: serde_json::Value) -> Result<()> {
        let r = self.http.patch(format!("https://www.googleapis.com/drive/v3/files/{file_id}?fields=id")).bearer_auth(&self.token).json(&body).send().await?;
        if r.status().as_u16() == 404 {
            return Ok(());
        }
        r.error_for_status().context("actualizando el archivo en Drive")?;
        Ok(())
    }

    /// Descarga un archivo a `path` (primero a un `.part`, para no dejar archivos a medias).
    pub async fn download(&self, file_id: &str, path: &Path, mut progress: impl FnMut(u64, u64)) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut r = self
            .http
            .get(format!("https://www.googleapis.com/drive/v3/files/{file_id}?alt=media"))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()
            .context("descargando de Drive")?;
        let total = r.content_length().unwrap_or(0);
        let part = path.with_extension("part");
        let mut out = tokio::fs::File::create(&part).await?;
        let mut got: u64 = 0;
        while let Some(chunk) = r.chunk().await? {
            out.write_all(&chunk).await?;
            got += chunk.len() as u64;
            progress(got, total);
        }
        out.flush().await?;
        drop(out);
        if total != 0 && got != total {
            let _ = tokio::fs::remove_file(&part).await;
            bail!("la descarga se cortó ({got} de {total} bytes)");
        }
        tokio::fs::rename(&part, path).await?;
        Ok(())
    }

    /// Hace el archivo visible para cualquiera con el enlace y devuelve el link.
    pub async fn make_public_link(&self, file_id: &str) -> Result<String> {
        self.http
            .post(format!("https://www.googleapis.com/drive/v3/files/{file_id}/permissions"))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({"role": "reader", "type": "anyone"}))
            .send()
            .await?
            .error_for_status()?;
        Ok(format!("https://drive.google.com/file/d/{file_id}/view"))
    }
}
