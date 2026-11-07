import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { fmtDate, fmtPlayed, type Match, type Totals } from "../api";
import { Icon } from "../components/Icon";
import { fmtDistance, fmtNum, fmtPct, hasPlacement } from "../matchStats";

/** Solo las partidas reales de Fortnite (no las grabaciones manuales), de la más vieja a la más nueva. */
const isGame = (m: Match) => m.status === "ready" && (!!m.replay || !!m.account_id);

const shortDay = (iso: string) => new Date(iso).toLocaleDateString("es", { day: "numeric", month: "short" });
const avg = (xs: number[]) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);
const dec = (n: number, d = 1) => n.toLocaleString("es", { maximumFractionDigits: d });

type Metric = {
  key: string;
  title: string;
  value: (m: Match) => number | null;
  fmt: (n: number) => string;
  /** Menos es mejor (puesto): el eje va al revés, con el #1 arriba. */
  lowerIsBetter?: boolean;
  form: "bars" | "line";
};

const METRICS: Metric[] = [
  { key: "kills", title: "Eliminaciones", value: (m) => m.kills, fmt: fmtNum, form: "bars" },
  { key: "chests", title: "Cofres", value: (m) => m.chests, fmt: fmtNum, form: "bars" },
  { key: "placement", title: "Puesto", value: (m) => (hasPlacement(m) ? m.placement : null), fmt: (n) => `#${Math.round(n)}`, lowerIsBetter: true, form: "line" },
  { key: "damage", title: "Daño hecho", value: (m) => m.damage_dealt ?? null, fmt: fmtNum, form: "bars" },
];

export function StatsView({ matches, totals, onOpen }: { matches: Match[]; totals: Totals | null; onOpen: (id: string) => void }) {
  const games = useMemo(() => matches.filter(isGame).sort((a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime()), [matches]);
  const [table, setTable] = useState(false);

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1>Estadísticas</h1>
          <p>Tu histórico completo y cómo te fue partida a partida, con las partidas que siguen en la biblioteca.</p>
        </div>
        {games.length > 0 && (
          <button className="btn ghost" onClick={() => setTable((t) => !t)} aria-pressed={table}>
            {table ? "Ver gráficos" : "Ver tabla"}
          </button>
        )}
      </header>

      {totals && totals.matches > 0 && <Lifetime t={totals} />}

      {games.length === 0 ? (
        <div className="empty">
          <Icon name="chart" size={36} />
          <h2>Sin partidas para graficar</h2>
          <p>Cuando termines una partida, aquí verás tus eliminaciones, cofres, puesto y daño en el tiempo.</p>
        </div>
      ) : table ? (
        <MatchTable games={games} onOpen={onOpen} />
      ) : (
        <>
          <section className="section" aria-labelledby="sv-por-partida">
            <div className="section-head">
              <h2 id="sv-por-partida" className="section-title">
                Por partida
              </h2>
              <span className="label">
                {games.length} {games.length === 1 ? "partida" : "partidas"} · de {shortDay(games[0].created_at)} a {shortDay(games[games.length - 1].created_at)}
              </span>
            </div>
            <div className="sv-grid">
              {METRICS.map((s) => (
                <Chart key={s.key} games={games} s={s} onOpen={onOpen} />
              ))}
            </div>
          </section>
          {games.length >= 12 && <Compare games={games} />}
        </>
      )}
    </div>
  );
}

function Lifetime({ t }: { t: Totals }) {
  const tiles: { label: string; value: string; sub?: string; accent?: boolean }[] = [
    { label: "Partidas", value: fmtNum(t.matches), sub: fmtPlayed(t.seconds) + " jugado" },
    { label: "Victorias", value: fmtNum(t.victories), sub: t.matches ? `${dec((t.victories / t.matches) * 100)} % de las partidas` : undefined, accent: t.victories > 0 },
    { label: "Eliminaciones", value: fmtNum(t.kills), sub: t.matches ? `${dec(t.kills / t.matches)} por partida` : undefined },
    { label: "Derribos", value: fmtNum(t.knocks) },
    { label: "Cofres", value: fmtNum(t.chests), sub: t.matches ? `${dec(t.chests / t.matches)} por partida` : undefined },
  ];
  if (t.best_placement != null) tiles.push({ label: "Mejor puesto", value: `#${t.best_placement}`, accent: t.best_placement === 1 });
  if (t.avg_placement != null) tiles.push({ label: "Puesto promedio", value: `#${dec(t.avg_placement)}`, sub: `en ${t.placed_matches ?? 0} partidas con puesto` });
  if (t.damage_dealt) tiles.push({ label: "Daño hecho", value: fmtNum(t.damage_dealt) });
  if (t.distance_m) tiles.push({ label: "Distancia", value: fmtDistance(t.distance_m) });
  return (
    <section className="section" aria-labelledby="sv-historico">
      <div className="section-head">
        <h2 id="sv-historico" className="section-title">
          Histórico
        </h2>
        <span className="label" title="Siguen sumando aunque las partidas venzan o se borren">
          {t.since ? `Desde ${new Date(t.since).toLocaleDateString("es", { day: "numeric", month: "long", year: "numeric" })}` : "Todas tus partidas"}
        </span>
      </div>
      <dl className="sv-tiles">
        {tiles.map((x) => (
          <div key={x.label}>
            <dt>{x.label}</dt>
            <dd className={x.accent ? "accent" : ""}>{x.value}</dd>
            {x.sub && <span className="sv-sub">{x.sub}</span>}
          </div>
        ))}
      </dl>
    </section>
  );
}

