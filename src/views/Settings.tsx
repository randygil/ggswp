import { useEffect, useState, type ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errText, fmtSize, type Monitor, type Settings, type SetupCheck } from "../api";
import { useToast } from "../components/Toast";
import { HotkeyInput } from "../components/HotkeyInput";

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
  const toast = useToast();

  useEffect(() => {
    api.settings().then(setS);
    api.monitors().then(setMonitors);
    api.checkSetup().then(setCheck);
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
      if (failed.length) toast.error(`No se pudieron registrar estos atajos (¿los usa otra app?): ${failed.join(", ")}`);
      else toast.info("Ajustes guardados");
      api.checkSetup().then(setCheck);
    } catch (e) {
      toast.error(errText(e));
    }
  };
  const compressClips = async () => {
    if (!confirm("Los clips que siguen a la calidad de grabación pasan a 1080p liviano y se reemplazan. La partida completa no se toca. ¿Comprimirlos?")) return;
    setCompressing(true);
    try {
      const r = await api.compressClips();
      toast.info(r.clips ? `${r.clips} clips comprimidos · ${fmtSize(r.saved_bytes)} liberados` : "No había clips por comprimir");
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
          <h1>Ajustes</h1>
          <p className="muted">Los cambios de grabación se aplican desde la próxima partida.</p>
        </div>
        <button className="btn primary" disabled={!dirty} onClick={save}>
          Guardar cambios
        </button>
      </header>

      {check && (
        <section className="panel">
          <h2>Estado del sistema</h2>
          <ul className="checks">
            <li className={check.ffmpeg ? "ok" : "bad"}>{check.ffmpeg ? `ffmpeg: ${check.ffmpeg.replace("ffmpeg version ", "")}` : `ffmpeg no encontrado (${check.ffmpeg_path})`}</li>
            <li className={check.nvenc ? "ok" : "bad"}>{check.nvenc ? `Codificador ${s.encoder} disponible` : `El codificador ${s.encoder} no está disponible`}</li>
            <li className={check.fortnite_log ? "ok" : "bad"}>{check.fortnite_log ? "Log de Fortnite encontrado" : "No se encontró el log de Fortnite (¿está instalado?)"}</li>
            <li className={check.replays_enabled === false ? "bad" : "ok"}>
              {check.replays_enabled === false
                ? "Las repeticiones están desactivadas en Fortnite: actívalas (Ajustes → Juego → Repeticiones) para detectar eliminaciones"
                : "Repeticiones de Fortnite activas (se usan para detectar eliminaciones)"}
            </li>
          </ul>
        </section>
      )}

      <section className="panel">
        <h2>Grabación</h2>
        <Row label="Grabar partidas automáticamente">
          <Toggle value={s.auto_record} onChange={(v) => set("auto_record", v)} />
        </Row>
        <Row label="Empezar a grabar" hint="La isla previa incluye el calentamiento">
          <select value={s.start_at} onChange={(e) => set("start_at", e.target.value as Settings["start_at"])}>
            <option value="warmup">Al empezar la partida (isla previa)</option>
            <option value="bus">Al subir al autobús</option>
          </select>
        </Row>
        <Row label="Terminar de grabar">
          <select value={s.stop_at} onChange={(e) => set("stop_at", e.target.value as Settings["stop_at"])}>
            <option value="player_finished">Cuando te eliminan o ganas</option>
            <option value="lobby">Al volver al lobby (incluye espectador)</option>
          </select>
        </Row>
        {s.stop_at === "player_finished" && (
          <Row label="Segundos extra al final">
            <input className="input num" type="number" min={0} max={60} value={s.post_roll_s} onChange={(e) => set("post_roll_s", num(e.target.value, 8))} />
          </Row>
        )}
        <Row label="Monitor" hint="Automático usa la pantalla donde está Fortnite">
          <select value={s.monitor ?? ""} onChange={(e) => set("monitor", e.target.value === "" ? null : Number(e.target.value))}>
            <option value="">Automático</option>
            {monitors.map((m) => (
              <option key={m.index} value={m.index}>
                Pantalla {m.index + 1} · {m.width}×{m.height}
              </option>
            ))}
          </select>
        </Row>
        <Row label="Codificador" hint="H.264 se reproduce en todos lados; HEVC/AV1 pesan menos">
          <select value={s.encoder} onChange={(e) => set("encoder", e.target.value)}>
            <option value="h264_nvenc">NVIDIA H.264 (recomendado)</option>
            <option value="hevc_nvenc">NVIDIA HEVC (H.265)</option>
            <option value="av1_nvenc">NVIDIA AV1</option>
          </select>
        </Row>
        <Row label="Calidad" hint="Menor número = más calidad y archivos más grandes">
          <select value={s.quality_cq} onChange={(e) => set("quality_cq", Number(e.target.value))}>
            <option value={16}>Máxima (CQ 16)</option>
            <option value={19}>Alta (CQ 19)</option>
            <option value={23}>Media (CQ 23)</option>
            <option value={27}>Ligera (CQ 27)</option>
          </select>
        </Row>
        <Row label="Cuadros por segundo">
          <select value={s.fps} onChange={(e) => set("fps", Number(e.target.value))}>
            {[30, 60, 120].map((f) => (
              <option key={f} value={f}>
                {f} fps
              </option>
            ))}
          </select>
        </Row>
        <Row label="Bitrate máximo (Mbps)">
          <input className="input num" type="number" min={10} max={300} value={s.max_bitrate_mbps} onChange={(e) => set("max_bitrate_mbps", num(e.target.value, 80))} />
        </Row>
      </section>

      <section className="panel">
        <h2>Audio</h2>
        <Row label="Audio del juego y del sistema">
          <Toggle value={s.system_audio} onChange={(v) => set("system_audio", v)} />
        </Row>
        <Row label="Micrófono">
          <Toggle value={s.microphone} onChange={(v) => set("microphone", v)} />
        </Row>
        {s.microphone && (
          <Row label="Volumen del micrófono">
            <input type="range" min={0} max={2} step={0.05} value={s.mic_volume} onChange={(e) => set("mic_volume", Number(e.target.value))} />
            <span className="muted">{Math.round(s.mic_volume * 100)}%</span>
          </Row>
        )}
      </section>

      <section className="panel">
        <h2>Clips</h2>
        <Row
          label="Formato de los clips"
          hint="Para compartir: H.264 a 1080p, cerca de 1 MB por segundo; entra en WhatsApp e Instagram. Original: como la grabación, unas 10 veces más pesado."
        >
          <select value={s.clip_quality} onChange={(e) => set("clip_quality", e.target.value as Settings["clip_quality"])}>
            <option value="share">Para compartir · 1080p (recomendado)</option>
            <option value="original">Calidad original</option>
          </select>
        </Row>
        <Row label="Clips que ya tienes" hint="Pasa al formato liviano los clips guardados a la calidad de grabación. El avance se ve en cada partida.">
          <button className="btn ghost" disabled={compressing} onClick={compressClips}>
            {compressing ? (
              <>
                <span className="spinner" /> Comprimiendo…
              </>
            ) : (
              "Comprimir clips existentes"
            )}
          </button>
        </Row>
        <Row
          label="Contar cofres abiertos"
          hint="Al terminar la partida lee en el video los avisos de experiencia (+PE · COFRE). Tarda unos segundos por cada minuto de partida."
        >
          <Toggle value={s.detect_chests} onChange={(v) => set("detect_chests", v)} />
        </Row>
        <Row label="Crear clips de eliminaciones y victorias">
          <Toggle value={s.auto_clips} onChange={(v) => set("auto_clips", v)} />
        </Row>
        <Row label="Segundos antes del momento">
          <input className="input num" type="number" min={2} max={60} value={s.clip_before_s} onChange={(e) => set("clip_before_s", num(e.target.value, 12))} />
        </Row>
        <Row label="Segundos después">
          <input className="input num" type="number" min={1} max={30} value={s.clip_after_s} onChange={(e) => set("clip_after_s", num(e.target.value, 4))} />
        </Row>
        <Row label="Incluir el momento en que te eliminan">
          <Toggle value={s.clip_deaths} onChange={(v) => set("clip_deaths", v)} />
        </Row>
      </section>

      <section className="panel">
        <h2>Atajos y highlights</h2>
        <Row label="Guardar los últimos segundos" hint="Funciona dentro y fuera de partida">
          <HotkeyInput value={s.hotkeys.save_clip} onChange={(v) => set("hotkeys", { ...s.hotkeys, save_clip: v })} />
        </Row>
        <Row label="Duración del clip">
          <select value={s.clip_seconds} onChange={(e) => set("clip_seconds", Number(e.target.value))}>
            {[15, 30, 45, 60, 90, 120].map((n) => (
              <option key={n} value={n}>
                Últimos {n} s
              </option>
            ))}
          </select>
        </Row>
        <Row label="Marcar momento" hint="Agrega una marca en la línea de tiempo de la partida">
          <HotkeyInput value={s.hotkeys.bookmark} onChange={(v) => set("hotkeys", { ...s.hotkeys, bookmark: v })} />
        </Row>
        <Row label="Iniciar / detener grabación">
          <HotkeyInput value={s.hotkeys.toggle_recording} onChange={(v) => set("hotkeys", { ...s.hotkeys, toggle_recording: v })} />
        </Row>
        <Row label="Menú de highlights sobre el juego">
          <HotkeyInput value={s.hotkeys.highlights_menu} onChange={(v) => set("hotkeys", { ...s.hotkeys, highlights_menu: v })} />
        </Row>
        <Row
          label="Búfer de repetición"
          hint="Mientras Fortnite está abierto graba en segundo plano (temporal) para poder guardar clips también en el lobby"
        >
          <Toggle value={s.replay_buffer} onChange={(v) => set("replay_buffer", v)} />
        </Row>
        <Row label="Avisos sobre el juego" hint="Requiere Fortnite en modo Ventana sin bordes">
          <Toggle value={s.overlay} onChange={(v) => set("overlay", v)} />
        </Row>
        {s.overlay && (
          <Row label="Avisar cuando se registra un momento destacado">
            <Toggle value={s.overlay_moments} onChange={(v) => set("overlay_moments", v)} />
          </Row>
        )}
      </section>

      <section className="panel">
        <h2>Biblioteca</h2>
        <Row label="Carpeta de grabaciones">
          <span className="path">{s.library_dir}</span>
          <button
            className="btn ghost"
            onClick={async () => {
              const dir = await open({ directory: true, defaultPath: s.library_dir });
              if (typeof dir === "string") set("library_dir", dir);
            }}
          >
            Cambiar…
          </button>
        </Row>
        <Row
          label="Borrar de la PC lo que ya está en Drive"
          hint="Antes de borrar se comprueba que la copia de Drive esté completa. Lo que no se subió se queda en la PC. Puedes volver a descargarlo desde la partida."
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
            <option value="upload">Apenas se sube</option>
            <option value="3">A los 3 días</option>
            <option value="7">A los 7 días</option>
            <option value="14">A los 14 días</option>
            <option value="0">Nunca</option>
          </select>
        </Row>
        <Row
          label="Borrar del todo (PC y Drive) después de"
          hint="Solo las partidas no conservadas. En Drive van a la papelera, donde se pueden recuperar durante 30 días."
        >
          <select value={s.retention_days} onChange={(e) => set("retention_days", Number(e.target.value))}>
            <option value={7}>7 días</option>
            <option value={15}>15 días</option>
            <option value={30}>30 días</option>
            <option value={60}>60 días</option>
            <option value={0}>Nunca</option>
          </select>
        </Row>
      </section>

      <section className="panel">
        <h2>Google Drive</h2>
        {s.drive.refresh_token ? (
          <>
            <Row label="Cuenta conectada">
              <span>{s.drive.account_email ?? "Conectada"}</span>
              <button
                className="btn ghost"
                onClick={async () => {
                  await api.driveDisconnect();
                  setS(await api.settings());
                }}
              >
                Desconectar
              </button>
            </Row>
            <Row label="Subir automáticamente al terminar cada partida">
              <Toggle value={s.drive.auto_upload} onChange={(v) => setDrive("auto_upload", v)} />
            </Row>
            {s.drive.auto_upload && (
              <>
                <Row label="Subir clips">
                  <Toggle value={s.drive.upload_clips} onChange={(v) => setDrive("upload_clips", v)} />
                </Row>
                <Row label="Subir la partida completa" hint="Con compresión, unos 130 MB por minuto de partida">
                  <Toggle value={s.drive.upload_full_match} onChange={(v) => setDrive("upload_full_match", v)} />
                </Row>
              </>
            )}
            <Row
              label="Formato de la copia en Drive"
              hint="Se recodifica con la tarjeta gráfica antes de subir: casi igual a la vista y unas 4 veces más liviano. La copia de la PC no cambia."
            >
              <select value={s.drive.codec} onChange={(e) => setDrive("codec", e.target.value)}>
                <option value="hevc">HEVC (recomendado)</option>
                <option value="av1">AV1 (el más liviano)</option>
                <option value="h264">H.264 (compatible con todo)</option>
                <option value="original">Original, sin comprimir</option>
              </select>
            </Row>

          </>
        ) : (
          <>
            <p className="muted small">
              Crea un ID de cliente OAuth de tipo <b>App de escritorio</b> en Google Cloud Console (API de Google Drive habilitada) y pega aquí sus datos. ggswp solo puede ver los
              archivos que él mismo sube.
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
                    toast.info(`Drive conectado: ${email}`);
                    setS(await api.settings());
                    setDirty(false);
                  } catch (e) {
                    toast.error(errText(e));
                  } finally {
                    setConnecting(false);
                  }
                }}
              >
                {connecting ? "Esperando autorización en el navegador…" : "Conectar Google Drive"}
              </button>
            </Row>
          </>
        )}
      </section>

      <section className="panel">
        <h2>Aplicación</h2>
        <Row label="Abrir ggswp al iniciar Windows">
          <Toggle value={s.launch_at_startup} onChange={(v) => set("launch_at_startup", v)} />
        </Row>
        <Row label="Iniciar minimizado en la bandeja">
          <Toggle value={s.start_minimized} onChange={(v) => set("start_minimized", v)} />
        </Row>
        <Row label="Al cerrar la ventana, seguir en la bandeja" hint="Necesario para grabar con la ventana cerrada">
          <Toggle value={s.close_to_tray} onChange={(v) => set("close_to_tray", v)} />
        </Row>
        <Row label="Puerto para compartir por Wi-Fi">
          <input className="input num" type="number" min={1024} max={65535} value={s.share_port} onChange={(e) => set("share_port", num(e.target.value, 8765))} />
        </Row>
      </section>
    </div>
  );
}
