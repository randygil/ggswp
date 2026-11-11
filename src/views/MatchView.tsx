import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  api,
  daysLeft,
  deathAt,
  errText,
  EVENT_COLORS,
  EVENT_KIND,
  expiryText,
  fileUrl,
  fmtClock,
  fmtDate,
  fmtSize,
  onTimeline,
  pad2,
  QUALITY_LABEL,
  type Clip,
  type Frame,
  type Fx,
  type Match,
  type MatchEvent,
  type MobileLayout,
  type MobilePreset,
  type Quality,
  type Step,
  type UploadProgress,
} from "../api";
import { useContextMenu } from "../components/ContextMenu";
import { Icon } from "../components/Icon";
import { EMPTY_FX, FxPanel, hasFx, TextPreview, toFx, type FxState } from "../components/EditorFx";
import { DEFAULT_MOBILE_LAYOUT, FREE_EXTRA, MINIMAP_EXTRA, MobileOverlay, MobilePreview } from "../components/MobileFrame";
import { clipMenuItems, eventMenuItems, makeVertical, selectAround, useRun } from "../components/menus";
import { ShareDialog } from "../components/ShareDialog";
import { rulerStep } from "../components/Signature";
import { StepProgress } from "../components/StepProgress";
import { useToast } from "../components/Toast";
import { locale, t, tr } from "../i18n";
import { hasPlacement, summaryRows } from "../matchStats";

type Source = { kind: "match" } | { kind: "clip"; clip: Clip };

/** Decimales con coma en español y con punto en inglés. */
const dec = (s: string) => (locale() === "en" ? s : s.replace(".", ","));

