import { useEffect, useState } from "react";
import { api, fmtClock, pad2, prettyHotkey, type Settings, type Status, type UploadProgress } from "../api";
import type { Route } from "../App";
import { Icon } from "./Icon";

const GAME_STATE: Record<string, string> = {
  idle: "Fortnite cerrado",
  lobby: "En el lobby",
  loading: "Cargando partida",
  in_match: "En partida",
};

function Elapsed({ since }: { since: string }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  return <>{fmtClock((now - new Date(since).getTime()) / 1000)}</>;
}

const ENCODER: Record<string, string> = { h264_nvenc: "H.264", hevc_nvenc: "HEVC", av1_nvenc: "AV1" };

export function Sidebar(props: {
  route: Route;
  onNavigate: (r: Route) => void;
  status: Status | null;
  settings: Settings | null;
  uploads: Record<string, UploadProgress>;
  counts: { matches: number; clips: number };
}) {
  const { route, onNavigate, status, settings, uploads, counts } = props;
  const rec = status?.recording;
  const active = (name: string) => route.name === name || (name === "gallery" && route.name === "match");
  const nav = (name: "gallery" | "clips" | "settings", label: string, icon: string, count?: number) => (
    <button className={`nav-item ${active(name) ? "active" : ""}`} aria-current={active(name) ? "page" : undefined} onClick={() => onNavigate({ name })}>
      <Icon name={icon} />
      <span>{label}</span>
      {count !== undefined && <span className="nav-count">{pad2(count)}</span>}
    </button>
  );
  const uploadList = Object.values(uploads);
  const hk = settings?.hotkeys;
  const word = rec ? "Grabando" : status?.game_state === "in_match" ? "En partida" : status?.fortnite_running ? "Esperando partida" : "Fortnite cerrado";

  return (
    <aside className="rail">
      <div className="brand">
        <span className="brand-mark" aria-hidden />
        <span className="brand-name">ggswp</span>
        <span className="brand-ver">v0.1</span>
      </div>

      <nav className="nav" aria-label="Secciones">
        {nav("gallery", "Partidas", "film", counts.matches)}
        {nav("clips", "Clips", "scissors", counts.clips)}
        {nav("settings", "Ajustes", "sliders")}
      </nav>

      <section className={`status ${rec ? "rec" : ""}`} aria-label="Estado de captura" aria-live="polite">
        <span className="label">Estado</span>
        <div className="status-head">
          <span className={`dot ${rec ? "rec" : status?.fortnite_running ? "on" : ""}`} />
          <span className="status-word">{word}</span>
        </div>
        <dl className="kv">
          {rec ? (
            <>
              <dt>Tiempo</dt>
              <dd>
                <Elapsed since={rec.started_at} />
              </dd>
              <dt>Tipo</dt>
              <dd>{rec.manual ? "manual" : "partida"}</dd>
            </>
          ) : (
            <>
              <dt>Fortnite</dt>
              <dd>{status?.fortnite_running ? GAME_STATE[status.game_state].toLowerCase() : "cerrado"}</dd>
            </>
          )}
          {settings && (
            <>
              <dt>Video</dt>
              <dd>
                {settings.fps} fps · {ENCODER[settings.encoder] ?? settings.encoder}
              </dd>
              <dt>Búfer</dt>
              <dd className={status?.buffer_active ? "accent" : ""}>
                {!settings.replay_buffer ? "apagado" : status?.buffer_active ? `${settings.clip_seconds} s listo` : "en espera"}
              </dd>
            </>
          )}
        </dl>
        {status &&
          Object.entries(status.steps ?? {}).map(([id, st]) => (
            <div key={id} className="upload-row" title={id}>
              <span>
                {st.label}
                {st.pct != null && ` · ${Math.round(st.pct)} %`}
              </span>
              <div className="meter">
                <div className={st.pct == null ? "indeterminate" : ""} style={{ width: st.pct == null ? "35%" : `${st.pct}%` }} />
              </div>
            </div>
          ))}
        {settings?.replay_buffer && (
          <div className="meter" aria-hidden>
            <div style={{ width: status?.buffer_active ? "100%" : "0%" }} />
          </div>
        )}
        {rec && (
          <button className="btn danger sm full" onClick={() => api.stopRecording()}>
            <Icon name="stop" size={14} filled /> Detener
          </button>
        )}
      </section>

      {hk && (
        <section className="keys" aria-label="Atajos">
          <span className="label">Atajos</span>
          {(
            [
              ["Guardar " + (settings?.clip_seconds ?? 30) + " s", hk.save_clip],
              ["Marcar momento", hk.bookmark],
              ["Grabar / detener", hk.toggle_recording],
              ["Menú en el juego", hk.highlights_menu],
            ] as const
          )
            .filter(([, k]) => k)
            .map(([label, k]) => (
              <div key={label} className="key-row">
                {label} <kbd>{prettyHotkey(k).replace(/ \+ /g, " ")}</kbd>
              </div>
            ))}
        </section>
      )}

      <div className="rail-foot">
        {uploadList.length > 0 && (
          <div className="uploads">
            {uploadList.map((u) => (
              <div key={u.key} className="upload-row">
                <span>
                  {u.waiting
                    ? "Drive en pausa mientras juegas"
                    : `${u.download ? "Bajando de Drive" : "Subiendo a Drive"} · ${Math.round((u.sent / Math.max(1, u.total)) * 100)} %`}
                </span>
                <div className="meter">
                  <div style={{ width: `${u.total ? (u.sent / u.total) * 100 : 0}%` }} />
                </div>
              </div>
            ))}
          </div>
        )}
        <div className="rail-drive">
          <Icon name="cloud" />
          {settings?.drive.refresh_token ? (
            <span title={settings.drive.account_email ?? undefined}>Drive · {settings.drive.account_email ?? "conectado"}</span>
          ) : (
            <>
              <span title="Google Drive sin conectar">Google Drive</span>
              <button className="link-btn" onClick={() => onNavigate({ name: "settings" })}>
                Conectar
              </button>
            </>
          )}
        </div>
      </div>
    </aside>
  );
}
