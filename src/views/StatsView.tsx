import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { fmtDate, fmtPlayed, type Match, type Totals } from "../api";
import { Icon } from "../components/Icon";
import { locale, t } from "../i18n";
import { fmtDistance, fmtNum, fmtPct, hasPlacement } from "../matchStats";

/** Solo las partidas reales de Fortnite (no las grabaciones manuales), de la más vieja a la más nueva. */
const isGame = (m: Match) => m.status === "ready" && (!!m.replay || !!m.account_id);

const shortDay = (iso: string) => new Date(iso).toLocaleDateString(locale(), { day: "numeric", month: "short" });
const avg = (xs: number[]) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);
const dec = (n: number, d = 1) => n.toLocaleString(locale(),{ maximumFractionDigits: d });

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
          <h1>{t("Estadísticas")}</h1>
          <p>{t("Tu histórico completo y cómo te fue partida a partida, con las partidas que siguen en la biblioteca.")}</p>
        </div>
        {games.length > 0 && (
          <button className="btn ghost" onClick={() => setTable((on) => !on)} aria-pressed={table}>
            {table ? t("Ver gráficos") : t("Ver tabla")}
          </button>
        )}
      </header>

      {totals && totals.matches > 0 && <Lifetime t={totals} />}

      {games.length === 0 ? (
        <div className="empty">
          <Icon name="chart" size={36} />
          <h2>{t("Sin partidas para graficar")}</h2>
          <p>{t("Cuando termines una partida, aquí verás tus eliminaciones, cofres, puesto y daño en el tiempo.")}</p>
        </div>
      ) : table ? (
        <MatchTable games={games} onOpen={onOpen} />
      ) : (
        <>
          <section className="section" aria-labelledby="sv-por-partida">
            <div className="section-head">
              <h2 id="sv-por-partida" className="section-title">
                {t("Por partida")}
              </h2>
              <span className="label">
                {t(games.length === 1 ? "{n} partida · de {from} a {to}" : "{n} partidas · de {from} a {to}", {
                  n: games.length,
                  from: shortDay(games[0].created_at),
                  to: shortDay(games[games.length - 1].created_at),
                })}
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

function Lifetime({ t: tot }: { t: Totals }) {
  const tiles: { label: string; value: string; sub?: string; accent?: boolean }[] = [
    { label: t("Partidas"), value: fmtNum(tot.matches), sub: t("{time} jugado", { time: fmtPlayed(tot.seconds) }) },
    {
      label: t("Victorias"),
      value: fmtNum(tot.victories),
      sub: tot.matches ? t("{pct} % de las partidas", { pct: dec((tot.victories / tot.matches) * 100) }) : undefined,
      accent: tot.victories > 0,
    },
    { label: t("Eliminaciones"), value: fmtNum(tot.kills), sub: tot.matches ? t("{n} por partida", { n: dec(tot.kills / tot.matches) }) : undefined },
    { label: t("Derribos"), value: fmtNum(tot.knocks) },
    { label: t("Cofres"), value: fmtNum(tot.chests), sub: tot.matches ? t("{n} por partida", { n: dec(tot.chests / tot.matches) }) : undefined },
  ];
  if (tot.best_placement != null) tiles.push({ label: t("Mejor puesto"), value: `#${tot.best_placement}`, accent: tot.best_placement === 1 });
  if (tot.avg_placement != null)
    tiles.push({ label: t("Puesto promedio"), value: `#${dec(tot.avg_placement)}`, sub: t("en {n} partidas con puesto", { n: tot.placed_matches ?? 0 }) });
  if (tot.damage_dealt) tiles.push({ label: t("Daño hecho"), value: fmtNum(tot.damage_dealt) });
  if (tot.distance_m) tiles.push({ label: t("Distancia"), value: fmtDistance(tot.distance_m) });
  return (
    <section className="section" aria-labelledby="sv-historico">
      <div className="section-head">
        <h2 id="sv-historico" className="section-title">
          {t("Histórico")}
        </h2>
        <span className="label" title={t("Siguen sumando aunque las partidas venzan o se borren")}>
          {tot.since
            ? t("Desde {date}", { date: new Date(tot.since).toLocaleDateString(locale(), { day: "numeric", month: "long", year: "numeric" }) })
            : t("Todas tus partidas")}
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
      <h3 id={`${id}-t`}>{t(s.title)}</h3>
      <span className="label">
        {mean == null ? t("sin datos") : t("prom. {v}", { v: s.lowerIsBetter ? `#${dec(mean)}` : dec(mean) })}
        {s.lowerIsBetter && ` · ${t("más arriba es mejor")}`}
      </span>
    </div>
  );

  if (present.length === 0) {
    return (
      <figure className="sv-chart">
        {head}
        <p className="sv-nodata">{t("Aún no hay datos: se leen del replay de Fortnite al terminar cada partida.")}</p>
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
  const desc = t(
    s.lowerIsBetter
      ? "{n} partidas con dato. Promedio {avg}, mejor {best}, última {last}. Usa las flechas para recorrer las partidas y Enter para abrir una."
      : "{n} partidas con dato. Promedio {avg}, máximo {best}, última {last}. Usa las flechas para recorrer las partidas y Enter para abrir una.",
    {
      n: present.length,
      avg: mean != null ? s.fmt(mean) : "—",
      best: s.fmt(extreme),
      last: vals[n - 1] != null ? s.fmt(vals[n - 1]!) : t("sin dato"),
    },
  );

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
            <b>{hv != null ? s.fmt(hv) : t("Sin dato")}</b>
            {s.lowerIsBetter && hv != null && games[hover].total_players ? <span> {t("de {n}", { n: games[hover].total_players! })}</span> : null}
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
          {t("Últimas 10 vs anteriores")}
        </h2>
        <span className="label">{t("Promedio por partida · {n} anteriores", { n: before.length })}</span>
      </div>
      <table className="sv-table sv-compare">
        <thead>
          <tr>
            <th scope="col">{t("Dato")}</th>
            <th scope="col">{t("Últimas 10")}</th>
            <th scope="col">{t("Anteriores")}</th>
            <th scope="col">{t("Cambio")}</th>
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
                <th scope="row">{t(r.label)}</th>
                <td>{a != null ? r.fmt(a) : "—"}</td>
                <td>{b != null ? r.fmt(b) : "—"}</td>
                <td className={better ? "accent" : "muted"}>
                  {diff == null ? "—" : same ? t("igual") : `${better ? t("▲ mejor") : t("▼ peor")}`}
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
          {t("Todas las partidas")}
        </h2>
        <span className="label">{t("De la más nueva a la más vieja")}</span>
      </div>
      <div className="sv-table-wrap">
        <table className="sv-table">
          <thead>
            <tr>
              <th scope="col">{t("Partida")}</th>
              <th scope="col">{t("Puesto")}</th>
              <th scope="col">{t("Elim.")}</th>
              <th scope="col">{t("Derribos")}</th>
              <th scope="col">{t("Cofres")}</th>
              <th scope="col">{t("Daño")}</th>
              <th scope="col">{t("Recibido")}</th>
              <th scope="col">{t("Precisión")}</th>
              <th scope="col">{t("Distancia")}</th>
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
