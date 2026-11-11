import { useEffect, useState, type ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errText, fmtSize, type Monitor, type Settings, type SetupCheck } from "../api";
import { useToast } from "../components/Toast";
import { HotkeyInput } from "../components/HotkeyInput";
import { t } from "../i18n";

const ENCODER_LABEL: Record<string, string> = {
  h264_nvenc: "NVIDIA H.264 (recomendado)",
  hevc_nvenc: "NVIDIA HEVC (H.265)",
  av1_nvenc: "NVIDIA AV1",
  h264_amf: "AMD H.264 (recomendado)",
  hevc_amf: "AMD HEVC (H.265)",
  av1_amf: "AMD AV1",
  h264_qsv: "Intel H.264 (recomendado)",
  hevc_qsv: "Intel HEVC (H.265)",
  av1_qsv: "Intel AV1",
  libx264: "Por software · H.264 (usa el procesador)",
};

const encoderLabel = (e: string) => (ENCODER_LABEL[e] ? t(ENCODER_LABEL[e]) : e);

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="row">
      <span className="row-label">
        {label}
        {hint && <small>{hint}</small>}
      </span>
      <span className="row-control">{children}</span>
    </label>
  );
}

function Toggle({ value, onChange }: { value: boolean; onChange: (v: boolean) => void }) {
  return <input type="checkbox" role="switch" className="switch" checked={value} onChange={(e) => onChange(e.target.checked)} />;
}

