# ggswp

Grabador automático de partidas de Fortnite para Windows. Detecta cuándo empieza y termina
cada partida, graba la pantalla (lo que ves tú) con el audio, marca tus eliminaciones en la
línea de tiempo, genera clips y te deja editarlos, enviarlos al teléfono o subirlos a Google Drive.

Una sola app de escritorio (Tauri + React), sin OBS: la captura va integrada.

**Descarga:** el instalador está en [Releases](https://github.com/randygil/ggswp/releases).
Una vez instalada, la app avisa sola cuando hay una versión nueva.

## Qué hace

- **Graba sola** cada partida de Fortnite y la guarda en la galería, con el mapa de la partida como
  una "firma" de eventos.
- **Eliminaciones, derribos, caída y victoria** exactos desde el replay de Fortnite.
- **Cofres abiertos**: lee los avisos de experiencia ("+600 PE · COFRE") en el video con el OCR de Windows.
- **Estadísticas**: puesto final, daño hecho y recibido, precisión, distancia; un histórico que no
  baja cuando las partidas vencen y una página con gráficos partida a partida.
- **Clips automáticos** de cada eliminación, clips con atajo (últimos 30 s) y editor con línea de tiempo.
- **Resumen de la partida** en un solo video y **versión vertical 9:16** (TikTok / Shorts / Reels)
  con tu contador de eliminaciones.
- **Google Drive**: sube solo cuando no estás jugando, con una copia comprimida (HEVC con NVENC,
  unas 4 veces más liviana). Lo que ya está en Drive se borra de la PC a los 7 días y del todo a
  los 15 (las partidas conservadas no vencen). Todo es configurable.
- **Clips livianos**: se guardan en H.264 a 1080p (cerca de 1 MB por segundo) en vez de a la
  calidad de grabación, listos para WhatsApp e Instagram. El editor exporta en horizontal,
  vertical 9:16 o **modo móvil** (9:16 con la salud y las armas en una franja arriba; las zonas
  se ajustan arrastrando sobre el video, con vista previa en vivo) y en tres calidades.
- **Compartir**: copiar el video al portapapeles, panel "Compartir" de Windows, guardar como,
  al teléfono por Wi-Fi o Drive con un QR. Menú de highlights sobre el juego y menús de clic derecho.

## Cómo funciona

| Pieza | Técnica |
|---|---|
| Detección de partida | Lee `%LOCALAPPDATA%\FortniteGame\Saved\Logs\FortniteGame.log` en vivo: `LoadMap` a un servidor → carga, `MatchState … InProgress` → empieza, `EAthenaGamePhase::Aircraft` → autobús, `Match finished for … end game` → te eliminaron o ganaste, `LoadMap … Frontend` → lobby. |
| Video | Desktop Duplication (`ddagrab`) del monitor donde está Fortnite → NVENC (H.264/HEVC/AV1). Los frames no salen de la GPU. |
| Audio | WASAPI loopback (lo que suena en tu salida) y micrófono opcional. Cada paquete se ubica por su timestamp QPC, el mismo reloj del video, así la sincronía es exacta. |
| Eliminaciones | Al terminar, lee el `.replay` de `Saved\Demos` (eventos `playerElim`, cifrados AES-256 con la clave de la cabecera): quién eliminó a quién, derribos, distancia y momento exacto. Si no hay replay, usa los avisos de highlight que Fortnite le manda a NVIDIA (`HLSetVideo`). |
| Anti-cheat | No se inyecta nada en el juego ni se lee su memoria: solo archivos que Fortnite escribe en disco (lo mismo que hace Medal). |

## Requisitos

- Windows 10/11 con GPU NVIDIA (NVENC).
- En Fortnite: **Ajustes → Juego → Repeticiones activadas** (para detectar eliminaciones) y
  **Modo de ventana: Ventana sin bordes** si quieres ver los avisos y el menú sobre el juego.
- Para desarrollar: Node 20+, Rust estable y ffmpeg con `ddagrab` (6.0 o superior).

## Uso

La app queda en la bandeja del sistema. Cuando entras a una partida empieza a grabar sola y,
al terminar, procesa el video: eventos, miniatura y clips automáticos de cada eliminación
(las eliminaciones seguidas se juntan en un clip: "Doble eliminación", "Victoria magistral"…).

### Atajos globales (se cambian en Ajustes)

| Atajo | Acción |
|---|---|
| `Alt+F8` | Guardar los últimos 30 s (dentro de partida o, con el búfer activo, en cualquier momento con Fortnite abierto) |
| `Alt+F7` | Marcar el momento en la línea de tiempo |
| `Alt+F11` | Iniciar / detener grabación manual |
| `Alt+Shift+G` | Menú de highlights sobre el juego |

Por defecto no se usan `Alt+F9`/`Alt+F10`/`Alt+Z` porque son los de NVIDIA.

### Editor

En la vista de partida: `I` / `O` marcan inicio y fin, se arrastran los bordes en la línea de
tiempo y **Exportar clip** corta exacto al frame (recodifica con NVENC). `←`/`→` 5 s,
`Shift+←/→` 1 s, `,`/`.` frame a frame, `Espacio` pausa, `F` pantalla completa.

### Compartir

Antes de compartir se elige el formato: **para compartir** (1080p, ~1 MB/s), **liviano**
(720p, menos de 16 MB) u **original**. Si hace falta, la copia se codifica en el momento y
queda en la carpeta `share/` de la partida.

- **Archivo**: copiar el video (para pegarlo con Ctrl+V en WhatsApp, Discord o Telegram),
  abrir el panel "Compartir" de Windows, guardar como o mostrarlo en la carpeta.
- **Wi-Fi**: QR con un enlace local (`http://<tu-ip>:8765/…`); el teléfono abre el video y
  puede descargarlo. Debe estar en la misma red. El enlace vence en 24 h. Se usa la dirección
  del adaptador de cable o Wi-Fi (no la de una VPN) y, si el firewall de Windows bloquea la
  app, el diálogo ofrece permitirla (pide permiso de administrador).
- **Google Drive**: sube el video a la carpeta `ggswp` de tu Drive y genera un QR al enlace
  (visible para quien tenga el link).

### Configurar Google Drive

1. En [Google Cloud Console](https://console.cloud.google.com/) crea un proyecto y habilita **Google Drive API**.
2. **Pantalla de consentimiento OAuth**: tipo *Externo*. Agrega tu cuenta como usuario de prueba
   o, mejor, publícala ("En producción"): el permiso `drive.file` no requiere verificación y así
   el acceso no vence a los 7 días.
3. **Credenciales → Crear ID de cliente OAuth → App de escritorio**. Copia el *Client ID* y el
   *Client secret* en ggswp → Ajustes → Google Drive → Conectar.

ggswp solo puede ver los archivos que él mismo sube. Opcionalmente sube cada partida y/o sus
clips automáticamente al terminar.

## Dónde se guarda todo

- Grabaciones: `%USERPROFILE%\Videos\ggswp\<fecha>\` (`match.mp4`, `meta.json`, `thumb.jpg`, `clips\`).
- Ajustes: `%APPDATA%\com.ggswp.app\settings.json` (y el histórico en `stats.json`).
- Búfer temporal: `%LOCALAPPDATA%\com.ggswp.app\buffer\` (se borra solo).

## Desarrollo

```bash
npm install
npx tauri dev        # app en modo desarrollo
npx tauri build      # instalador NSIS en src-tauri/target/release/bundle/nsis/
```

Para el instalador, copia un `ffmpeg.exe` con `ddagrab` a `src-tauri/bin/` (se incluye como
recurso; si no está, la app usa el `ffmpeg` del PATH).

Pruebas (las marcadas `ignore` usan los archivos reales de Fortnite y graban la pantalla):

```bash
cd src-tauri
cargo test                                  # parser del log
cargo test -- --ignored --nocapture         # replay real, captura de 6 s
cargo test --test pipeline -- --ignored     # grabación + análisis + clips de punta a punta
```

### Mapa del código

```
src-tauri/src/
  fortnite/log.rs     seguimiento del log y eventos de partida
  fortnite/replay.rs  lector de .replay (eliminaciones)
  capture/            ddagrab+NVENC, WASAPI, mux final, utilidades Win32
  engine.rs           máquina de estados, búfer de repetición, post-proceso
  library.rs          biblioteca en disco, línea de tiempo, clips
  overlay.rs          avisos y menú de highlights sobre el juego
  hotkeys.rs          atajos globales
  share.rs            servidor Wi-Fi + QR, red local y firewall
  winshare.rs         portapapeles y panel "Compartir" de Windows
  export.rs           formatos de salida de los clips (liviano, mínimo, original)
  gdrive.rs uploads.rs  Google Drive
  fortnite/chests.rs  cofres abiertos (OCR de los avisos de PE)
  montage.rs          resumen de la partida y versión vertical
  retention.rs        vencimiento (PC y Drive)
  stats.rs            histórico de estadísticas
src/                  interfaz React (galería, clips, editor, estadísticas, ajustes, overlay)
scripts/release.ps1   publica una versión en GitHub Releases (con firma para el actualizador)
```

### Publicar una versión

1. Sube la versión en `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` y `package.json`, y haz commit.
2. `powershell -File scripts/release.ps1 -Notes "Qué cambió"`

El script compila, firma el instalador con la clave de `%USERPROFILE%\.tauri\ggswp.key` (nunca va
al repositorio), genera `latest.json` y crea la release. Las copias instaladas la descargan solas.

## Licencias de terceros

El instalador incluye `ffmpeg.exe` (FFmpeg, GPLv3), que ggswp ejecuta como programa aparte. Ver
[`src-tauri/bin/FFMPEG-NOTICE.txt`](src-tauri/bin/FFMPEG-NOTICE.txt) para la versión y el código fuente.

## Limitaciones conocidas

- Con Fortnite en *Pantalla completa* exclusiva no se ven los avisos ni el menú (la grabación
  funciona igual).
- El audio del sistema incluye todo lo que suena (Discord, música…). La captura solo del
  proceso de Fortnite queda pendiente.
- Solo codificadores NVIDIA por ahora (AMD AMF / Intel QSV serían fáciles de agregar).
- Los videos HEVC/AV1 se reproducen dentro de la app solo si Windows tiene los códecs; H.264
  funciona siempre.
