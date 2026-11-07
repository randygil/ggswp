import { useMemo, useState } from "react";
import {
  api,
  clipTag,
  daysLeft,
  deathAt,
  errText,
  expiryText,
  fileUrl,
  fmtClock,
  fmtDate,
  fmtPlayed,
  fmtSize,
  fmtTime,
  longestShot,
  onTimeline,
  pad2,
  totalSize,
  type Clip,
  type Match,
  type Status,
  type Step,
  type Totals,
} from "../api";
import { Icon } from "../components/Icon";
import { useClipMenu, useMatchMenu } from "../components/menus";
import { ShareDialog } from "../components/ShareDialog";
import { Signature } from "../components/Signature";
import { StepProgress } from "../components/StepProgress";
import { useToast } from "../components/Toast";
import { fmtNum, hasPlacement } from "../matchStats";

type Filter = "all" | "wins" | "kept";
type Sort = "recent" | "kills" | "long";

const upper = (s: string) => s.toLocaleUpperCase("es");

export function Gallery(props: {
  matches: Match[];
  status: Status | null;
  onOpen: (id: string) => void;
  onOpenClip: (id: string, clipId: string) => void;
  onAllClips: () => void;
  retentionDays?: number;
  totals: Totals | null;
}) {
  const { matches, status, onOpen, onOpenClip, onAllClips, retentionDays, totals } = props;
  const toast = useToast();
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<Sort>("recent");
  const [query, setQuery] = useState("");
  const [share, setShare] = useState<Match | null>(null);

  const hero = useMemo(() => matches.find((m) => m.status === "ready" && m.video), [matches]);
  const list = useMemo(() => {
    const q = query.trim().toLocaleLowerCase("es");
    const out = matches.filter((m) => {
      if (filter === "wins" && !m.victory) return false;
      if (filter === "kept" && !m.keep) return false;
      if (!q) return true;
      return [m.title, m.mode ?? "", fmtDate(m.created_at), ...m.clips.map((c) => c.title)].some((s) => s.toLocaleLowerCase("es").includes(q));
    });
    if (sort === "kills") out.sort((a, b) => b.kills - a.kills);
    if (sort === "long") out.sort((a, b) => b.duration_s - a.duration_s);
    return out;
  }, [matches, filter, sort, query]);
  const recentClips = useMemo(
    () =>
      matches
        .flatMap((m) => m.clips.map((c) => ({ m, c })))
        .sort((a, b) => new Date(b.c.created_at).getTime() - new Date(a.c.created_at).getTime())
        .slice(0, 4),
    [matches],
  );
  const clipCount = matches.reduce((a, m) => a + m.clips.length, 0);
  const rec = status?.recording;

  const chips: [Filter, string, number][] = [
    ["all", "Todas", matches.length],
    ["wins", "Victorias", matches.filter((m) => m.victory).length],
    ["kept", "Conservadas", matches.filter((m) => m.keep).length],
  ];

  return (
    <div className="page">
      <header className="topbar">
        <div className="crumbs">
          Biblioteca<i>/</i>
          {matches.length} {matches.length === 1 ? "partida" : "partidas"}
          {matches.length > 0 && (
            <>
              <i>/</i>
              {fmtSize(totalSize(matches))}
            </>
          )}
        </div>
        <div className="topbar-actions">
          <label className="search">
            <Icon name="search" size={16} />
            <span className="sr">Buscar partidas</span>
            <input type="search" placeholder="Buscar por modo, fecha o clip" value={query} onChange={(e) => setQuery(e.target.value)} />
          </label>
          <button
            className={`btn ${rec ? "danger" : "primary"}`}
            onClick={() => (rec ? api.stopRecording() : api.startRecording()).catch((e) => toast.error(errText(e)))}
            title={rec ? "Detener la grabación" : "Grabar ahora sin esperar a una partida"}
          >
            <span className="rec-dot" />
            {rec ? "Detener grabación" : "Grabar ahora"}
          </button>
        </div>
      </header>

      {totals && totals.matches > 0 && <TotalsStrip t={totals} />}

      {hero ? (
        <Hero m={hero} onOpen={() => onOpen(hero.id)} onShare={() => setShare(hero)} />
      ) : (
        <div className="empty">
          <Icon name="film" size={36} />
          <h2>{matches.length ? "Tu próxima partida va aquí" : "Todavía no hay partidas"}</h2>
          <p>
            {status?.fortnite_running
              ? "Fortnite está abierto. La grabación arranca sola cuando empiece la partida."
              : "Abre Fortnite y entra a una partida: ggswp la graba automáticamente."}
          </p>
        </div>
      )}

      {matches.length > 0 && (
        <section className="section" aria-labelledby="todas">
          <div className="section-head">
            <h2 id="todas" className="section-title">
              Partidas
            </h2>
            <div className="chips" role="group" aria-label="Filtrar partidas">
              {chips.map(([k, label, n]) => (
                <button key={k} className={`chip ${filter === k ? "on" : ""}`} aria-pressed={filter === k} onClick={() => setFilter(k)}>
                  {label} <span>{pad2(n)}</span>
                </button>
              ))}
            </div>
            <label className="sort">
              Ordenar
              <select value={sort} onChange={(e) => setSort(e.target.value as Sort)}>
                <option value="recent">Más recientes</option>
                <option value="kills">Más eliminaciones</option>
                <option value="long">Más largas</option>
              </select>
            </label>
          </div>
          {list.length === 0 ? (
            <div className="empty">
              <h2>Nada por aquí</h2>
              <p>Ninguna partida coincide con este filtro o búsqueda.</p>
            </div>
          ) : (
            <div className="grid">
              {list.map((m) => (
                <MatchCard key={m.id} m={m} step={status?.steps?.[m.id]} retentionDays={retentionDays} onOpen={() => onOpen(m.id)} />
              ))}
            </div>
          )}
        </section>
      )}

      {recentClips.length > 0 && (
        <section className="section" aria-labelledby="clips-recientes">
          <div className="section-head">
            <h2 id="clips-recientes" className="section-title">
              Clips recientes
            </h2>
            <button className="link" onClick={onAllClips}>
              Ver los {clipCount}
            </button>
          </div>
          <div className="grid">
            {recentClips.map(({ m, c }) => (
              <ClipCard key={`${m.id}/${c.id}`} m={m} c={c} onOpen={() => onOpenClip(m.id, c.id)} />
            ))}
          </div>
        </section>
      )}

      {share && <ShareDialog match={share} onClose={() => setShare(null)} />}
    </div>
  );
}

