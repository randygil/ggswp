import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { api, errText, fmtSize, QUALITY_LABEL, type Clip, type Firewall, type Match, type Quality, type ShareFile, type ShareLink } from "../api";
import { Icon } from "./Icon";
import { useToast } from "./Toast";
import { t, tr } from "../i18n";

type Tab = "file" | "wifi" | "drive";

const QUALITIES: Quality[] = ["share", "small", "original"];
const QUALITY_HINT: Record<Quality, string> = {
  share: "H.264 a 1080p, cerca de 1 MB por segundo. Se ve bien en WhatsApp, Instagram, Discord y cualquier teléfono.",
  small: "720p ajustado para no pasar de 16 MB, el tope habitual para mandar un video por WhatsApp Web o por correo.",
  original: "El archivo tal como está en tu biblioteca.",
};

export function ShareDialog({ match, clip, onClose }: { match: Match; clip?: Clip; onClose: () => void }) {
  // Si el archivo ya no está en la PC, solo se puede compartir desde Drive.
  const onlyDrive = clip ? !!clip.offloaded : match.offloaded;
  const toast = useToast();
  const [tab, setTab] = useState<Tab>(onlyDrive ? "drive" : "file");
  // La partida completa pesa gigas: no se comprime sin que lo pidas.
  const [quality, setQuality] = useState<Quality>(clip ? "share" : "original");
  const [files, setFiles] = useState<Partial<Record<Quality, ShareFile>>>({});
  const [preparing, setPreparing] = useState<Quality | null>(null);
  /** Formatos que no se pudieron preparar: no se reintentan solos. */
  const [failed, setFailed] = useState<Quality[]>([]);
  const [pct, setPct] = useState(0);
  const [lan, setLan] = useState<Partial<Record<Quality, ShareLink>>>({});
  const [firewall, setFirewall] = useState<Firewall | "fixing" | null>(null);
  const [drive, setDrive] = useState<ShareLink | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const alive = useRef(true);

  const key = `${match.id}/${clip?.id ?? "match"}`;
  const file = files[quality];
  const link = lan[quality];
  const adapter = link?.addresses?.[0]?.name;

  useEffect(() => {
    alive.current = true;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    const sub = listen<{ key: string; pct: number }>("share-progress", (e) => e.payload.key === key && setPct(e.payload.pct));
    return () => {
      alive.current = false;
      window.removeEventListener("keydown", onKey);
      sub.then((un) => un());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Deja listo el archivo en el formato elegido (lo comprime si hace falta).
  useEffect(() => {
    if (onlyDrive || files[quality] || preparing || failed.includes(quality)) return;
    setPreparing(quality);
    setPct(0);
    setError(null);
    api
      .sharePrepare(match.id, clip?.id, quality)
      .then((f) => alive.current && setFiles((x) => ({ ...x, [quality]: f })))
      .catch((e) => {
        if (!alive.current) return;
        setError(errText(e));
        setFailed((x) => [...x, quality]);
        // Vuelve a un formato que no necesita codificar.
        if (quality !== "original") setQuality("original");
      })
      .finally(() => alive.current && setPreparing(null));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [quality, preparing]);

  // El enlace Wi-Fi apunta al archivo del formato elegido.
  useEffect(() => {
    if (tab !== "wifi" || !file || lan[quality]) return;
    api
      .shareLan(match.id, clip?.id, quality)
      .then((l) => alive.current && setLan((x) => ({ ...x, [quality]: l })))
      .catch((e) => alive.current && setError(errText(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, file, quality]);

  useEffect(() => {
    if (tab !== "wifi" || firewall || !lan[quality]) return;
    api
      .shareFirewall(adapter)
      .then((f) => alive.current && setFirewall(f))
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, lan, quality]);

  const act = async (name: string, done: string, fn: () => Promise<unknown>) => {
    setBusy(name);
    setError(null);
    try {
      await fn();
      if (done) toast.info(done);
    } catch (e) {
      setError(errText(e));
    } finally {
      if (alive.current) setBusy(null);
    }
  };

  const saveAs = async () => {
    const name = (clip ? `${tr(match.title)} - ${tr(clip.title)}` : tr(match.title)).replace(/[\\/:*?"<>|]/g, "-");
    const dest = await save({ defaultPath: `${name}.mp4`, filters: [{ name: t("Video MP4"), extensions: ["mp4"] }] });
    if (dest) await act("save", t("Video guardado"), () => api.shareSave(match.id, clip?.id, quality, dest));
  };

  const pickAddress = (ip: string) =>
    api
      .shareLan(match.id, clip?.id, quality, ip)
      .then((l) => {
        // La elegida va primero: es la que muestra el selector.
        const all = l.addresses ?? [];
        setLan((x) => ({ ...x, [quality]: { ...l, addresses: [...all.filter((a) => a.ip === ip), ...all.filter((a) => a.ip !== ip)] } }));
      })
      .catch((e) => setError(errText(e)));

  const allowFirewall = () => {
    setFirewall("fixing");
    setError(null);
    api
      .shareFirewallAllow(adapter)
      .then((f) => alive.current && setFirewall(f))
      .catch((e) => {
        if (!alive.current) return;
        setError(errText(e));
        setFirewall("blocked");
      });
  };

  const loadDrive = () =>
    act("drive", "", async () => {
      const l = await api.driveShare(match.id, clip?.id);
      if (alive.current) setDrive(l);
    });

  // Lo que solo está en Drive ya tiene su archivo allá: el enlace sale al abrir.
  useEffect(() => {
    if (onlyDrive) loadDrive();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const waiting = (
    <div className="share-wait">
      <span className="spinner" />
      {preparing ? t("Comprimiendo el video… {pct} %", { pct: Math.round(pct) }) : t("Preparando…")}
    </div>
  );

  return (
    <div className="modal-back" onClick={onClose}>
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby="share-title" onClick={(e) => e.stopPropagation()}>
        <header>
          <h2 id="share-title">{t("Compartir")}</h2>
          <button className="icon-btn" onClick={onClose} aria-label={t("Cerrar")}>
            <Icon name="x" />
          </button>
        </header>
        <p className="muted">{clip ? tr(clip.title) : t("Partida completa · {title}", { title: tr(match.title) })}</p>

        {!onlyDrive && (
          <div className="share-format">
            <div className="chips" role="group" aria-label={t("Formato del video")}>
              {QUALITIES.map((q) => (
                <button key={q} className={`chip ${quality === q ? "on" : ""}`} aria-pressed={quality === q} disabled={!!preparing || failed.includes(q)} onClick={() => setQuality(q)}>
                  {t(QUALITY_LABEL[q])}
                  {files[q] && <span>{fmtSize(files[q]!.size_bytes)}</span>}
                </button>
              ))}
            </div>
            {preparing ? (
              <div className="step-bar" role="progressbar" aria-label={t("Comprimiendo el video")} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)}>
                <i style={{ width: `${pct}%` }} />
              </div>
            ) : (
              <p className="muted small">
                {file && (
                  <b className="share-size">
                    {fmtSize(file.size_bytes)}
                    {file.width > 0 && ` · ${file.width}×${file.height}`}
                    {file.size_bytes < file.original_bytes * 0.9 && ` · ${t("el original pesa {size}", { size: fmtSize(file.original_bytes) })}`}
                  </b>
                )}{" "}
                {t(QUALITY_HINT[quality])}
              </p>
            )}
          </div>
        )}

        <div className="tabs" role="tablist">
          <button role="tab" aria-selected={tab === "file"} className={tab === "file" ? "on" : ""} disabled={onlyDrive} onClick={() => setTab("file")}>
            {t("Archivo")}
          </button>
          <button role="tab" aria-selected={tab === "wifi"} className={tab === "wifi" ? "on" : ""} disabled={onlyDrive} onClick={() => setTab("wifi")}>
            {t("Al teléfono (Wi-Fi)")}
          </button>
          <button role="tab" aria-selected={tab === "drive"} className={tab === "drive" ? "on" : ""} onClick={() => setTab("drive")}>
            {t("Enlace de Drive")}
          </button>
        </div>

        <div className="share-body">
          {tab === "file" &&
            (file ? (
              <div className="share-file">
                <div className="share-actions">
                  <button className="btn primary" disabled={!!busy} onClick={() => act("copy", t("Video copiado: pégalo con Ctrl+V"), () => api.shareFile(match.id, clip?.id, quality, "copy"))}>
                    <Icon name="copy" size={16} /> {t("Copiar video")}
                  </button>
                  <button className="btn light" disabled={!!busy} onClick={() => act("system", "", () => api.shareFile(match.id, clip?.id, quality, "system"))}>
                    <Icon name="upload" size={16} /> {t("Compartir con…")}
                  </button>
                  <button className="btn ghost" disabled={!!busy} onClick={saveAs}>
                    <Icon name="download" size={16} /> {t("Guardar como…")}
                  </button>
                  <button className="btn ghost" disabled={!!busy} onClick={() => act("reveal", "", () => api.shareFile(match.id, clip?.id, quality, "reveal"))}>
                    <Icon name="folder" size={16} /> {t("Mostrar en carpeta")}
                  </button>
                </div>
                <ul className="share-tips muted small">
                  <li>
                    <b>{t("Copiar video")}</b> {t("y pegar con")} <kbd>Ctrl</kbd> + <kbd>V</kbd> {t("en el chat de WhatsApp, Discord o Telegram.")}
                  </li>
                  <li>
                    <b>{t("Compartir con…")}</b> {t("abre el panel de Windows: apps instaladas, Vínculo Móvil y dispositivos cercanos.")}
                  </li>
                  <li>
                    {t("Para Instagram o TikTok desde la PC, usa")} <b>{t("Mostrar en carpeta")}</b> {t("y arrastra el archivo a la página de subida.")}
                  </li>
                </ul>
              </div>
            ) : error && !preparing ? null : (
              waiting
            ))}

          {tab === "wifi" &&
            (file && link ? (
              <>
                <div className="qr" dangerouslySetInnerHTML={{ __html: link.qr_svg }} />
                <div className="share-text">
                  <p>{t("Escanea con la cámara del teléfono conectado a la misma red. Se abre el video con un botón para descargarlo ({size}).", { size: fmtSize(file.size_bytes) })}</p>
                  <div className="url-row">
                    <input className="input" readOnly value={link.url} onFocus={(e) => e.target.select()} aria-label={t("Enlace para el teléfono")} />
                    <button className="btn ghost" onClick={() => navigator.clipboard.writeText(link.url).then(() => (setCopied(true), setTimeout(() => setCopied(false), 1500)))}>
                      <Icon name={copied ? "check" : "copy"} /> {copied ? t("Copiado") : t("Copiar")}
                    </button>
                  </div>
                  {(link.addresses?.length ?? 0) > 1 && (
                    <label className="sort" style={{ marginLeft: 0 }}>
                      {t("Red")}
                      <select value={link.addresses![0].ip} onChange={(e) => pickAddress(e.target.value)}>
                        {link.addresses!.map((a) => (
                          <option key={a.ip} value={a.ip}>
                            {a.name ? `${a.name} · ${a.ip}` : a.ip}
                          </option>
                        ))}
                      </select>
                    </label>
                  )}
                  {firewall === "allowed" ? (
                    <p className="muted small">{t("El firewall de Windows permite la conexión. Si el teléfono no abre el enlace, revisa que esté en el mismo Wi-Fi y, si usas una VPN en la PC, que permita el tráfico de la red local.")}</p>
                  ) : firewall ? (
                    <div className="share-warn" role="alert">
                      <p>
                        {firewall === "blocked"
                          ? t("El firewall de Windows está bloqueando a ggswp: el teléfono no puede abrir este enlace.")
                          : firewall === "fixing"
                            ? t("Acepta el aviso de Windows para cambiar el firewall…")
                            : t("El firewall de Windows todavía no permite a ggswp en esta red: es probable que el teléfono no pueda abrir el enlace.")}
                      </p>
                      <button className="btn light sm" disabled={firewall === "fixing"} onClick={allowFirewall}>
                        {firewall === "fixing" ? <span className="spinner" /> : <Icon name="check" size={15} />} {t("Permitir en el firewall")}
                      </button>
                    </div>
                  ) : null}
                </div>
              </>
            ) : error && !preparing ? null : (
              waiting
            ))}

          {tab === "drive" &&
            (drive ? (
              <>
                <div className="qr" dangerouslySetInnerHTML={{ __html: drive.qr_svg }} />
                <div className="share-text">
                  <p>{t("Cualquiera con este enlace puede verlo y descargarlo desde Drive, desde cualquier red.")}</p>
                  <div className="url-row">
                    <input className="input" readOnly value={drive.url} onFocus={(e) => e.target.select()} aria-label={t("Enlace de Drive")} />
                    <button className="btn ghost" onClick={() => navigator.clipboard.writeText(drive.url).then(() => (setCopied(true), setTimeout(() => setCopied(false), 1500)))}>
                      <Icon name={copied ? "check" : "copy"} /> {copied ? t("Copiado") : t("Copiar")}
                    </button>
                  </div>
                </div>
              </>
            ) : busy === "drive" ? (
              <div className="share-wait">
                <span className="spinner" /> {t("Subiendo a Drive y creando el enlace…")}
              </div>
            ) : (
              <div className="share-wait">
                <p>{t("Se sube el video a tu Drive (carpeta ggswp) y se crea un enlace para verlo. Drive guarda su propia copia comprimida, la que eliges en Ajustes.")}</p>
                <button className="btn primary" onClick={loadDrive}>
                  <Icon name="cloud" /> {t("Crear enlace de Drive")}
                </button>
              </div>
            ))}
        </div>
        {error && <p className="error-text">{error}</p>}
      </div>
    </div>
  );
}
