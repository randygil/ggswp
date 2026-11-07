# ggswp

Grabador automático de partidas de Fortnite para Windows. Detecta cuándo empieza y termina
cada partida, graba la pantalla (lo que ves tú) con el audio, marca tus eliminaciones en la
línea de tiempo, genera clips y te deja editarlos, enviarlos al teléfono o subirlos a Google Drive.

Una sola app de escritorio (Tauri + React), sin OBS: la captura va integrada.

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

### Enviar al teléfono

- **Wi-Fi**: QR con un enlace local (`http://<tu-ip>:8765/…`); el teléfono abre el video y
  puede descargarlo. Debe estar en la misma red. El enlace vence en 24 h.
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
- Ajustes: `%APPDATA%\com.ggswp.app\settings.json`.
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
  share.rs            servidor Wi-Fi + QR
  gdrive.rs uploads.rs  Google Drive
src/                  interfaz React (galería, clips, editor, ajustes, overlay)
```

## Limitaciones conocidas

- Con Fortnite en *Pantalla completa* exclusiva no se ven los avisos ni el menú (la grabación
  funciona igual).
- El audio del sistema incluye todo lo que suena (Discord, música…). La captura solo del
  proceso de Fortnite queda pendiente.
- Solo codificadores NVIDIA por ahora (AMD AMF / Intel QSV serían fáciles de agregar).
- Los videos HEVC/AV1 se reproducen dentro de la app solo si Windows tiene los códecs; H.264
  funciona siempre.