function Hero({ m, onOpen, onShare }: { m: Match; onOpen: () => void; onShare: () => void }) {
  const shot = longestShot(m);
  const word = m.victory ? "Victoria" : m.died ? "Eliminado" : "Partida";
  const sub = m.victory ? "Royale" : m.mode ?? m.title;
  const autoClips = m.clips.filter((c) => c.kind !== "hotkey" && c.kind !== "manual").length;
  const menu = useMatchMenu(m, onOpen);
  return (
    <section className="hero" aria-labelledby="ultima" onContextMenu={menu.onContextMenu}>
      {menu.dialogs}
      <div className="hero-eyebrow">
        <span id="ultima">
          Última partida — {upper(fmtDate(m.created_at))}
          {m.mode && ` — ${upper(m.mode)}`}
        </span>
        <span>
          {m.width > 0 && `${m.width}×${m.height} · `}
          {fmtSize(m.size_bytes)}
        </span>
      </div>

      <div className="hero-row">
        <div className="hero-info">
          <div>
            <div className={`hero-word ${m.victory ? "win" : ""}`}>{word}</div>
            <div className="hero-sub">{sub}</div>
          </div>
          <dl className="stats">
            <div className="stat">
              <dt>Eliminaciones</dt>
              <dd>{pad2(m.kills)}</dd>
            </div>
            {hasPlacement(m) ? (
              <div className="stat">
                <dt>Puesto</dt>
                <dd className={m.placement === 1 ? "accent" : ""}>
                  #{m.placement}
                  {m.total_players != null && <small> / {m.total_players}</small>}
                </dd>
              </div>
            ) : (
              <div className="stat">
                <dt>Duración</dt>
                <dd>{fmtClock(m.duration_s)}</dd>
              </div>
            )}
            <div className="stat">
              <dt>Disparo más largo</dt>
              <dd>
                {shot ? (
                  <>
                    {Math.round(shot)}
                    <small> m</small>
                  </>
                ) : (
                  "—"
                )}
              </dd>
            </div>
            <div className="stat">
              <dt>{m.chests > 0 ? "Cofres abiertos" : m.knocks > 0 ? "Derribos" : "Clips automáticos"}</dt>
              <dd>{pad2(m.chests > 0 ? m.chests : m.knocks > 0 ? m.knocks : autoClips)}</dd>
            </div>
          </dl>
          <div className="hero-actions">
            <button className="btn light" onClick={onOpen}>
              <Icon name="play" size={16} filled /> Ver partida
            </button>
            <button className="btn ghost" onClick={onShare}>
              <Icon name="upload" size={16} /> Al teléfono
            </button>
          </div>
        </div>

        <button className="hero-frame" onClick={onOpen} aria-label={`Reproducir ${m.title}`}>
          <div className={`vf big ${m.thumb ? "" : "dots"}`}>
            {m.thumb && <img src={fileUrl(m, m.thumb, m.size_bytes)} alt="" />}
            <div className="vf-shade" />
            {m.width > 0 && <span className="hero-res">{m.width}×{m.height}</span>}
            <span className="play-dot">
              <Icon name="play" size={30} filled />
            </span>
            <span className="hero-time">
              00:00 <span className="muted">/ {fmtClock(m.duration_s)}</span>
            </span>
          </div>
        </button>
      </div>

      {m.events.length > 0 && (
        <figure className="sig-panel" aria-label={`Línea de tiempo: ${m.kills} eliminaciones`} style={{ margin: 0 }}>
          <div className="sig-legend label">
            <span>
              <i style={{ width: 3, height: 12, background: "var(--accent)" }} />
              Eliminación
            </span>
            <span>
              <i style={{ width: 12, height: 8, background: "var(--accent-soft)", border: "1px solid var(--accent-edge)" }} />
              Clip
            </span>
            <span>
              <i style={{ width: 2, height: 12, background: "var(--text)" }} />
              Marca / autobús
            </span>
            {m.chests > 0 && (
              <span>
                <i style={{ width: 2, height: 8, background: "var(--text-2)" }} />
                Cofre
              </span>
            )}
            {m.died && (
              <span>
                <i style={{ width: 3, height: 12, background: "var(--flare)" }} />
                Caída
              </span>
            )}
            <span style={{ marginLeft: "auto" }}>{fmtClock(m.duration_s)}</span>
          </div>
          <Signature variant="full" events={m.events} clips={m.clips.filter((c) => onTimeline(c) && c.end <= m.duration_s + 1)} duration={m.duration_s} />
        </figure>
      )}
    </section>
  );
}