export function MatchView(props: {
  id: string;
  initialClip?: string;
  matches: Match[];
  uploads: Record<string, UploadProgress>;
  retentionDays?: number;
  localDays?: number;
  /** Formato en que se crean los clips (Ajustes): el que propone el editor. */
  clipQuality?: Quality;
  /** Paso en curso si la partida se está procesando o re-analizando. */
  step?: Step;
  onBack: () => void;
}) {
  const { id, matches, uploads, retentionDays, localDays, clipQuality, step, onBack } = props;
  const m = matches.find((x) => x.id === id);
  const toast = useToast();
  const menu = useContextMenu();
  const [source, setSource] = useState<Source>(() => {
    const c = m?.clips.find((c) => c.id === props.initialClip);
    return c ? { kind: "clip", clip: c } : { kind: "match" };
  });
  const [share, setShare] = useState<{ clip?: Clip } | null>(null);
  const [tab, setTab] = useState<"events" | "clips">(props.initialClip ? "clips" : "events");
  const [editingTitle, setEditingTitle] = useState(false);
  // Clips elegidos para unir, en orden; `null` = no se está uniendo.
  const [joining, setJoining] = useState<string[] | null>(null);

  // Si el clip activo se borra, volvemos a la partida completa.
  useEffect(() => {
    if (source.kind === "clip" && m && !m.clips.some((c) => c.id === source.clip.id)) setSource({ kind: "match" });
  }, [m, source]);

  // Un clip recién creado (resumen, vertical) se abre cuando llega en la lista de la partida.
  const [pendingClip, setPendingClip] = useState<string | null>(null);
  useEffect(() => {
    const c = pendingClip ? m?.clips.find((c) => c.id === pendingClip) : undefined;
    if (c) {
      setSource({ kind: "clip", clip: c });
      setPendingClip(null);
    }
  }, [m, pendingClip]);

  if (!m) {
    return (
      <div className="page">
        <button className="btn ghost" onClick={onBack} style={{ alignSelf: "flex-start" }}>
          <Icon name="arrowLeft" /> {t("Volver")}
        </button>
        <div className="empty">
          <h2>{t("Partida no encontrada")}</h2>
        </div>
      </div>
    );
  }

  const activeClip = source.kind === "clip" ? m.clips.find((c) => c.id === source.clip.id) ?? source.clip : undefined;
  const onlyDrive = activeClip ? !!activeClip.offloaded : m.offloaded;
  const src = onlyDrive
    ? null
    : activeClip
      ? fileUrl(m, activeClip.file)
      : m.video && m.status === "ready"
        ? fileUrl(m, m.video, m.size_bytes)
        : null;
  const matchUpload = uploads[`${m.id}/match`];
  const activeTransfer = uploads[`${m.id}/${activeClip ? activeClip.id : "match"}`];
  const died = deathAt(m);
  const left = daysLeft(m, retentionDays);
  const localLeft = m.drive && !m.offloaded ? daysLeft(m, localDays) : null;
  const inDrive = !!m.drive || m.clips.some((c) => c.drive);

  const run = async (label: string, fn: () => Promise<unknown>) => {
    try {
      await fn();
      if (label) toast.info(label);
    } catch (e) {
      toast.error(errText(e));
    }
  };

  // Desde un clip, primero se vuelve a la partida completa y se espera a que monte el reproductor.
  const inMatch = (fn: () => void) => {
    if (source.kind === "clip") {
      setSource({ kind: "match" });
      setTimeout(fn, 150);
    } else fn();
  };
  const canClip = m.status === "ready" && !m.offloaded && !!m.video;
  const hasReel = m.clips.some((c) => c.kind === "reel");
  // Al terminar, se abre el clip nuevo en la pestaña de clips.
  const showClip = (c: Clip) => {
    setPendingClip(c.id);
    setTab("clips");
  };
  const createReel = () =>
    run(t("Resumen listo"), async () => {
      showClip(await api.createReel(m.id));
    });
  const eventMenu =(ev: React.MouseEvent, e: MatchEvent) =>
    menu.open(
      ev,
      eventMenuItems(e.t, {
        seek: () => inMatch(() => window.dispatchEvent(new CustomEvent("ggswp-seek", { detail: Math.max(0, e.t - 3) }))),
        makeClip: canClip ? () => inMatch(() => selectAround(e.t)) : undefined,
        run,
      }),
    );

  return (
    <div className="page match-page">
      <header className="match-head">
        <button className="icon-btn" onClick={onBack} aria-label={t("Volver a partidas")}>
          <Icon name="arrowLeft" />
        </button>
        <div className="match-title">
          {editingTitle ? (
            <input
              className="title-input"
              autoFocus
              aria-label={t("Nombre de la partida")}
              defaultValue={m.title}
              onBlur={(e) => {
                setEditingTitle(false);
                if (e.target.value !== m.title) run("", () => api.updateMatch(m.id, { title: e.target.value }));
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                if (e.key === "Escape") setEditingTitle(false);
              }}
            />
          ) : (
            <h1 onDoubleClick={() => setEditingTitle(true)} title={t("Doble clic para renombrar")}>
              {m.victory && !m.title.startsWith("Victoria") && (
                <>
                  {t("Victoria")} <span className="accent">·</span>{" "}
                </>
              )}
              {tr(m.title)}
            </h1>
          )}
          <p>
            {fmtDate(m.created_at)}
            {m.duration_s > 0 && ` · ${fmtClock(m.duration_s)}`}
            {m.mode && ` · ${m.mode}`}
            {hasPlacement(m) && ` · #${m.placement}${m.total_players ? `/${m.total_players}` : ""}`}
            {m.width > 0 && ` · ${m.width}×${m.height}`}
            {m.size_bytes > 0 && ` · ${fmtSize(m.size_bytes)}`}
            {m.game ? ` · ${m.game}` : ` · ${t("{n} elim", { n: pad2(m.kills) })}`}
            {m.chests > 0 && ` · ${t("{n} cofres", { n: pad2(m.chests) })}`}
            {localLeft !== null && (left === null || localLeft < left) && ` · ${localLeft <= 0 ? t("sale de la PC hoy") : localLeft === 1 ? t("sale de la PC mañana") : t("sale de la PC en {n} días", { n: localLeft })}`}
            {m.keep ? ` · ${t("conservada")}` : left !== null ? ` · ${expiryText(left).toLocaleLowerCase(locale())}` : ""}
            {m.offloaded && ` · ${t("solo en Drive")}`}
          </p>
        </div>
        <div className="head-actions">
          <button
            className={`icon-btn ${m.keep ? "fav" : ""}`}
            onClick={() => run(m.keep ? t("Ya no está conservada") : t("Partida conservada: no vencerá"), () => api.updateMatch(m.id, { keep: !m.keep }))}
            aria-label={m.keep ? t("Dejar de conservar") : t("Conservar partida")}
            aria-pressed={m.keep}
            title={m.keep ? t("Conservada: no vence. Clic para dejar de conservarla") : t("Conservar: la partida no vencerá")}
          >
            <Icon name="keep" filled={m.keep} />
          </button>
          <button className="icon-btn" onClick={() => setEditingTitle(true)} title={t("Renombrar")} aria-label={t("Renombrar")}>
            <Icon name="edit" />
          </button>
          <button className="icon-btn" onClick={() => run("", () => api.reveal(m.id))} title={t("Abrir carpeta")} aria-label={t("Abrir carpeta")}>
            <Icon name="folder" />
          </button>
          <button
            className="icon-btn"
            disabled={m.offloaded}
            onClick={() => run(t("Partida analizada de nuevo"), () => api.reprocess(m.id))}
            title={m.offloaded ? t("Descarga el video para volver a analizarlo") : t("Volver a leer el replay y regenerar clips")}
            aria-label={t("Volver a analizar")}
          >
            <Icon name="refresh" />
          </button>
          <button
            className="icon-btn"
            disabled={!!matchUpload}
            onClick={() => (m.drive ? run("", () => api.driveOpen(m.id)) : run(t("Subida a Drive completa"), () => api.driveUpload(m.id)))}
            title={m.drive ? t("Abrir en Google Drive") : t("Subir partida completa a Drive")}
            aria-label={m.drive ? t("Abrir en Google Drive") : t("Subir a Google Drive")}
          >
            <Icon name={m.drive ? "check" : "cloud"} />
          </button>
          <button
            className="icon-btn danger"
            onClick={() => {
              const msg = inDrive
                ? t("¿Borrar esta partida y sus clips? También se mandan a la papelera de Google Drive.")
                : t("¿Borrar esta partida y todos sus clips del disco?");
              if (confirm(msg)) run(t("Partida borrada"), () => api.deleteMatch(m.id)).then(onBack);
            }}
            title={t("Borrar")}
            aria-label={t("Borrar partida")}
          >
            <Icon name="trash" />
          </button>
          <button
            className="btn ghost"
            disabled={!canClip || !!step}
            onClick={createReel}
            title={m.offloaded ? t("Descarga el video para crear un resumen") : t("Junta las eliminaciones y los mejores momentos en un solo video")}
          >
            <Icon name="film" size={16} /> {hasReel ? t("Rehacer resumen") : t("Crear resumen")}
          </button>
          <button className="btn light" onClick={() => setShare({})} title={t("Compartir el video de la partida completa")}>
            <Icon name="upload" size={16} /> {t("Compartir partida")}
          </button>
        </div>
      </header>

      <div className="match-layout">
        <section className="player-col">
          {activeClip && (
            <div className="clip-bar">
              <button className="btn ghost sm" onClick={() => setSource({ kind: "match" })} title={t("Volver al video de la partida completa")}>
                <Icon name="arrowLeft" size={15} /> {t("Partida completa")}
              </button>
              <span className="clip-bar-title">
                <span className="label">{t("Viendo el clip")}</span>
                <b>{tr(activeClip.title)}</b>
                <small>
                  {fmtClock(activeClip.end - activeClip.start)}
                  {activeClip.kind === "vertical" && " · 9:16"}
                  {m.clip_sizes?.[activeClip.id] != null && ` · ${fmtSize(m.clip_sizes[activeClip.id])}`}
                </small>
              </span>
              <div className="clip-bar-actions">
                <button className="btn primary sm" onClick={() => setShare({ clip: activeClip })}>
                  <Icon name="upload" size={15} /> {t("Compartir clip")}
                </button>
                <button
                  className="icon-btn sm"
                  title={t("Mostrar el clip en la carpeta")}
                  aria-label={t("Mostrar el clip en la carpeta")}
                  disabled={!!activeClip.offloaded}
                  onClick={() => run("", () => api.reveal(m.id, activeClip.id))}
                >
                  <Icon name="folder" size={16} />
                </button>
                <button
                  className="icon-btn sm danger"
                  title={t("Borrar este clip")}
                  aria-label={t("Borrar este clip")}
                  onClick={() =>
                    confirm(t(activeClip.drive ? '¿Borrar el clip "{title}"? También se manda a la papelera de Drive.' : '¿Borrar el clip "{title}"?', { title: tr(activeClip.title) })) &&
                    run(t("Clip borrado"), () => api.deleteClip(m.id, activeClip.id))
                  }
                >
                  <Icon name="trash" size={16} />
                </button>
              </div>
            </div>
          )}
          {src ? (
            <Player
              key={src}
              src={src}
              match={m}
              clip={activeClip}
              defaultQuality={clipQuality ?? "share"}
              busy={!!step}
              onExport={async (o) => {
                const c = await api.exportClip(m.id, { ...o, source: activeClip?.id });
                toast.info(t('Clip "{title}" guardado', { title: tr(c.title) }));
                showClip(c);
              }}
            />
          ) : onlyDrive ? (
            <div className={`vf big player-ph drive-ph ${m.thumb ? "" : "dots"}`}>
              {(activeClip?.thumb ?? m.thumb) && <img src={fileUrl(m, (activeClip?.thumb ?? m.thumb)!)} alt="" />}
              <div className="drive-ph-body">
                <span className="label">{t("Solo en Google Drive")}</span>
                <p>
                  {activeClip
                    ? t("Este clip se borró de la PC después de subirse. Puedes verlo en Drive o descargarlo de nuevo para editarlo.")
                    : t("Esta partida se borró de la PC después de subirse. Puedes verlo en Drive o descargarlo de nuevo para editarlo.")}
                </p>
                {activeTransfer ? (
                  <span className="mono">
                    <span className="spinner" /> {t("Descargando… {n} %", { n: Math.round((activeTransfer.sent / Math.max(1, activeTransfer.total)) * 100) })}
                  </span>
                ) : (
                  <div className="drive-ph-actions">
                    <button className="btn light" onClick={() => run("", () => api.driveOpen(m.id, activeClip?.id))}>
                      <Icon name="cloud" size={16} /> {t("Ver en Drive")}
                    </button>
                    <button className="btn ghost" onClick={() => run(t("Descargado de nuevo a la PC"), () => api.driveDownload(m.id, activeClip?.id))}>
                      <Icon name="download" size={16} /> {t("Descargar a la PC")}
                    </button>
                  </div>
                )}
              </div>
            </div>
          ) : (
            <div className="vf big dots player-ph">
              {m.status === "error" ? (
                <span className="flare">{t("Error: {msg}", { msg: tr(m.error ?? "") })}</span>
              ) : (
                m.status === "recording" ? (
                  <>
                    <span className="spinner" /> {t("Grabando…")}
                  </>
                ) : (
                  <StepProgress step={step} fallback={t("Procesando la partida…")} />
                )
              )}
            </div>
          )}
          {src && step && (
            <div className="source-bar">
              <StepProgress step={step} fallback={t("Analizando…")} />
            </div>
          )}
          {matchUpload && (
            <div className="source-bar">
              <span>{t(matchUpload.download ? "Bajando la partida de Drive… {n} %" : "Subiendo partida a Drive… {n} %", { n: Math.round((matchUpload.sent / Math.max(1, matchUpload.total)) * 100) })}</span>
            </div>
          )}
        </section>

        <aside className="side-col">
          <dl className="side-stats">
            <div>
              <dt>{t("Eliminaciones")}</dt>
              <dd>{pad2(m.kills)}</dd>
            </div>
            <div>
              <dt>{t("Cofres")}</dt>
              <dd>{pad2(m.chests)}</dd>
            </div>
            <div>
              <dt>{t("Derribos")}</dt>
              <dd>{pad2(m.knocks)}</dd>
            </div>
            {hasPlacement(m) ? (
              <div>
                <dt>{m.placement === 1 ? t("Victoria") : t("Puesto")}</dt>
                <dd className={m.placement === 1 ? "accent" : ""}>
                  #{m.placement}
                  {m.total_players != null && <small> / {m.total_players}</small>}
                </dd>
              </div>
            ) : (
              <div>
                <dt>{m.victory ? t("Victoria") : died !== null ? t("Caída") : t("Resultado")}</dt>
                <dd className={m.victory ? "accent" : died !== null ? "flare" : ""}>{m.victory ? "#1" : died !== null ? fmtClock(died) : "—"}</dd>
              </div>
            )}
          </dl>
          {summaryRows(m).length > 0 && (
            <dl className="side-more" aria-label={t("Resumen de fin de partida")}>
              {hasPlacement(m) && died !== null && m.placement !== 1 && (
                <div>
                  <dt>{t("Caída")}</dt>
                  <dd className="flare">{fmtClock(died)}</dd>
                </div>
              )}
              {summaryRows(m).map(([label, value]) => (
                <div key={label}>
                  <dt>{t(label)}</dt>
                  <dd>{value}</dd>
                </div>
              ))}
            </dl>
          )}
          <div className="tabs" role="tablist" aria-label={t("Contenido de la partida")}>
            <button role="tab" aria-selected={tab === "events"} className={tab === "events" ? "on" : ""} onClick={() => setTab("events")}>
              {t("Eventos")} <span>{pad2(m.events.length)}</span>
            </button>
            <button role="tab" aria-selected={tab === "clips"} className={tab === "clips" ? "on" : ""} onClick={() => setTab("clips")}>
              {t("Clips")} <span>{pad2(m.clips.length)}</span>
            </button>
          </div>
          {tab === "clips" && m.clips.filter((c) => !c.offloaded).length >= 2 && (
            <div className="join-bar">
              {joining ? (
                <>
                  <span className="muted small">{joining.length < 2 ? t("Elige los clips en el orden en que van.") : t("{n} clips, en ese orden.", { n: joining.length })}</span>
                  <button
                    className="btn primary sm"
                    disabled={joining.length < 2 || !!step}
                    onClick={() => {
                      const ids = joining;
                      setJoining(null);
                      run(t("Clips unidos"), async () => showClip(await api.joinClips(m.id, ids)));
                    }}
                  >
                    <Icon name="film" size={15} /> {t("Unir")}
                  </button>
                  <button className="btn ghost sm" onClick={() => setJoining(null)}>
                    {t("Cancelar")}
                  </button>
                </>
              ) : (
                <button className="btn ghost sm" disabled={!!step} onClick={() => setJoining([])} title={t("Arma un video con varios clips, en el orden que elijas")}>
                  <Icon name="film" size={15} /> {t("Unir clips")}
                </button>
              )}
            </div>
          )}
          {tab === "events" ? (
            <ol className="event-list">
              {m.events.length === 0 && <li className="muted pad">{t("Sin eventos detectados en esta partida.")}</li>}
              {m.events.map((e, i) => (
                <li key={i} onContextMenu={(ev) => eventMenu(ev, e)}>
                  <button
                    className="ev-row"
                    onClick={() => {
                      setSource({ kind: "match" });
                      window.dispatchEvent(new CustomEvent("ggswp-seek", { detail: Math.max(0, e.t - 3) }));
                    }}
                  >
                    <span className="ev-time">{fmtClock(e.t)}</span>
                    <span className="ev-bar" style={{ background: EVENT_COLORS[e.kind] }} />
                    <span className="ev-text">
                      <b>{tr(e.label)}</b>
                      <small>
                        {t(EVENT_KIND[e.kind] ?? e.kind)}
                        {e.distance_m ? ` · ${t("a {n} m", { n: Math.round(e.distance_m) })}` : ""}
                      </small>
                    </span>
                  </button>
                </li>
              ))}
            </ol>
          ) : (
            <ol className="clip-list">
              {m.clips.length === 0 && <li className="muted pad">{t("Sin clips. Usa «Nuevo clip» en el editor, ajusta el tramo y expórtalo.")}</li>}
              {m.clips.map((c) => {
                const up = uploads[`${m.id}/${c.id}`];
                const active = source.kind === "clip" && source.clip.id === c.id;
                return (
                  <li
                    key={c.id}
                    className={active ? "active" : ""}
                    onContextMenu={(e) =>
                      menu.open(
                        e,
                        clipMenuItems(m, c, { play: () => setSource({ kind: "clip", clip: c }), share: () => setShare({ clip: c }), run, busy: !!up, onCreated: showClip }),
                      )
                    }
                  >
                    <button
                      className="clip-main"
                      disabled={!!joining && !!c.offloaded}
                      onClick={() => (joining ? setJoining(joining.includes(c.id) ? joining.filter((x) => x !== c.id) : [...joining, c.id]) : setSource({ kind: "clip", clip: c }))}
                      aria-current={active}
                      aria-pressed={joining ? joining.includes(c.id) : undefined}
                    >
                      <div className={`vf ${c.thumb ? "" : "dots"}`}>
                        {c.thumb && <img src={fileUrl(m, c.thumb)} alt="" />}
                        {joining?.includes(c.id) && <span className="join-n">{joining.indexOf(c.id) + 1}</span>}
                      </div>
                      <span>
                        <b>
                          {c.best && (
                            <i className="best" title={t("La mejor jugada de la partida")}>
                              ★{" "}
                            </i>
                          )}
                          {tr(c.title)}
                        </b>
                        <small>
                          {onTimeline(c) ? `${fmtClock(c.start)} → ${fmtClock(c.end)} · ${fmtClock(c.end - c.start)}` : `${c.kind === "reel" ? `${t("Resumen")} · ` : c.kind === "join" ? `${t("Unión")} · ` : ""}${fmtClock(c.end - c.start)}`}
                          {c.kind === "vertical" && " · 9:16"}
                          {m.clip_sizes?.[c.id] != null && ` · ${fmtSize(m.clip_sizes[c.id])}`}
                          {up &&` · ${t(up.download ? "bajando {n} %" : "subiendo {n} %", { n: Math.round((up.sent / Math.max(1, up.total)) * 100) })}`}
                          {c.offloaded && !up && ` · ${t("solo en Drive")}`}
                        </small>
                      </span>
                    </button>
                    <div className="clip-actions">
                      <button className="icon-btn sm bare" title={t("Compartir")} aria-label={t("Compartir {title}", { title: tr(c.title) })} onClick={() => setShare({ clip: c })}>
                        <Icon name="upload" size={16} />
                      </button>
                      {c.kind !== "vertical" && (
                        <button
                          className="icon-btn sm bare"
                          title={t("Versión vertical (TikTok, Shorts, Reels)")}
                          aria-label={t("Crear versión vertical de {title}", { title: tr(c.title) })}
                          disabled={!!c.offloaded || !!step || m.status !== "ready"}
                          onClick={() => makeVertical(m, c, run, showClip)}
                        >
                          <Icon name="phone" size={16} />
                        </button>
                      )}
                      <button
                        className="icon-btn sm bare"
                        title={c.drive ? t("Abrir en Google Drive") : t("Subir a Drive")}
                        aria-label={t(c.drive ? "Abrir {title} en Drive" : "Subir {title} a Drive", { title: tr(c.title) })}
                        disabled={!!up}
                        onClick={() => (c.drive ? run("", () => api.driveOpen(m.id, c.id)) : run(t("Clip subido a Drive"), () => api.driveUpload(m.id, c.id)))}
                      >
                        <Icon name={c.drive ? "check" : "cloud"} size={16} />
                      </button>
                      <button
                        className="icon-btn sm bare"
                        title={t("Mostrar en carpeta")}
                        disabled={!!c.offloaded}
                        aria-label={t("Mostrar {title} en la carpeta", { title: tr(c.title) })}
                        onClick={() => run("", () => api.reveal(m.id, c.id))}
                      >
                        <Icon name="folder" size={16} />
                      </button>
                      <button
                        className="icon-btn sm bare danger"
                        title={t("Borrar clip")}
                        aria-label={t("Borrar {title}", { title: tr(c.title) })}
                        onClick={() =>
                          confirm(t(c.drive ? '¿Borrar el clip "{title}"? También se manda a la papelera de Drive.' : '¿Borrar el clip "{title}"?', { title: tr(c.title) })) &&
                          run(t("Clip borrado"), () => api.deleteClip(m.id, c.id))
                        }
                      >
                        <Icon name="trash" size={16} />
                      </button>
                    </div>
                  </li>
                );
              })}
            </ol>
          )}
        </aside>
      </div>

      {share && <ShareDialog match={m} clip={share.clip} onClose={() => setShare(null)} />}
    </div>
  );
}

