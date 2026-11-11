import { useEffect, useState } from "react";
import { api, fmtClock, pad2, prettyHotkey, type Settings, type Status, type UploadProgress } from "../api";
import type { Route } from "../App";
import { t, tr } from "../i18n";
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

/** "h264_amf" → "H.264"; "libx264" → "H.264". */
const encoderName = (e: string) => (e.startsWith("hevc") ? "HEVC" : e.startsWith("av1") ? "AV1" : "H.264");

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
  const nav = (name: "gallery" | "clips" | "stats" | "settings", label: string, icon: string, count?: number) => (
    <button className={`nav-item ${active(name) ? "active" : ""}`} aria-current={active(name) ? "page" : undefined} onClick={() => onNavigate({ name })}>
      <Icon name={icon} />
      <span>{label}</span>
      {count !== undefined && <span className="nav-count">{pad2(count)}</span>}
    </button>
  );
  const uploadList = Object.values(uploads);
  const hk = settings?.hotkeys;
  const word = rec
    ? t("Grabando")
    : status?.game_state === "in_match"
      ? t("En partida")
      : status?.fortnite_running
        ? t("Esperando partida")
        : status?.game
          ? t("{game} abierto", { game: status.game })
          : t("Fortnite cerrado");

  return (
    <aside className="rail">
      <div className="brand">
        <span className="brand-mark" aria-hidden />
        <span className="brand-name">ggswp</span>
        <span className="brand-ver">v0.1</span>
      </div>

      <nav className="nav" aria-label={t("Secciones")}>
        {nav("gallery", t("Partidas"), "film", counts.matches)}
        {nav("clips", t("Clips"), "scissors", counts.clips)}
        {nav("stats", t("Estadísticas"), "chart")}
        {nav("settings", t("Ajustes"), "sliders")}
      </nav>

      <section className={`status ${rec ? "rec" : ""}`} aria-label={t("Estado de captura")} aria-live="polite">
        <span className="label">{t("Estado")}</span>
        <div className="status-head">
          <span className={`dot ${rec ? "rec" : status?.fortnite_running ? "on" : ""}`} />
          <span className="status-word">{word}</span>
        </div>
        <dl className="kv">
          {rec ? (
            <>
              <dt>{t("Tiempo")}</dt>
              <dd>
                <Elapsed since={rec.started_at} />
              </dd>
              <dt>{t("Tipo")}</dt>
              <dd>{rec.manual ? t("manual") : status?.game ? status.game : t("partida")}</dd>
            </>
          ) : (
            <>
              <dt>Fortnite</dt>
              <dd>{status?.fortnite_running ? t(GAME_STATE[status.game_state]).toLowerCase() : t("cerrado")}</dd>
            </>
          )}
          {settings && (
            <>
              <dt>{t("Video")}</dt>
              <dd>
                {settings.fps} fps · {encoderName(settings.encoder)}
              </dd>
              <dt>{t("Búfer")}</dt>
              <dd className={status?.buffer_active ? "accent" : ""}>
                {!settings.replay_buffer ? t("apagado") : status?.buffer_active ? t("{n} s listo", { n: settings.clip_seconds }) : t("en espera")}
              </dd>
            </>
          )}
        </dl>
        {status &&
          Object.entries(status.steps ?? {}).map(([id, st]) => (
            <div key={id} className="upload-row" title={id}>
              <span>
                {tr(st.label)}
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
            <Icon name="stop" size={14} filled /> {t("Detener")}
          </button>
        )}
      </section>

      {hk && (
        <section className="keys" aria-label={t("Atajos")}>
          <span className="label">{t("Atajos")}</span>
          {(
            [
              [t("Guardar {n} s", { n: settings?.clip_seconds ?? 30 }), hk.save_clip],
              [t("Marcar momento"), hk.bookmark],
              [t("Grabar / detener"), hk.toggle_recording],
              [t("Menú en el juego"), hk.highlights_menu],
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
                    ? t("Drive en pausa mientras juegas")
                    : `${u.compressing ? t("Comprimiendo para Drive") : u.download ? t("Bajando de Drive") : t("Subiendo a Drive")} · ${Math.round((u.sent / Math.max(1, u.total)) * 100)} %`}
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
            <span title={settings.drive.account_email ?? undefined}>Drive · {settings.drive.account_email ?? t("conectado")}</span>
          ) : (
            <>
              <span title={t("Google Drive sin conectar")}>Google Drive</span>
              <button className="link-btn" onClick={() => onNavigate({ name: "settings" })}>
                {t("Conectar")}
              </button>
            </>
          )}
        </div>
      </div>
    </aside>
  );
}
