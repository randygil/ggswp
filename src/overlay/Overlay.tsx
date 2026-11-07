import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, fileUrl, fmtClock, fmtDate, fmtTime, pad2, prettyHotkey, type Clip, type Match, type Settings, type Status } from "../api";
import { Icon } from "../components/Icon";

type Toast = { id: number; text: string; kind: string };

const TOAST_ICON: Record<string, string> = {
  clip: "check",
  bookmark: "flag",
  rec: "record",
  moment: "target",
  warn: "x",
  info: "check",
};

export function Overlay() {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [menu, setMenu] = useState(false);
  const [status, setStatus] = useState<Status | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [matches, setMatches] = useState<Match[]>([]);

  useEffect(() => {
    const subs = [
      listen<{ text: string; kind: string }>("overlay-toast", (e) => {
        const id = Date.now() + Math.random();
        setToasts((t) => [...t.slice(-2), { id, ...e.payload }]);
        setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 3200);
      }),
      listen<boolean>("overlay-menu", (e) => {
        setMenu(e.payload);
        if (e.payload) {
          api.status().then(setStatus);
          api.settings().then(setSettings);
          api.matches().then(setMatches);
        }
      }),
      listen<Status>("status", (e) => setStatus(e.payload)),
      listen("library", () => api.matches().then(setMatches)),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, []);

  // Sin nada que mostrar: la ventana se oculta para no quedar encima del juego.
  useEffect(() => {
    if (toasts.length || menu) return;
    const t = setTimeout(() => api.overlayIdle(), 250);
    return () => clearTimeout(t);
  }, [toasts.length, menu]);

  useEffect(() => {
    if (!menu) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && api.overlayClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [menu]);

  return (
    <div className="ov">
      <div className="ov-toasts">
        {toasts.map((t) => (
          <div key={t.id} className={`ov-toast ${t.kind}`}>
            <span className="ov-toast-icon">
              <Icon name={TOAST_ICON[t.kind] ?? "check"} size={18} />
            </span>
            <span className="ov-toast-text">{t.text}</span>
          </div>
        ))}
      </div>
      {menu && settings && <HighlightsMenu status={status} settings={settings} matches={matches} />}
    </div>
  );
}

function useNow(active: boolean) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(t);
  }, [active]);
  return now;
}