export function SettingsView() {
  const [s, setS] = useState<Settings | null>(null);
  const [monitors, setMonitors] = useState<Monitor[]>([]);
  const [check, setCheck] = useState<SetupCheck | null>(null);
  const [dirty, setDirty] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [compressing, setCompressing] = useState(false);
  const [apps, setApps] = useState<{ title: string; exe: string }[]>([]);
  const toast = useToast();

  useEffect(() => {
    api.settings().then(setS);
    api.monitors().then(setMonitors);
    api.checkSetup().then(setCheck);
    api.openApps().then(setApps).catch(() => {});
  }, []);

  if (!s) return <div className="page" />;

  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => {
    setS({ ...s, [k]: v });
    setDirty(true);
  };
  const setDrive = <K extends keyof Settings["drive"]>(k: K, v: Settings["drive"][K]) => {
    setS({ ...s, drive: { ...s.drive, [k]: v } });
    setDirty(true);
  };
  const save = async () => {
    try {
      const failed = await api.saveSettings(s);
      setDirty(false);
      if (failed.length) toast.error(t("No se pudieron registrar estos atajos (¿los usa otra app?): {list}", { list: failed.join(", ") }));
      else toast.info(t("Ajustes guardados"));
      api.checkSetup().then(setCheck);
    } catch (e) {
      toast.error(errText(e));
    }
  };
  const compressClips = async () => {
    if (!confirm(t("Los clips que siguen a la calidad de grabación pasan a 1080p liviano y se reemplazan. La partida completa no se toca. ¿Comprimirlos?"))) return;
    setCompressing(true);
    try {
      const r = await api.compressClips();
      toast.info(r.clips ? t("{n} clips comprimidos · {size} liberados", { n: r.clips, size: fmtSize(r.saved_bytes) }) : t("No había clips por comprimir"));
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setCompressing(false);
    }
  };
  const num = (v: string, d: number) => (v === "" || isNaN(Number(v)) ? d : Number(v));

  return (
    <div className="page settings">
      <header className="page-head">
        <div>
          <h1>{t("Ajustes")}</h1>
          <p className="muted">{t("Los cambios de grabación se aplican desde la próxima partida.")}</p>
        </div>
        <button className="btn primary" disabled={!dirty} onClick={save}>
          {t("Guardar cambios")}
        </button>
      </header>

      {check && (
        <section className="panel">
          <h2>{t("Estado del sistema")}</h2>
          <ul className="checks">
            <li className={check.ffmpeg ? "ok" : "bad"}>
              {check.ffmpeg ? `ffmpeg: ${check.ffmpeg.replace("ffmpeg version ", "")}` : t("ffmpeg no encontrado ({path})", { path: check.ffmpeg_path })}
            </li>
            <li className={check.nvenc ? "ok" : check.encoders.length ? "warn" : "bad"}>
              {check.nvenc
                ? t("Codificador {name} disponible", { name: encoderLabel(s.encoder) })
                : check.encoders.length
                  ? t("{name} no funciona en esta PC: se graba con {used}", { name: encoderLabel(s.encoder), used: encoderLabel(check.encoder_in_use) })
                  : t("No se encontró ningún codificador de video que funcione")}
            </li>
            <li className={check.fortnite_log ? "ok" : "bad"}>{check.fortnite_log ? t("Log de Fortnite encontrado") : t("No se encontró el log de Fortnite (¿está instalado?)")}</li>
            <li className={check.replays_enabled === false ? "bad" : "ok"}>
              {check.replays_enabled === false
                ? t("Las repeticiones están desactivadas en Fortnite: actívalas (Ajustes → Juego → Repeticiones) para detectar eliminaciones")
                : t("Repeticiones de Fortnite activas (se usan para detectar eliminaciones)")}
            </li>
          </ul>
        </section>
      )}

      <section className="panel">
        <h2>{t("Grabación")}</h2>
        <Row label={t("Idioma")} hint={t("Idioma de la interfaz. Automático usa el de Windows.")}>
          <select value={s.language ?? "auto"} onChange={(e) => set("language", e.target.value as Settings["language"])}>
            <option value="auto">{t("Automático")}</option>
            <option value="es">Español</option>
            <option value="en">English</option>
          </select>
        </Row>
        <Row label={t("Grabar partidas automáticamente")}>
          <Toggle value={s.auto_record} onChange={(v) => set("auto_record", v)} />
        </Row>
        <Row label={t("Empezar a grabar")} hint={t("La isla previa incluye el calentamiento")}>
          <select value={s.start_at} onChange={(e) => set("start_at", e.target.value as Settings["start_at"])}>
            <option value="warmup">{t("Al empezar la partida (isla previa)")}</option>
            <option value="bus">{t("Al subir al autobús")}</option>
          </select>
        </Row>
        <Row label={t("Terminar de grabar")}>
          <select value={s.stop_at} onChange={(e) => set("stop_at", e.target.value as Settings["stop_at"])}>
            <option value="player_finished">{t("Cuando te eliminan o ganas")}</option>
            <option value="lobby">{t("Al volver al lobby (incluye espectador)")}</option>
          </select>
        </Row>
        {s.stop_at === "player_finished" && (
          <Row label={t("Segundos extra al final")}>
            <input className="input num" type="number" min={0} max={60} value={s.post_roll_s} onChange={(e) => set("post_roll_s", num(e.target.value, 8))} />
          </Row>
        )}
        <Row label={t("Monitor")} hint={t("Automático usa la pantalla donde está Fortnite")}>
          <select value={s.monitor ?? ""} onChange={(e) => set("monitor", e.target.value === "" ? null : Number(e.target.value))}>
            <option value="">{t("Automático")}</option>
            {monitors.map((m) => (
              <option key={m.index} value={m.index}>
                {t("Pantalla {n} · {w}×{h}", { n: m.index + 1, w: m.width, h: m.height })}
              </option>
            ))}
          </select>
        </Row>
        <Row
          label={t("Codificador")}
          hint={t("H.264 se reproduce en todos lados; HEVC/AV1 pesan menos. Se listan los que funcionan con tu tarjeta gráfica; por software sirve en cualquier PC pero usa mucho procesador.")}
        >
          <select value={s.encoder} onChange={(e) => set("encoder", e.target.value)}>
            {/* El elegido siempre aparece, aunque no funcione aquí; el resto, solo si funciona. */}
            {Object.keys(ENCODER_LABEL)
              .filter((e) => e === s.encoder || !check || check.encoders.includes(e))
              .map((e) => (
                <option key={e} value={e}>
                  {check && !check.encoders.includes(e) ? t("{name} (no disponible)", { name: encoderLabel(e) }) : encoderLabel(e)}
                </option>
              ))}
          </select>
        </Row>
        <Row label={t("Calidad")} hint={t("Menor número = más calidad y archivos más grandes")}>
          <select value={s.quality_cq} onChange={(e) => set("quality_cq", Number(e.target.value))}>
            <option value={16}>{t("Máxima (CQ 16)")}</option>
            <option value={19}>{t("Alta (CQ 19)")}</option>
            <option value={23}>{t("Media (CQ 23)")}</option>
            <option value={27}>{t("Ligera (CQ 27)")}</option>
          </select>
        </Row>
        <Row label={t("Cuadros por segundo")}>
          <select value={s.fps} onChange={(e) => set("fps", Number(e.target.value))}>
            {[30, 60, 120].map((f) => (
              <option key={f} value={f}>
                {f} fps
              </option>
            ))}
          </select>
        </Row>
        <Row label={t("Bitrate máximo (Mbps)")}>
          <input className="input num" type="number" min={10} max={300} value={s.max_bitrate_mbps} onChange={(e) => set("max_bitrate_mbps", num(e.target.value, 80))} />
        </Row>
      </section>

      <section className="panel">
        <h2>{t("Otros juegos")}</h2>
        <p className="muted small" style={{ margin: 0 }}>
          {t("Además de Fortnite, ggswp graba la sesión entera de estos juegos mientras estén abiertos. No detecta eliminaciones ni arma clips solo: usa los atajos para marcar momentos y guardar clips.")}
        </p>
        {s.games.map((g, i) => (
          <Row key={g.exe} label={g.name} hint={g.exe}>
            <input className="input" aria-label={t("Nombre de {exe}", { exe: g.exe })} value={g.name} maxLength={40} onChange={(e) => set("games", s.games.map((x, n) => (n === i ? { ...x, name: e.target.value } : x)))} />
            <button className="btn ghost" onClick={() => set("games", s.games.filter((_, n) => n !== i))}>
              {t("Quitar")}
            </button>
          </Row>
        ))}
        <Row label={t("Agregar un juego")} hint={t("Abre el juego y elígelo de la lista de programas abiertos.")}>
          <select
            value=""
            onFocus={() => api.openApps().then(setApps).catch(() => {})}
            onChange={(e) => {
              const a = apps.find((x) => x.exe === e.target.value);
              if (a && !s.games.some((g) => g.exe.toLowerCase() === a.exe.toLowerCase())) set("games", [...s.games, { name: a.title.slice(0, 40), exe: a.exe }]);
            }}
          >
            <option value="">{apps.length ? t("Elegir un programa abierto…") : t("Buscar programas abiertos…")}</option>
            {apps.map((a) => (
              <option key={a.exe} value={a.exe}>
                {a.title} ({a.exe})
              </option>
            ))}
          </select>
        </Row>
      </section>

      <section className="panel">
        <h2>{t("Audio")}</h2>
        <Row label={t("Audio del juego y del sistema")}>
          <Toggle value={s.system_audio} onChange={(v) => set("system_audio", v)} />
        </Row>
        {s.system_audio && (
          <Row
            label={t("Separar el audio del juego")}
            hint={t("Graba el juego en una pista y el resto (Discord, música, navegador) en otra, para elegir qué lleva cada clip. Necesita Windows 10 de 2020 o más nuevo; si no se puede, graba todo junto.")}
          >
            <Toggle value={s.separate_audio} onChange={(v) => set("separate_audio", v)} />
          </Row>
        )}
        {s.system_audio && s.separate_audio && (
          <Row label={t("Los clips incluyen las otras apps")} hint={t("Apagado, los clips llevan solo el sonido del juego. La partida completa siempre guarda todo, y en el editor puedes cambiarlo clip por clip.")}>
            <Toggle value={s.clip_other_audio} onChange={(v) => set("clip_other_audio", v)} />
          </Row>
        )}
        <Row label={t("Micrófono")}>
          <Toggle value={s.microphone} onChange={(v) => set("microphone", v)} />
        </Row>
        {s.microphone && (
          <Row label={t("Volumen del micrófono")}>
            <input type="range" min={0} max={2} step={0.05} value={s.mic_volume} onChange={(e) => set("mic_volume", Number(e.target.value))} />
            <span className="muted">{Math.round(s.mic_volume * 100)}%</span>
          </Row>
        )}
        {s.microphone && (
          <Row label={t("Los clips incluyen el micrófono")}>
            <Toggle value={s.clip_mic_audio} onChange={(v) => set("clip_mic_audio", v)} />
          </Row>
        )}
      </section>

      <section className="panel">
        <h2>{t("Clips")}</h2>
        <Row
          label={t("Formato de los clips")}
          hint={t("Para compartir: H.264 a 1080p, cerca de 1 MB por segundo; entra en WhatsApp e Instagram. Original: como la grabación, unas 10 veces más pesado.")}
        >
          <select value={s.clip_quality} onChange={(e) => set("clip_quality", e.target.value as Settings["clip_quality"])}>
            <option value="share">{t("Para compartir · 1080p (recomendado)")}</option>
            <option value="original">{t("Calidad original")}</option>
          </select>
        </Row>
        <Row label={t("Clips que ya tienes")} hint={t("Pasa al formato liviano los clips guardados a la calidad de grabación. El avance se ve en cada partida.")}>
          <button className="btn ghost" disabled={compressing} onClick={compressClips}>
            {compressing ? (
              <>
                <span className="spinner" /> {t("Comprimiendo…")}
              </>
            ) : (
              t("Comprimir clips existentes")
            )}
          </button>
        </Row>
        <Row
          label={t("Contar cofres abiertos")}
          hint={t("Al terminar la partida lee en el video los avisos de experiencia (+PE · COFRE). Tarda unos segundos por cada minuto de partida.")}
        >
          <Toggle value={s.detect_chests} onChange={(v) => set("detect_chests", v)} />
        </Row>
        <Row label={t("Crear clips de eliminaciones y victorias")}>
          <Toggle value={s.auto_clips} onChange={(v) => set("auto_clips", v)} />
        </Row>
        {s.auto_clips && (
          <Row label={t("Mejor jugada lista para subir")} hint={t("Al terminar cada partida elige su mejor clip y deja una versión 9:16 para TikTok, Reels y Shorts.")}>
            <select value={s.best_play} onChange={(e) => set("best_play", e.target.value as Settings["best_play"])}>
              <option value="vertical">{t("Vertical 9:16")}</option>
              <option value="mobile">{t("Modo móvil (con salud y armas)")}</option>
              <option value="off">{t("No crearla")}</option>
            </select>
          </Row>
        )}
        <Row label={t("Segundos antes del momento")}>
          <input className="input num" type="number" min={2} max={60} value={s.clip_before_s} onChange={(e) => set("clip_before_s", num(e.target.value, 12))} />
        </Row>
        <Row label={t("Segundos después")}>
          <input className="input num" type="number" min={1} max={30} value={s.clip_after_s} onChange={(e) => set("clip_after_s", num(e.target.value, 4))} />
        </Row>
        <Row label={t("Incluir el momento en que te eliminan")}>
          <Toggle value={s.clip_deaths} onChange={(v) => set("clip_deaths", v)} />
        </Row>
      </section>

      <section className="panel">
        <h2>{t("Atajos y highlights")}</h2>
        <Row label={t("Guardar los últimos segundos")} hint={t("Funciona dentro y fuera de partida")}>
          <HotkeyInput value={s.hotkeys.save_clip} onChange={(v) => set("hotkeys", { ...s.hotkeys, save_clip: v })} />
        </Row>
        <Row label={t("Duración del clip")}>
          <select value={s.clip_seconds} onChange={(e) => set("clip_seconds", Number(e.target.value))}>
            {[15, 30, 45, 60, 90, 120].map((n) => (
              <option key={n} value={n}>
                {t("Últimos {n} s", { n })}
              </option>
            ))}
          </select>
        </Row>
        <Row label={t("Marcar momento")} hint={t("Agrega una marca en la línea de tiempo de la partida")}>
          <HotkeyInput value={s.hotkeys.bookmark} onChange={(v) => set("hotkeys", { ...s.hotkeys, bookmark: v })} />
        </Row>
        <Row label={t("Iniciar / detener grabación")}>
          <HotkeyInput value={s.hotkeys.toggle_recording} onChange={(v) => set("hotkeys", { ...s.hotkeys, toggle_recording: v })} />
        </Row>
        <Row label={t("Menú de highlights sobre el juego")}>
          <HotkeyInput value={s.hotkeys.highlights_menu} onChange={(v) => set("hotkeys", { ...s.hotkeys, highlights_menu: v })} />
        </Row>
        <Row
          label={t("Búfer de repetición")}
          hint={t("Mientras Fortnite está abierto graba en segundo plano (temporal) para poder guardar clips también en el lobby")}
        >
          <Toggle value={s.replay_buffer} onChange={(v) => set("replay_buffer", v)} />
        </Row>
        <Row label={t("Avisos sobre el juego")} hint={t("Requiere Fortnite en modo Ventana sin bordes")}>
          <Toggle value={s.overlay} onChange={(v) => set("overlay", v)} />
        </Row>
        {s.overlay && (
          <Row label={t("Avisar cuando se registra un momento destacado")}>
            <Toggle value={s.overlay_moments} onChange={(v) => set("overlay_moments", v)} />
          </Row>
        )}
      </section>

      <section className="panel">
        <h2>{t("Biblioteca")}</h2>
        <Row label={t("Carpeta de grabaciones")}>
          <span className="path">{s.library_dir}</span>
          <button
            className="btn ghost"
            onClick={async () => {
              const dir = await open({ directory: true, defaultPath: s.library_dir });
              if (typeof dir === "string") set("library_dir", dir);
            }}
          >
            {t("Cambiar…")}
          </button>
        </Row>
        <Row
          label={t("Borrar de la PC lo que ya está en Drive")}
          hint={t("Antes de borrar se comprueba que la copia de Drive esté completa. Lo que no se subió se queda en la PC. Puedes volver a descargarlo desde la partida.")}
        >
          <select
            value={s.drive.delete_local_after_upload ? "upload" : String(s.local_days)}
            onChange={(e) => {
              const v = e.target.value;
              setS({
                ...s,
                local_days: v === "upload" ? s.local_days : Number(v),
                drive: { ...s.drive, delete_local_after_upload: v === "upload" },
              });
              setDirty(true);
            }}
          >
            <option value="upload">{t("Apenas se sube")}</option>
            <option value="3">{t("A los 3 días")}</option>
            <option value="7">{t("A los 7 días")}</option>
            <option value="14">{t("A los 14 días")}</option>
            <option value="0">{t("Nunca")}</option>
          </select>
        </Row>
        <Row
          label={t("Borrar del todo (PC y Drive) después de")}
          hint={t("Solo las partidas no conservadas. En Drive van a la papelera, donde se pueden recuperar durante 30 días.")}
        >
          <select value={s.retention_days} onChange={(e) => set("retention_days", Number(e.target.value))}>
            <option value={7}>{t("7 días")}</option>
            <option value={15}>{t("15 días")}</option>
            <option value={30}>{t("30 días")}</option>
            <option value={60}>{t("60 días")}</option>
            <option value={0}>{t("Nunca")}</option>
          </select>
        </Row>
      </section>

      <section className="panel">
        <h2>Google Drive</h2>
        {s.drive.refresh_token ? (
          <>
            <Row label={t("Cuenta conectada")}>
              <span>{s.drive.account_email ?? t("Conectada")}</span>
              <button
                className="btn ghost"
                onClick={async () => {
                  await api.driveDisconnect();
                  setS(await api.settings());
                }}
              >
                {t("Desconectar")}
              </button>
            </Row>
            <Row label={t("Subir automáticamente al terminar cada partida")}>
              <Toggle value={s.drive.auto_upload} onChange={(v) => setDrive("auto_upload", v)} />
            </Row>
            {s.drive.auto_upload && (
              <>
                <Row label={t("Subir clips")}>
                  <Toggle value={s.drive.upload_clips} onChange={(v) => setDrive("upload_clips", v)} />
                </Row>
                <Row label={t("Subir la partida completa")} hint={t("Con compresión, unos 130 MB por minuto de partida")}>
                  <Toggle value={s.drive.upload_full_match} onChange={(v) => setDrive("upload_full_match", v)} />
                </Row>
              </>
            )}
            <Row
              label={t("Formato de la copia en Drive")}
              hint={t("Se recodifica con la tarjeta gráfica antes de subir: casi igual a la vista y unas 4 veces más liviano. La copia de la PC no cambia.")}
            >
              <select value={s.drive.codec} onChange={(e) => setDrive("codec", e.target.value)}>
                <option value="hevc">{t("HEVC (recomendado)")}</option>
                <option value="av1">{t("AV1 (el más liviano)")}</option>
                <option value="h264">{t("H.264 (compatible con todo)")}</option>
                <option value="original">{t("Original, sin comprimir")}</option>
              </select>
            </Row>

          </>
        ) : (
          <>
            <p className="muted small">
              {t("Crea un ID de cliente OAuth de tipo")} <b>{t("App de escritorio")}</b>{" "}
              {t("en Google Cloud Console (API de Google Drive habilitada) y pega aquí sus datos. ggswp solo puede ver los archivos que él mismo sube.")}
            </p>
            <Row label="Client ID">
              <input className="input wide" value={s.drive.client_id} onChange={(e) => setDrive("client_id", e.target.value.trim())} />
            </Row>
            <Row label="Client secret">
              <input className="input wide" type="password" value={s.drive.client_secret} onChange={(e) => setDrive("client_secret", e.target.value.trim())} />
            </Row>
            <Row label="">
              <button
                className="btn primary"
                disabled={!s.drive.client_id || connecting}
                onClick={async () => {
                  setConnecting(true);
                  try {
                    await api.saveSettings(s);
                    const email = await api.driveConnect();
                    toast.info(t("Drive conectado: {email}", { email }));
                    setS(await api.settings());
                    setDirty(false);
                  } catch (e) {
                    toast.error(errText(e));
                  } finally {
                    setConnecting(false);
                  }
                }}
              >
                {connecting ? t("Esperando autorización en el navegador…") : t("Conectar Google Drive")}
              </button>
            </Row>
          </>
        )}
      </section>

      <section className="panel">
        <h2>{t("Aplicación")}</h2>
        <Row label={t("Abrir ggswp al iniciar Windows")}>
          <Toggle value={s.launch_at_startup} onChange={(v) => set("launch_at_startup", v)} />
        </Row>
        <Row label={t("Iniciar minimizado en la bandeja")}>
          <Toggle value={s.start_minimized} onChange={(v) => set("start_minimized", v)} />
        </Row>
        <Row label={t("Al cerrar la ventana, seguir en la bandeja")} hint={t("Necesario para grabar con la ventana cerrada")}>
          <Toggle value={s.close_to_tray} onChange={(v) => set("close_to_tray", v)} />
        </Row>
        <Row label={t("Puerto para compartir por Wi-Fi")}>
          <input className="input num" type="number" min={1024} max={65535} value={s.share_port} onChange={(e) => set("share_port", num(e.target.value, 8765))} />
        </Row>
      </section>
    </div>
  );
}
