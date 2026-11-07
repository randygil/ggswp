import { useEffect, useState } from "react";
import { api, errText, type Clip, type Match, type ShareLink } from "../api";
import { Icon } from "./Icon";

type Tab = "wifi" | "drive";

export function ShareDialog({ match, clip, onClose }: { match: Match; clip?: Clip; onClose: () => void }) {
  // Si el archivo ya no está en la PC, solo se puede compartir desde Drive.
  const onlyDrive = clip ? !!clip.offloaded : match.offloaded;
  const [tab, setTab] = useState<Tab>(onlyDrive ? "drive" : "wifi");
  const [links, setLinks] = useState<Partial<Record<Tab, ShareLink>>>({});
  const [busy, setBusy] = useState<Tab | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const load = async (t: Tab) => {
    setBusy(t);
    setError(null);
    try {
      const l = t === "wifi" ? await api.shareLan(match.id, clip?.id) : await api.driveShare(match.id, clip?.id);
      setLinks((x) => ({ ...x, [t]: l }));
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(null);
    }
  };

  useEffect(() => {
    load(onlyDrive ? "drive" : "wifi");
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const link = links[tab];

  return (
    <div className="modal-back" onClick={onClose}>
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby="share-title" onClick={(e) => e.stopPropagation()}>
        <header>
          <h2 id="share-title">Enviar al teléfono</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Cerrar">
            <Icon name="x" />
          </button>
        </header>
        <p className="muted">{clip ? clip.title : `Partida completa · ${match.title}`}</p>
        <div className="tabs" role="tablist">
          <button role="tab" aria-selected={tab === "wifi"} className={tab === "wifi" ? "on" : ""} onClick={() => setTab("wifi")}>
            Wi-Fi (misma red)
          </button>
          <button role="tab" aria-selected={tab === "drive"} className={tab === "drive" ? "on" : ""} onClick={() => setTab("drive")}>
            Enlace de Google Drive
          </button>
        </div>

        <div className="share-body">
          {link ? (
            <>
              <div className="qr" dangerouslySetInnerHTML={{ __html: link.qr_svg }} />
              <div className="share-text">
                <p>
                  {tab === "wifi"
                    ? "Escanea con la cámara del teléfono conectado al mismo Wi-Fi. Se abre el video con un botón para descargarlo."
                    : "Cualquiera con este enlace puede verlo y descargarlo desde Drive, desde cualquier red."}
                </p>
                <div className="url-row">
                  <input className="input" readOnly value={link.url} onFocus={(e) => e.target.select()} />
                  <button
                    className="btn ghost"
                    onClick={() => navigator.clipboard.writeText(link.url).then(() => (setCopied(true), setTimeout(() => setCopied(false), 1500)))}
                  >
                    <Icon name={copied ? "check" : "copy"} /> {copied ? "Copiado" : "Copiar"}
                  </button>
                </div>
                {tab === "wifi" && <p className="muted small">Si Windows pregunta por el firewall, permite el acceso en redes privadas.</p>}
              </div>
            </>
          ) : busy === tab ? (
            <div className="share-wait">
              <span className="spinner" /> {tab === "drive" ? "Subiendo a Drive y creando el enlace…" : "Preparando enlace…"}
            </div>
          ) : tab === "drive" ? (
            <div className="share-wait">
              <p>Se sube el video a tu Drive (carpeta ggswp) y se crea un enlace para verlo.</p>
              <button className="btn primary" onClick={() => load("drive")}>
                <Icon name="cloud" /> Crear enlace de Drive
              </button>
            </div>
          ) : null}
          {error && <p className="error-text">{error}</p>}
        </div>
      </div>
    </div>
  );
}