/** Totales de todas las partidas: siguen sumando aunque las partidas venzan. */
function TotalsStrip({ t }: { t: Totals }) {
  // Con puesto, el número de partidas pasa a la etiqueta para no pasar de seis datos.
  const placed = t.avg_placement != null && !!t.placed_matches;
  const items: [string, string, boolean, string?][] = [
    [String(t.chests), t.chests === 1 ? "cofre" : "cofres", true],
    [String(t.kills), t.kills === 1 ? "eliminación" : "eliminaciones", false],
    [String(t.victories), t.victories === 1 ? "victoria" : "victorias", false],
  ];
  if (placed) items.push([`#${Math.round(t.avg_placement!)}`, "puesto prom.", false, `Mejor puesto: #${t.best_placement} · promedio de ${t.placed_matches} partidas con puesto`]);
  else items.push([String(t.matches), t.matches === 1 ? "partida" : "partidas", false]);
  if (t.damage_dealt) items.push([fmtNum(t.damage_dealt), "de daño", false]);
  items.push([fmtPlayed(t.seconds), "jugado", false]);
  return (
    <section className="totals" aria-label="Totales de todas tus partidas">
      <span className="label" title="Siguen sumando aunque las partidas venzan o se borren">
        Histórico
        {placed && ` · ${t.matches} ${t.matches === 1 ? "partida" : "partidas"}`}
        {t.since && ` · desde ${new Date(t.since).toLocaleDateString("es", { day: "numeric", month: "short" })}`}
      </span>
      <dl>
        {items.map(([v, label, accent, title]) => (
          <div key={label} title={title}>
            <dd className={accent ? "accent" : ""}>{v}</dd>
            <dt>{label}</dt>
          </div>
        ))}
      </dl>
    </section>
  );
}