function HighlightsMenu({ status, settings, matches }: { status: Status | null; settings: Settings; matches: Match[] }) {
  const rec = status?.recording;
  const now = useNow(!!rec);
  const elapsed = rec ? (now - new Date(rec.started_at).getTime()) / 1000 : 0;
  const first = useRef<HTMLButtonElement>(null);
  const hk = settings.hotkeys;

  useEffect(() => first.current?.focus(), []);

  const recent = useMemo(
    () =>
      matches
        .flatMap((m) => m.clips.map((c) => ({ m, c })))
        .sort((a, b) => new Date(b.c.created_at).getTime() - new Date(a.c.created_at).getTime())
        .slice(0, 4),
    [matches],
  );

  const kills = status?.live.filter((x) => x.kind === "highlight").length ?? 0;
  const marks = status?.live.filter((x) => x.kind === "bookmark").length ?? 0;
  const key = (h: string) => prettyHotkey(h).replace(/ \+ /g, " ");
  const span = Math.max(elapsed, 1);
  const bufferWidth = Math.min(100, (settings.clip_seconds / span) * 100);

  return (
    <div className="ov-back" onClick={() => api.overlayClose()}>
      <div className="ov-dock" role="dialog" aria-modal="true" aria-label="Menú de highlights" onClick={(e) => e.stopPropagation()}>
        <header className="ov-head">
          <span className="ov-state">
            <span className={`dot ${rec ? "rec" : status?.buffer_active ? "on" : ""}`} />
            {rec ? "GRABANDO" : status?.buffer_active ? "BÚFER ACTIVO" : "SIN GRABACIÓN"}
          </span>
          {rec && <span className="ov-clock">{fmtClock(elapsed)}</span>}
          <span className="ov-meta">
            {rec
              ? `${rec.manual ? "Grabación manual" : "Partida"} · ${pad2(kills)} momentos · ${pad2(marks)} marcas`
              : status?.buffer_active
                ? `Los últimos ${settings.clip_seconds} s están listos para guardar`
                : "Abre Fortnite para activar el búfer"}
          </span>
          <button className="ov-close" onClick={() => api.overlayClose()}>
            <kbd>Esc</kbd> cerrar
          </button>
        </header>

        {rec && (
          <div className="ov-line" aria-hidden>
            <span className="ov-buf" style={{ right: 0, width: `${bufferWidth}%` }} />
            <div className="sig">
              {status!.live.map((x, i) => (
                <span
                  key={i}
                  className={`sig-tick ${x.kind === "highlight" ? "kill" : x.kind === "bookmark" ? "bookmark" : "kill"}`}
                  style={{ left: `${Math.min(100, (x.t / span) * 100)}%`, background: x.kind === "clip" ? "var(--text)" : undefined }}
                  title={`${x.label} · ${fmtClock(x.t)}`}
                />
              ))}
            </div>
            <span className="ov-now" />
            <div className="ov-line-labels">
              <span>00:00</span>
              <span>AHORA</span>
            </div>
          </div>
        )}

        <section className="ov-actions">
          <button ref={first} className="ov-action primary" onClick={() => api.saveClipNow()}>
            <span className="ov-action-top">
              <Icon name="download" size={22} />
              {hk.save_clip && <kbd>{key(hk.save_clip)}</kbd>}
            </span>
            <b>Guardar {settings.clip_seconds} s</b>
            <small>Lo último que pasó, al instante</small>
          </button>
          <button className="ov-action" onClick={() => api.addBookmark()} disabled={!rec}>
            <span className="ov-action-top">
              <Icon name="flag" size={22} />
              {hk.bookmark && <kbd>{key(hk.bookmark)}</kbd>}
            </span>
            <b>Marcar momento</b>
            <small>{rec ? "Queda señalado en la línea de tiempo" : "Disponible mientras grabas"}</small>
          </button>
          <button className="ov-action" onClick={() => api.toggleRecording()}>
            <span className="ov-action-top">
              <Icon name={rec ? "stop" : "record"} size={22} filled />
              {hk.toggle_recording && <kbd>{key(hk.toggle_recording)}</kbd>}
            </span>
            <b>{rec ? "Detener" : "Grabar ahora"}</b>
            <small>{rec ? "La partida se procesa al terminar" : "Sin esperar a que empiece una partida"}</small>
          </button>
        </section>

        <section>
          <div className="ov-recent-head label">
            <span>Clips recientes</span>
            <span>Se abren en ggswp</span>
          </div>
          {recent.length === 0 ? (
            <p className="muted small">Todavía no hay clips.</p>
          ) : (
            <div className="ov-recent">
              {recent.map(({ m, c }) => (
                <RecentClip key={`${m.id}/${c.id}`} m={m} c={c} />
              ))}
            </div>
          )}
        </section>

        <footer className="ov-foot">{key(hk.highlights_menu || "Alt+Shift+KeyG")} abre y cierra este menú</footer>
      </div>
    </div>
  );
}

function RecentClip({ m, c }: { m: Match; c: Clip }) {
  return (
    <button className="ov-clip" onClick={() => api.openInMain(m.id, c.id)} title="Abrir en ggswp">
      <div className={`vf ${c.thumb ? "" : "dots"}`}>{c.thumb && <img src={fileUrl(m, c.thumb)} alt="" />}</div>
      <span className="ov-clip-text">
        <b>{c.title}</b>
        <small>
          {fmtTime(c.end - c.start)} · {fmtDate(c.created_at)}
        </small>
      </span>
    </button>
  );
}