/** 1, 2, 5 × 10^k: el paso "redondo" más cercano para unas 3–4 marcas. */
function niceStep(span: number, ticks = 3) {
  const raw = Math.max(span / ticks, 1);
  const p = 10 ** Math.floor(Math.log10(raw));
  const f = raw / p;
  return (f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10) * p;
}

const tickText = (n: number) => (n >= 10_000 ? `${dec(n / 1000)}K` : fmtNum(n));

function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [w, setW] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setW(e.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, w] as const;
}

const H = 200;
const PAD = { l: 40, r: 12, t: 18, b: 26 };

function Chart({ games, s, onOpen }: { games: Match[]; s: Metric; onOpen: (id: string) => void }) {
  const id = useId();
  const [ref, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const vals = games.map(s.value);
  const present = vals.filter((v): v is number => v != null);
  const mean = avg(present);

  const head = (
    <div className="sv-chart-head">
      <h3 id={`${id}-t`}>{s.title}</h3>
      <span className="label">
        {mean == null ? "sin datos" : `prom. ${s.lowerIsBetter ? `#${dec(mean)}` : dec(mean)}`}
        {s.lowerIsBetter && " · más arriba es mejor"}
      </span>
    </div>
  );

  if (present.length === 0) {
    return (
      <figure className="sv-chart">
        {head}
        <p className="sv-nodata">Aún no hay datos: se leen del replay de Fortnite al terminar cada partida.</p>
      </figure>
    );
  }

  const n = games.length;
  const iw = Math.max(0, width - PAD.l - PAD.r);
  const ih = H - PAD.t - PAD.b;
  const band = n ? iw / n : 0;
  const cx = (i: number) => PAD.l + band * (i + 0.5);
  const base = PAD.t + ih;

  // Escala: barras desde 0; el puesto va de #1 (arriba) al total de jugadores (abajo).
  let ticks: number[];
  let y: (v: number) => number;
  if (s.lowerIsBetter) {
    const worst = Math.max(...present, ...games.map((g) => g.total_players ?? 0), 2);
    const step = niceStep(worst - 1);
    const top = Math.ceil(worst / step) * step;
    ticks = [1];
    for (let v = step; v <= top; v += step) if (v > 1) ticks.push(v);
    y = (v) => PAD.t + ((v - 1) / (top - 1)) * ih;
  } else {
    const max = Math.max(...present, 1);
    const step = niceStep(max);
    const top = Math.ceil(max / step) * step;
    ticks = [];
    for (let v = 0; v <= top; v += step) ticks.push(v);
    y = (v) => base - (v / top) * ih;
  }

  const bw = band >= 4 ? Math.min(24, band - 2) : Math.max(band, 1);
  const extreme = s.lowerIsBetter ? Math.min(...present) : Math.max(...present);
  const extremeAt = vals.lastIndexOf(extreme);

  const bar = (i: number, v: number) => {
    const x0 = cx(i) - bw / 2;
    const top = y(v);
    const h = base - top;
    if (h <= 0) return "";
    const r = Math.min(4, bw / 2, h);
    return `M${x0} ${base}V${top + r}Q${x0} ${top} ${x0 + r} ${top}H${x0 + bw - r}Q${x0 + bw} ${top} ${x0 + bw} ${top + r}V${base}Z`;
  };

  let line = "";
  let pen = false;
  vals.forEach((v, i) => {
    if (v == null) {
      pen = false;
      return;
    }
    line += `${pen ? "L" : "M"}${cx(i)} ${y(v)}`;
    pen = true;
  });

  const onKey = (e: KeyboardEvent) => {
    const cur = hover ?? n - 1;
    if (e.key === "ArrowLeft") setHover(Math.max(0, cur - 1));
    else if (e.key === "ArrowRight") setHover(Math.min(n - 1, cur + 1));
    else if (e.key === "Home") setHover(0);
    else if (e.key === "End") setHover(n - 1);
    else if (e.key === "Enter" && hover != null) onOpen(games[hover].id);
    else return;
    e.preventDefault();
  };

  const hv = hover != null ? vals[hover] : null;
  const desc =
    `${present.length} partidas con dato. Promedio ${mean != null ? s.fmt(mean) : "—"}, ` +
    `${s.lowerIsBetter ? "mejor" : "máximo"} ${s.fmt(extreme)}, última ${vals[n - 1] != null ? s.fmt(vals[n - 1]!) : "sin dato"}. ` +
    "Usa las flechas para recorrer las partidas y Enter para abrir una.";

  return (
    <figure className="sv-chart">
      {head}
      <div className="sv-plot" ref={ref}>
        {width > 0 && (
          <svg
            width={width}
            height={H}
            role="img"
            aria-labelledby={`${id}-t ${id}-d`}
            tabIndex={0}
            onKeyDown={onKey}
            onFocus={() => setHover((h) => h ?? n - 1)}
            onBlur={() => setHover(null)}
            onPointerLeave={() => setHover(null)}
          >
            <desc id={`${id}-d`}>{desc}</desc>
            {ticks.map((t) => (
              <g key={t}>
                <line x1={PAD.l} x2={width - PAD.r} y1={y(t)} y2={y(t)} className="sv-grid-line" />
                <text x={PAD.l - 8} y={y(t)} className="sv-tick" textAnchor="end" dominantBaseline="middle">
                  {s.lowerIsBetter ? `#${t}` : tickText(t)}
                </text>
              </g>
            ))}
            {!s.lowerIsBetter && <line x1={PAD.l} x2={width - PAD.r} y1={base} y2={base} className="sv-axis" />}
            {mean != null && (
              <g className="sv-mean">
                <line x1={PAD.l} x2={width - PAD.r} y1={y(mean)} y2={y(mean)} />
              </g>
            )}

            {hover != null && <line x1={cx(hover)} x2={cx(hover)} y1={PAD.t} y2={base} className="sv-cross" />}

            {s.form === "bars" &&
              vals.map((v, i) => (v != null && v > 0 ? <path key={i} d={bar(i, v)} className={`sv-bar ${hover === i ? "on" : ""}`} /> : null))}

            {s.form === "line" && (
              <>
                <path d={line} className="sv-line" />
                {vals.map((v, i) =>
                  v != null && (band >= 7 || hover === i || i === extremeAt) ? (
                    <circle key={i} cx={cx(i)} cy={y(v)} r={hover === i ? 5 : 4} className={`sv-dot ${hover === i ? "on" : ""}`} />
                  ) : null,
                )}
              </>
            )}

            {hover == null && (
              <text
                x={Math.min(Math.max(cx(extremeAt), PAD.l + 12), width - PAD.r - 12)}
                y={y(extreme) - (s.form === "line" ? 10 : 6)}
                className="sv-val"
                textAnchor="middle"
              >
                {s.fmt(extreme)}
              </text>
            )}

            <text x={PAD.l} y={H - 6} className="sv-tick">
              {shortDay(games[0].created_at)}
            </text>
            {n > 1 && (
              <text x={width - PAD.r} y={H - 6} className="sv-tick" textAnchor="end">
                {shortDay(games[n - 1].created_at)}
              </text>
            )}

            {games.map((g, i) => (
              <rect
                key={g.id}
                x={PAD.l + band * i}
                y={PAD.t}
                width={band}
                height={ih}
                className="sv-hit"
                onPointerEnter={() => setHover(i)}
                onClick={() => onOpen(g.id)}
              />
            ))}
          </svg>
        )}
        {hover != null && (
          <div className="sv-tip" style={{ left: Math.min(Math.max(cx(hover), 80), width - 80), top: hv != null ? y(hv) : base }} aria-live="polite">
            <b>{hv != null ? s.fmt(hv) : "Sin dato"}</b>
            {s.lowerIsBetter && hv != null && games[hover].total_players ? <span> de {games[hover].total_players}</span> : null}
            <span className="sv-tip-meta">
              {fmtDate(games[hover].created_at)}
              {games[hover].mode && ` · ${games[hover].mode}`}
            </span>
          </div>
        )}
      </div>
    </figure>
  );
}

/** Últimas 10 partidas contra todas las anteriores (promedios por partida). */
function Compare({ games }: { games: Match[] }) {
  const recent = games.slice(-10);
  const before = games.slice(0, -10);
  // `eps`: diferencia por debajo de la cual se considera "igual".
  const rows: { label: string; v: (m: Match) => number | null; fmt: (n: number) => string; eps: number; lower?: boolean }[] = [
    { label: "Eliminaciones", v: (m) => m.kills, fmt: (n) => dec(n), eps: 0.05 },
    { label: "Cofres", v: (m) => m.chests, fmt: (n) => dec(n), eps: 0.05 },
    { label: "Puesto", v: (m) => (hasPlacement(m) ? m.placement : null), fmt: (n) => `#${dec(n)}`, eps: 0.5, lower: true },
    { label: "Daño hecho", v: (m) => m.damage_dealt ?? null, fmt: (n) => fmtNum(n), eps: 1 },
    { label: "Precisión", v: (m) => m.accuracy ?? null, fmt: fmtPct, eps: 0.005 },
    { label: "Victorias", v: (m) => (m.victory ? 1 : 0), fmt: fmtPct, eps: 0.005 },
  ];
  const mean = (list: Match[], f: (m: Match) => number | null) => avg(list.map(f).filter((x): x is number => x != null));
  return (
    <section className="section" aria-labelledby="sv-comparar">
      <div className="section-head">
        <h2 id="sv-comparar" className="section-title">
          Últimas 10 vs anteriores
        </h2>
        <span className="label">Promedio por partida · {before.length} anteriores</span>
      </div>
      <table className="sv-table sv-compare">
        <thead>
          <tr>
            <th scope="col">Dato</th>
            <th scope="col">Últimas 10</th>
            <th scope="col">Anteriores</th>
            <th scope="col">Cambio</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => {
            const a = mean(recent, r.v);
            const b = mean(before, r.v);
            const diff = a != null && b != null ? a - b : null;
            const same = diff == null || Math.abs(diff) < r.eps;
            const better = diff != null && !same && (r.lower ? diff < 0 : diff > 0);
            return (
              <tr key={r.label}>
                <th scope="row">{r.label}</th>
                <td>{a != null ? r.fmt(a) : "—"}</td>
                <td>{b != null ? r.fmt(b) : "—"}</td>
                <td className={better ? "accent" : "muted"}>
                  {diff == null ? "—" : same ? "igual" : `${better ? "▲ mejor" : "▼ peor"}`}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </section>
  );
}

function MatchTable({ games, onOpen }: { games: Match[]; onOpen: (id: string) => void }) {
  const rows = [...games].reverse();
  const opt = (v: number | null | undefined, f: (n: number) => string) => (v == null ? "—" : f(v));
  return (
    <section className="section" aria-labelledby="sv-tabla">
      <div className="section-head">
        <h2 id="sv-tabla" className="section-title">
          Todas las partidas
        </h2>
        <span className="label">De la más nueva a la más vieja</span>
      </div>
      <div className="sv-table-wrap">
        <table className="sv-table">
          <thead>
            <tr>
              <th scope="col">Partida</th>
              <th scope="col">Puesto</th>
              <th scope="col">Elim.</th>
              <th scope="col">Derribos</th>
              <th scope="col">Cofres</th>
              <th scope="col">Daño</th>
              <th scope="col">Recibido</th>
              <th scope="col">Precisión</th>
              <th scope="col">Distancia</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((m) => (
              <tr key={m.id}>
                <th scope="row">
                  <button className="link-btn" onClick={() => onOpen(m.id)}>
                    {fmtDate(m.created_at)}
                  </button>
                  {m.mode && <span className="muted"> · {m.mode}</span>}
                </th>
                <td className={m.placement === 1 ? "accent" : ""}>
                  {hasPlacement(m) ? `#${m.placement}${m.total_players ? ` / ${m.total_players}` : ""}` : "—"}
                </td>
                <td>{m.kills}</td>
                <td>{m.knocks}</td>
                <td>{m.chests}</td>
                <td>{opt(m.damage_dealt, fmtNum)}</td>
                <td>{opt(m.damage_taken, fmtNum)}</td>
                <td>{opt(m.accuracy, fmtPct)}</td>
                <td>{opt(m.distance_m, fmtDistance)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