function MatchCard({ m, step, retentionDays, onOpen }: { m: Match; step?: Step; retentionDays?: number; onOpen: () => void }) {
  const busy = m.status === "recording" || m.status === "processing" || !!step;
  const died = deathAt(m);
  const left = daysLeft(m, retentionDays);
  const menu = useMatchMenu(m, onOpen);
  return (
    <button className="card" onClick={onOpen} onContextMenu={menu.onContextMenu} disabled={m.status === "recording"}>
      {menu.dialogs}
      <div className={`vf ${m.thumb ? "" : "dots"}`}>
        {m.thumb && <img src={fileUrl(m, m.thumb, m.size_bytes)} alt="" loading="lazy" />}
        <div className="vf-shade" />
        {m.keep ? (
          <span className="tag light pos-tl" title="No vence">
            <Icon name="keep" size={12} filled /> Conservada
          </span>
        ) : left !== null && left <= 2 ? (
          <span className="tag dead pos-tl">{expiryText(left)}</span>
        ) : null}
        {m.offloaded && (
          <span className="tag dark pos-br" title="El video está solo en Google Drive">
            <Icon name="cloud" size={12} /> Solo en Drive
          </span>
        )}
        {m.duration_s > 0 && <span className="tag dark pos-tr mono">{fmtClock(m.duration_s)}</span>}
        {m.victory ? (
          <span className="tag win pos-bl">Victoria</span>
        ) : died !== null ? (
          <span className="tag dead pos-bl">Eliminado {fmtClock(died)}</span>
        ) : null}
        <span className="play-dot sm">
          <Icon name="play" size={16} filled />
        </span>
        {busy && (
          <div className="overlay-msg">
            {m.status === "recording" ? (
              <>
                <span className="dot rec" /> Grabando
              </>
            ) : (
              <StepProgress step={step} fallback="Procesando" />
            )}
          </div>
        )}
        {m.status === "error" && <div className="overlay-msg error">Error al procesar</div>}
      </div>
      <div className="card-row">
        <div className="card-text">
          <span className="card-title">{m.title}</span>
          <span className="card-meta">
            {fmtDate(m.created_at)}
            {m.mode && ` · ${m.mode}`}
            {left !== null && left > 2 && ` · ${expiryText(left).toLocaleLowerCase("es")}`}
          </span>
        </div>
        {hasPlacement(m) && (
          <div className="card-kills card-place" title={m.total_players ? `Puesto ${m.placement} de ${m.total_players}` : `Puesto ${m.placement}`}>
            <b className={m.placement === 1 ? "accent" : ""}>#{m.placement}</b>
            <span>{m.total_players ? `DE ${m.total_players}` : "PUESTO"}</span>
          </div>
        )}
        <div className="card-kills">
          <b>{pad2(m.kills)}</b>
          <span>ELIM</span>
        </div>
      </div>
      {m.duration_s > 0 && <Signature events={m.events} duration={m.duration_s} />}
    </button>
  );
}

export function ClipCard({ m, c, onOpen }: { m: Match; c: Clip; onOpen: () => void }) {
  const tag = clipTag(c.kind);
  const menu = useClipMenu(m, c, onOpen);
  return (
    <button className="card" onClick={onOpen} onContextMenu={menu.onContextMenu}>
      {menu.dialogs}
      <div className={`vf ${c.thumb ? "" : "dots"}`}>
        {c.thumb && <img src={fileUrl(m, c.thumb)} alt="" loading="lazy" />}
        <div className="vf-shade" />
        <span className={`tag pos-tl ${tag === "Auto" ? "win" : "light"}`}>{tag}</span>
        <span className="tag dark pos-br mono">{fmtTime(c.end - c.start)}</span>
        {c.drive && (
          <span className="tag dark pos-tr" title={c.offloaded ? "Solo está en Google Drive" : "También está en Google Drive"}>
            {c.offloaded ? "Solo en Drive" : "Drive"}
          </span>
        )}
        <span className="play-dot sm">
          <Icon name="play" size={16} filled />
        </span>
      </div>
      <span className="card-big">{c.title}</span>
      <span className="card-meta">
        {m.title}
        {c.kind !== "reel" && ` · ${fmtClock(c.start)}`}
      </span>
    </button>
  );
}
