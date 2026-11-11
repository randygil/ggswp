import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { errText } from "../api";
import { t } from "../i18n";
import { Icon } from "./Icon";

const EVERY_MS = 6 * 60 * 60 * 1000;

/** Busca versiones nuevas en GitHub al abrir la app y cada 6 horas. */
export function UpdateBanner({ busy }: { busy: boolean }) {
  const [update, setUpdate] = useState<Update | null>(null);
  const [hidden, setHidden] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (import.meta.env.DEV) return;
    let alive = true;
    const run = () =>
      check()
        .then((u) => {
          if (alive && u) {
            setUpdate(u);
            setHidden(false);
          }
        })
        .catch(() => {});
    const first = setTimeout(run, 15_000);
    const timer = setInterval(run, EVERY_MS);
    return () => {
      alive = false;
      clearTimeout(first);
      clearInterval(timer);
    };
  }, []);

  if (!update || hidden) return null;

  const install = async () => {
    setError(null);
    setProgress(0);
    let total = 0;
    let got = 0;
    try {
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") total = e.data.contentLength ?? 0;
        if (e.event === "Progress") {
          got += e.data.chunkLength;
          setProgress(total ? Math.round((got / total) * 100) : null);
        }
      });
      await relaunch();
    } catch (e) {
      setProgress(null);
      setError(errText(e));
    }
  };

  return (
    <aside className="update" role="status" aria-live="polite">
      <div className="update-head">
        <span className="label">{t("Actualización")}</span>
        <b>ggswp {update.version}</b>
        {progress === null && (
          <button className="icon-btn sm bare" onClick={() => setHidden(true)} aria-label={t("Recordármelo después")}>
            <Icon name="x" size={16} />
          </button>
        )}
      </div>
      {update.body && <p className="update-notes">{update.body}</p>}
      {error && <p className="update-error">{t("No se pudo actualizar: {error}", { error })}</p>}
      {progress !== null ? (
        <div className="update-progress">
          <span>{progress !== null ? t("Descargando · {pct} %", { pct: progress }) : t("Descargando…")}</span>
          <div className="meter">
            <div style={{ width: `${progress ?? 0}%` }} />
          </div>
        </div>
      ) : (
        <button className="btn primary sm full" onClick={install} disabled={busy} title={busy ? t("Espera a que termine la grabación") : undefined}>
          <Icon name="download" size={14} /> {busy ? t("Disponible al terminar de grabar") : t("Actualizar y reiniciar")}
        </button>
      )}
    </aside>
  );
}