/** Las etiquetas centradas sobre el cursor no se salen de la pista en los extremos. */
const edgeSafe = (p: string) => `clamp(26px, ${p}, calc(100% - 26px))`;

type ExportOptions = { start: number; end: number; title: string; quality: Quality; frame: Frame; fx: Fx };

function Player(props: {
  src: string;
  match: Match;
  /** Clip que se está viendo; sin él, la partida completa. */
  clip?: Clip;
  defaultQuality: Quality;
  /** La partida está ocupada con otro paso (análisis, resumen…). */
  busy: boolean;
  onExport: (o: ExportOptions) => Promise<void>;
}) {
  const { src, match, clip, defaultQuality, busy, onExport } = props;
  const video = useRef<HTMLVideoElement>(null);
  const wrap = useRef<HTMLDivElement>(null);
  const bar = useRef<HTMLDivElement>(null);
  const [pos, setT] = useState(0);
  const [dur, setDur] = useState(clip ? clip.end - clip.start : match.duration_s);
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(false);
  const [rate, setRate] = useState(1);
  const [range, setRange] = useState<{ a: number; b: number } | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [title, setTitle] = useState("");
  const [exporting, setExporting] = useState(false);
  const [quality, setQuality] = useState<Quality>(defaultQuality);
  const [frame, setFrame] = useState<Frame>("horizontal");
  // Un clip vertical se recorta tal cual: no se puede volver a encuadrar.
  const canFrame = clip?.kind !== "vertical";
  // Las zonas se ven solo mientras hay algo que exportar.
  const mobile = frame === "mobile" && canFrame && (!!range || !!clip);
  // Zonas del modo móvil: se guardan en los ajustes al soltar cada cambio.
  const [layout, setLayout] = useState<MobileLayout>(DEFAULT_MOBILE_LAYOUT);
  const [presets, setPresets] = useState<MobilePreset[]>([]);
  // Nombre de la plantilla que se está por guardar; `null` = no se está guardando.
  const [naming, setNaming] = useState<string | null>(null);
  // Efectos del clip que se está armando.
  const [fx, setFx] = useState<FxState>(EMPTY_FX);
  const [showFx, setShowFx] = useState(false);
  // Fuentes de audio separadas: solo las tiene el video de la partida.
  const sources = useMemo(() => (clip ? [] : (match.audio_tracks ?? []).filter((k) => k !== "mix")), [clip, match.audio_tracks]);
  useEffect(() => {
    api
      .settings()
      .then((s) => {
        if (s.mobile_layout) setLayout({ ...DEFAULT_MOBILE_LAYOUT, ...s.mobile_layout });
        setPresets(s.mobile_presets ?? []);
        // Lo que Ajustes deja fuera de los clips arranca apagado.
        setFx((f) => ({ ...f, muted: [...(s.clip_other_audio ? [] : ["other"]), ...(s.clip_mic_audio ? [] : ["mic"])] }));
      })
      .catch(() => {});
  }, []);
  const commitLayout = (l: MobileLayout) => {
    setLayout(l);
    api.setMobileLayout(l).catch((e) => toast.error(errText(e)));
  };
  const savePresets = (next: MobilePreset[]) => {
    setPresets(next);
    api.setMobilePresets(next).catch((e) => toast.error(errText(e)));
  };
  const presetName = presets.find((p) => JSON.stringify({ ...DEFAULT_MOBILE_LAYOUT, ...p.layout }) === JSON.stringify(layout))?.name ?? "";
  const drag = useRef<"a" | "b" | "seek" | null>(null);
  const evLane = useRef<HTMLDivElement>(null);
  const menu = useContextMenu();
  const run = useRun();
  const toast = useToast();
  /** Los eventos y las marcas de la partida solo aplican a su video completo. */
  const inMatch = !clip;

  const seek = useCallback((s: number) => {
    const v = video.current;
    if (!v) return;
    v.currentTime = Math.max(0, Math.min(s, v.duration || s));
    setT(v.currentTime);
  }, []);
  const toggle = useCallback(() => {
    const v = video.current;
    if (!v) return;
    if (v.paused) v.play().catch(() => {});
    else v.pause();
  }, []);
  const fullscreen = useCallback(() => {
    if (document.fullscreenElement) document.exitFullscreen();
    else wrap.current?.requestFullscreen();
  }, []);
  const markIn = useCallback(() => {
    const now = video.current?.currentTime ?? 0;
    setRange((r) => ({ a: now, b: r && r.b > now ? r.b : Math.min(now + 15, dur) }));
  }, [dur]);
  const markOut = useCallback(() => {
    const now = video.current?.currentTime ?? 0;
    setRange((r) => ({ a: r && r.a < now ? r.a : Math.max(0, now - 15), b: now }));
  }, []);

  useEffect(() => {
    const onSeek = (e: Event) => seek((e as CustomEvent<number>).detail);
    window.addEventListener("ggswp-seek", onSeek);
    return () => window.removeEventListener("ggswp-seek", onSeek);
  }, [seek]);

  /** Fija la selección de entrada/salida, ajustada a la duración del video, y salta a la entrada. */
  const selectRange = useCallback(
    (start: number, end: number) => {
      const max = video.current?.duration || dur;
      const a = Math.max(0, Math.min(start, max - 0.5));
      const b = Math.max(a + 0.5, Math.min(end, max));
      setRange({ a, b });
      seek(a);
    },
    [dur, seek],
  );

  useEffect(() => {
    if (!inMatch) return;
    const onSelect = (e: Event) => {
      const { start, end } = (e as CustomEvent<{ start: number; end: number }>).detail;
      selectRange(start, end);
    };
    window.addEventListener("ggswp-select", onSelect);
    return () => window.removeEventListener("ggswp-select", onSelect);
  }, [inMatch, selectRange]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      const v = video.current;
      if (!v) return;
      const k = e.key.toLowerCase();
      if (k === " " || k === "k") toggle();
      else if (k === "arrowleft" || k === "j") seek(v.currentTime - (e.shiftKey ? 1 : 5));
      else if (k === "arrowright" || k === "l") seek(v.currentTime + (e.shiftKey ? 1 : 5));
      else if (k === ",") seek(v.currentTime - 1 / 60);
      else if (k === ".") seek(v.currentTime + 1 / 60);
      else if (k === "i") markIn();
      else if (k === "o") markOut();
      else if (k === "escape") setRange(null);
      else if (k === "m") setMuted((x) => !x);
      else if (k === "f") fullscreen();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggle, seek, markIn, markOut, fullscreen]);

  useEffect(() => {
    if (video.current) video.current.muted = muted;
  }, [muted]);

  const posToTime = (clientX: number) => {
    const r = bar.current!.getBoundingClientRect();
    return Math.max(0, Math.min(1, (clientX - r.left) / r.width)) * dur;
  };
  const onPointerDown = (e: React.PointerEvent, what: "a" | "b" | "seek") => {
    // El clic derecho abre el menú de eventos; no mueve el cursor.
    if (e.button !== 0) return;
    e.stopPropagation();
    // Un clic sobre un evento va a ese momento y lo deja marcado para crear el clip.
    const ev = what === "seek" ? eventAt(e) : null;
    if (ev) {
      selectRange(ev.t - 10, ev.t + 4);
      return;
    }
    drag.current = what;
    bar.current?.setPointerCapture(e.pointerId);
    if (what === "seek") seek(posToTime(e.clientX));
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const s = posToTime(e.clientX);
    setHover(s);
    if (drag.current === "seek") seek(s);
    else if (drag.current === "a") setRange((r) => r && { ...r, a: Math.min(s, r.b - 0.5) });
    else if (drag.current === "b") setRange((r) => r && { ...r, b: Math.max(s, r.a + 0.5) });
  };
  const pct = (s: number) => `${(Math.max(0, Math.min(s, dur)) / Math.max(dur, 0.001)) * 100}%`;
  const events = useMemo(() => (clip ? [] : match.events), [clip, match.events]);

  /** Evento de la pista de eventos más cercano al puntero (a menos de 10 px). */
  const eventAt = (e: { clientX: number; clientY: number }) => {
    const lane = evLane.current?.getBoundingClientRect();
    if (!lane || !events.length || e.clientY < lane.top - 3 || e.clientY > lane.bottom + 3) return null;
    const x = (s: number) => lane.left + (Math.max(0, Math.min(s, dur)) / Math.max(dur, 0.001)) * lane.width;
    let near: MatchEvent | null = null;
    let best = 10;
    for (const ev of events) {
      const d = Math.abs(x(ev.t) - e.clientX);
      if (d <= best) [near, best] = [ev, d];
    }
    return near;
  };

  /** Clic derecho sobre la pista de eventos: menú del evento. */
  const onLanesContextMenu = (e: React.MouseEvent) => {
    const near = eventAt(e);
    if (!near) return;
    const t = near.t;
    menu.open(
      e,
      eventMenuItems(t, {
        seek: () => seek(Math.max(0, t - 3)),
        makeClip: () => selectRange(t - 10, t + 4),
        run,
      }),
    );
  };
  const lanesClips = useMemo(() => (clip ? [] : match.clips.filter((c) => onTimeline(c) && c.end <= match.duration_s + 1)), [clip, match]);

  const step = rulerStep(dur || 1);
  const half = step / 2;
  const ticks = useMemo(() => Array.from({ length: Math.floor((dur || 0) / half) + 1 }, (_, i) => i * half), [dur, half]);

  /** Tramo nuevo de 15 s alrededor del cursor, listo para ajustar con las asas. */
  const newClip = () => {
    const now = video.current?.currentTime ?? 0;
    selectRange(now - 5, now + 10);
  };

  // Sin tramo marcado, un clip se exporta entero (en otro formato o en vertical).
  const whole = !range && !!clip;
  // Tramo que se va a exportar y, dentro de él, dónde cae la cámara lenta.
  const span = range ?? (clip ? { a: 0, b: dur } : null);
  const moment = span && fx.moment ? { a: span.a + fx.moment.start, b: Math.min(span.a + fx.moment.end, span.b) } : null;
  const inMoment = !!moment && !!fx.moment && pos >= moment.a && pos < moment.b;
  const zoom = inMoment ? fx.moment!.zoom : 1;
  // La vista previa también va lenta dentro del efecto.
  useEffect(() => {
    if (video.current) video.current.playbackRate = rate * (inMoment ? fx.moment!.speed : 1);
  }, [inMoment, rate, fx.moment]);
  const doExport = async () => {
    const span = range ?? (clip ? { a: 0, b: video.current?.duration || dur } : null);
    if (!span) return;
    setExporting(true);
    try {
      await onExport({ start: span.a, end: span.b, title, quality, frame: canFrame ? frame : "horizontal", fx: toFx(fx, sources) });
      setRange(null);
      setTitle("");
      setFx((f) => ({ ...EMPTY_FX, muted: f.muted }));
    } catch (e) {
      toast.error(errText(e));
    } finally {
      setExporting(false);
    }
  };

  return (
    <div className="player">
      <div className={`video-wrap ${mobile ? "mobile" : ""}`} ref={wrap} onDoubleClick={fullscreen}>
        <video
          ref={video}
          src={src}
          preload="auto"
          onClick={toggle}
          onTimeUpdate={(e) => {
            const v = e.currentTarget;
            setT(v.currentTime);
            if (range && playing && v.currentTime >= range.b && v.currentTime < range.b + 0.3) v.pause();
          }}
          onLoadedMetadata={(e) => setDur(e.currentTarget.duration)}
          onPlay={() => setPlaying(true)}
          onPause={() => setPlaying(false)}
          style={!mobile && zoom > 1 ? { transform: `scale(${zoom})` } : undefined}
        />
        {mobile && <MobileOverlay video={video} layout={layout} onChange={setLayout} onCommit={commitLayout} />}
        {mobile && <MobilePreview video={video} layout={layout} texts={fx.texts} zoom={zoom} onChange={setLayout} onCommit={commitLayout} />}
        {!mobile && span && <TextPreview video={video} texts={fx.texts} frame={canFrame ? frame : "horizontal"} />}
      </div>

      <div className="controls">
        <button className="icon-btn" onClick={() => seek((video.current?.currentTime ?? 0) - 1 / 60)} aria-label={t("Cuadro anterior")} title={t("Cuadro anterior (,)")}>
          <Icon name="prev" size={16} />
        </button>
        <button className="icon-btn play" onClick={toggle} aria-label={playing ? t("Pausar") : t("Reproducir")}>
          <Icon name={playing ? "pause" : "play"} size={16} filled />
        </button>
        <button className="icon-btn" onClick={() => seek((video.current?.currentTime ?? 0) + 1 / 60)} aria-label={t("Cuadro siguiente")} title={t("Cuadro siguiente (.)")}>
          <Icon name="next" size={16} />
        </button>
        <span className="time">
          {fmtClock(pos)}.{String(Math.floor((pos % 1) * 100)).padStart(2, "0")} <span>/ {fmtClock(dur)}</span>
        </span>
        <span className="spacer" />
        <button className="icon-btn bare" onClick={() => setMuted((x) => !x)} aria-label={muted ? t("Activar sonido") : t("Silenciar")}>
          <Icon name={muted ? "mute" : "volume"} />
        </button>
        <label className="sort" style={{ marginLeft: 0 }}>
          {t("Velocidad")}
          <select value={rate} onChange={(e) => setRate(Number(e.target.value))}>
            {[0.25, 0.5, 1, 1.5, 2].map((r) => (
              <option key={r} value={r}>
                {dec(String(r))}×
              </option>
            ))}
          </select>
        </label>
        <button className="icon-btn" onClick={fullscreen} aria-label={t("Pantalla completa")}>
          <Icon name="fullscreen" size={16} />
        </button>
      </div>

      <section className="editor" aria-label={t("Editor de línea de tiempo")}>
        <div className="editor-top">
          <span className="label">{clip ? t("Recorte") : t("Selección")}</span>
          {range ? (
            <>
              <span className="editor-range">
                {fmtClock(range.a)}
                <i>→</i>
                {fmtClock(range.b)}
              </span>
              <span className="editor-len">{dec((range.b - range.a).toFixed(1))} s</span>
            </>
          ) : (
            <span className="muted small">{clip ? t("Clip completo. Marca un tramo para recortarlo.") : t("Marca el tramo que quieres convertir en clip.")}</span>
          )}
          <div className="editor-actions">
            {range ? (
              <>
                <button className="btn ghost sm" onClick={markIn}>
                  {t("Entrada")} <kbd>I</kbd>
                </button>
                <button className="btn ghost sm" onClick={markOut}>
                  {t("Salida")} <kbd>O</kbd>
                </button>
                <button className="btn ghost sm" onClick={() => (seek(range.a), video.current?.play())}>
                  {t("Previsualizar")}
                </button>
                <button className="btn ghost sm" onClick={() => setRange(null)}>
                  {t("Cancelar")}
                </button>
              </>
            ) : (
              <button className={`btn sm ${clip ? "ghost" : "primary"}`} onClick={newClip}>
                <Icon name="scissors" size={15} /> {clip ? t("Recortar") : t("Nuevo clip")}
              </button>
            )}
          </div>
        </div>

        {mobile && (
          <p className="muted small mf-help">
            {t("Sobre el video: arrastra")} <b>{t("Salud")}</b> {t("y")} <b>{t("Armas")}</b> {t("hasta cubrir esas partes de tu HUD (la esquina de cada zona cambia su tamaño) y mueve")} <b>{t("Encuadre")}</b>{" "}
            {t("para elegir qué parte del juego se ve. A la derecha, el resultado: arrástralo hacia arriba o abajo para mover la franja. Con")} <b>{t("+ Minimapa")}</b> {t("y")} <b>{t("+ Zona")}</b>{" "}
            {t("sumas piezas sueltas: se mueven arrastrándolas en el resultado y se agrandan desde su esquina.")}
          </p>
        )}
        {(range || clip) && (
          <div className="export-bar">
            <span className="label">{t("Exportar")}</span>
            {canFrame && (
              <div className="seg" role="group" aria-label={t("Encuadre")}>
                <button className={frame === "horizontal" ? "on" : ""} aria-pressed={frame === "horizontal"} onClick={() => setFrame("horizontal")}>
                  {t("Horizontal")}
                </button>
                <button className={frame === "vertical" ? "on" : ""} aria-pressed={frame === "vertical"} onClick={() => setFrame("vertical")} title={t("Recorte central 9:16 con el marcador de la partida")}>
                  {t("Vertical 9:16")}
                </button>
                <button
                  className={frame === "mobile" ? "on" : ""}
                  aria-pressed={frame === "mobile"}
                  onClick={() => setFrame("mobile")}
                  title={t("9:16 con la salud y las armas en una franja arriba, para TikTok, Reels y Shorts")}
                >
                  {t("Móvil")}
                </button>
              </div>
            )}
            {mobile && (
              <>
                <button
                  className="btn ghost sm"
                  aria-pressed={layout.two_rows}
                  onClick={() => commitLayout({ ...layout, two_rows: !layout.two_rows })}
                  title={t("La salud arriba y las armas abajo, más grandes")}
                >
                  {layout.two_rows ? t("HUD en una fila") : t("HUD en dos filas")}
                </button>
                <button className="btn ghost sm" disabled={(layout.extras ?? []).some((e) => e.label === MINIMAP_EXTRA.label)} onClick={() => commitLayout({ ...layout, extras: [...(layout.extras ?? []), MINIMAP_EXTRA] })} title={t("Agrega el minimapa como una pieza suelta que puedes mover")}>
                  {t("+ Minimapa")}
                </button>
                <button
                  className="btn ghost sm"
                  disabled={(layout.extras ?? []).length >= 4}
                  onClick={() => commitLayout({ ...layout, extras: [...(layout.extras ?? []), { ...FREE_EXTRA, label: `Zona ${(layout.extras ?? []).length + 1}` }] })}
                  title={t("Agrega otra parte de la pantalla (el contador de eliminaciones, la brújula…)")}
                >
                  {t("+ Zona")}
                </button>
                <label className="sort" style={{ marginLeft: 0 }}>
                  {t("Plantilla")}
                  <select
                    value={presetName}
                    onChange={(e) => {
                      const v = e.target.value;
                      if (v === "__default") commitLayout(DEFAULT_MOBILE_LAYOUT);
                      else {
                        const p = presets.find((p) => p.name === v);
                        if (p) commitLayout({ ...DEFAULT_MOBILE_LAYOUT, ...p.layout });
                      }
                    }}
                  >
                    {!presetName && <option value="">{t("Sin guardar")}</option>}
                    {presets.map((p) => (
                      <option key={p.name} value={p.name}>
                        {p.name}
                      </option>
                    ))}
                    <option value={"__default"}>{t("Fortnite 100 % (restablecer)")}</option>
                  </select>
                </label>
                {presetName ? (
                  <button className="btn ghost sm" onClick={() => savePresets(presets.filter((p) => p.name !== presetName))} title={t('Borra la plantilla "{name}"', { name: presetName })}>
                    {t("Borrar plantilla")}
                  </button>
                ) : (
                  naming === null ? (
                    <button className="btn ghost sm" title={t("Guarda estas zonas con un nombre para volver a usarlas")} onClick={() => setNaming(t("Plantilla {n}", { n: presets.length + 1 }))}>
                      {t("Guardar plantilla")}
                    </button>
                  ) : (
                    <form
                      className="preset-form"
                      onSubmit={(e) => {
                        e.preventDefault();
                        const name = naming.trim();
                        if (name) savePresets([...presets.filter((p) => p.name !== name), { name, layout }]);
                        setNaming(null);
                      }}
                    >
                      <input className="input" autoFocus aria-label={t("Nombre de la plantilla")} value={naming} maxLength={30} onChange={(e) => setNaming(e.target.value)} onKeyDown={(e) => e.key === "Escape" && setNaming(null)} />
                      <button className="btn primary sm" type="submit" disabled={!naming.trim()}>
                        {t("Guardar")}
                      </button>
                      <button className="btn ghost sm" type="button" onClick={() => setNaming(null)}>
                        {t("Cancelar")}
                      </button>
                    </form>
                  )
                )}
              </>
            )}
            <label className="sort" style={{ marginLeft: 0 }}>
              {t("Calidad")}
              <select value={quality} onChange={(e) => setQuality(e.target.value as Quality)}>
                {(["share", "small", "original"] as Quality[]).map((q) => (
                  <option key={q} value={q}>
                    {t(QUALITY_LABEL[q])}
                  </option>
                ))}
              </select>
            </label>
            <button className={`btn sm ${showFx || hasFx(fx) ? "light" : "ghost"}`} aria-expanded={showFx} onClick={() => setShowFx((v) => !v)} title={t("Texto, cámara lenta, zoom, música y qué audio lleva el clip")}>
              <Icon name="sliders" size={15} /> {t("Efectos")}{hasFx(fx) ? " ·" : ""}
            </button>
            <input className="input" aria-label={t("Nombre del clip")} placeholder={t("Nombre del clip")} value={title} onChange={(e) => setTitle(e.target.value)} maxLength={80} />
            <button
              className="btn primary sm"
              disabled={exporting || busy}
              title={busy && !exporting ? t("Espera a que termine el paso en curso") : undefined}
              onClick={doExport}
            >
              {exporting ? (
                <>
                  <span className="spinner" /> {t("Exportando…")}
                </>
              ) : (
                <>
                  <Icon name="film" size={15} /> {whole ? t("Exportar clip completo") : t("Exportar clip")}
                </>
              )}
            </button>
          </div>
        )}

        <div className="tl">
          <div className="tl-labels" aria-hidden>
            <span>{t("TIEMPO")}</span>
            <span>{t("EVENTOS")}</span>
            <span>{t("CLIPS")}</span>
            <span>{clip ? t("CLIP") : t("PARTIDA")}</span>
          </div>
          <div
            className="tl-lanes"
            ref={bar}
            role="slider"
            tabIndex={0}
            aria-label={t("Posición")}
            aria-valuemin={0}
            aria-valuemax={Math.round(dur)}
            aria-valuenow={Math.round(pos)}
            aria-valuetext={fmtClock(pos)}
            onPointerDown={(e) => onPointerDown(e, "seek")}
            onPointerMove={onPointerMove}
            onPointerUp={() => (drag.current = null)}
            onPointerLeave={() => setHover(null)}
            onContextMenu={onLanesContextMenu}
          >
            <div className="lane lane-ruler">
              {ticks.map((s) => (
                <span key={s} className={`rt ${s % step === 0 ? "major" : ""}`} style={{ left: pct(s) }} />
              ))}
              {ticks
                .filter((s) => s % step === 0 && s <= dur - step * 0.4)
                .map((s) => (
                  <span key={`l${s}`} className="rl" style={{ left: pct(s) }}>
                    {fmtClock(s)}
                  </span>
                ))}
            </div>
            <div className="lane lane-events" ref={evLane}>
              {events.map((e, i) => (
                <span key={i} className={`ev ${e.kind}`} style={{ left: pct(e.t), background: EVENT_COLORS[e.kind] }} title={`${tr(e.label)} · ${fmtClock(e.t)}\n${t("Clic: ir a este momento y marcarlo para crear un clip")}`} />
              ))}
            </div>
            <div className="lane lane-clips">
              {lanesClips.map((c) => (
                <button
                  key={c.id}
                  type="button"
                  className="cl"
                  style={{ left: pct(c.start), width: pct(c.end - c.start) }}
                  title={`${tr(c.title)} · ${fmtClock(c.start)} – ${fmtClock(c.end)}\n${t("Clic: ir a este momento y marcarlo para crear un clip")}`}
                  // Sin esto, la pista toma el clic como un salto y empieza a arrastrar el cursor.
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={() => selectRange(c.start, c.end)}
                >
                  {tr(c.title)}
                </button>
              ))}
            </div>
            <div className="lane lane-track">
              <div className="played" style={{ width: pct(pos) }} />
            </div>

            {range && (
              <>
                <span className="tl-dim" style={{ left: 0, width: pct(range.a) }} />
                <span className="tl-dim" style={{ left: pct(range.b), right: 0 }} />
                <span className="tl-sel" style={{ left: pct(range.a), width: pct(range.b - range.a) }} />
                <span className="tl-handle a" style={{ left: pct(range.a) }} onPointerDown={(e) => onPointerDown(e, "a")} />
                <span className="tl-handle b" style={{ left: pct(range.b) }} onPointerDown={(e) => onPointerDown(e, "b")} />
              </>
            )}
            {moment && <span className="tl-fx" style={{ left: pct(moment.a), width: pct(moment.b - moment.a) }} title={t("Cámara lenta / zoom")} />}
            <span className="tl-head" style={{ left: pct(pos) }} />
            {hover !== null && !drag.current && (
              <span className="tl-hover" style={{ left: edgeSafe(pct(hover)) }}>
                {fmtClock(hover)}
              </span>
            )}
            <span className="tl-head-label" style={{ left: edgeSafe(pct(pos)) }}>
              {fmtClock(pos)}
            </span>
          </div>
        </div>

        {showFx && span && <FxPanel fx={fx} onChange={setFx} sources={sources} length={span.b - span.a} now={pos - span.a} />}

        <div className="legend">
          <span>
            <kbd>I</kbd> / <kbd>O</kbd> {t("entrada y salida")}
          </span>
          <span>
            <kbd>← →</kbd> 5 s
          </span>
          <span>
            <kbd>⇧ ← →</kbd> 1 s
          </span>
          <span>
            <kbd>, .</kbd> {t("cuadro a cuadro")}
          </span>
          <span>
            <kbd>{t("Espacio")}</kbd> {t("pausa")}
          </span>
          <span>
            <kbd>F</kbd> {t("pantalla completa")}
          </span>
        </div>
      </section>
    </div>
  );
}
