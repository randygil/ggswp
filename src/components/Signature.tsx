import { fmtClock, type Clip, type MatchEvent } from "../api";
import { t, tr } from "../i18n";

/** Paso de la regla: el primero que deja como mucho ~12 marcas. */
export function rulerStep(duration: number) {
  return [10, 15, 30, 60, 120, 300, 600, 1200].find((s) => duration / s <= 12) ?? 1800;
}

type Flag = { t: number; label: string; plain: boolean };

/** Etiquetas sobre la firma: victoria, multi-eliminaciones, tiros largos y marcas; sin pisarse. */
function flagsFor(events: MatchEvent[], clips: Clip[], duration: number): Flag[] {
  const raw: Flag[] = [];
  for (const e of events) {
    if (e.kind === "victory") raw.push({ t: e.t, label: t("Victoria"), plain: false });
    else if (e.kind === "bookmark") raw.push({ t: e.t, label: t("Marca"), plain: true });
    else if (e.kind === "kill" && (e.distance_m ?? 0) >= 100) raw.push({ t: e.t, label: `${Math.round(e.distance_m!)} m`, plain: false });
    else if (e.kind === "bus") raw.push({ t: e.t, label: t("Autobús"), plain: true });
  }
  for (const c of clips) {
    if (/Doble|Triple|Multi/.test(c.title)) raw.push({ t: (c.start + c.end) / 2, label: tr(c.title), plain: false });
  }
  raw.sort((a, b) => a.t - b.t);
  const out: Flag[] = [];
  const minGap = duration * 0.09;
  for (const f of raw) {
    const last = out[out.length - 1];
    if (last && f.t - last.t < minGap) {
      // Ante un choque, gana la etiqueta con acento (más informativa).
      if (last.plain && !f.plain) out[out.length - 1] = f;
      continue;
    }
    out.push(f);
  }
  return out;
}

const pct = (t: number, d: number) => `${(Math.max(0, Math.min(t, d)) / Math.max(d, 0.001)) * 100}%`;

/** La partida como "firma": cada evento es una marca sobre la duración. */
export function Signature(props: { events: MatchEvent[]; duration: number; clips?: Clip[]; variant?: "mini" | "full" }) {
  const { events, duration: d, clips = [], variant = "mini" } = props;
  const ticks = events.map((e, i) => <span key={i} className={`sig-tick ${e.kind}`} style={{ left: pct(e.t, d) }} />);

  if (variant === "mini") {
    return (
      <div className="sig mini" aria-hidden>
        {ticks}
      </div>
    );
  }

  const step = rulerStep(d);
  const marks = Array.from({ length: Math.floor(d / step) + 1 }, (_, i) => i * step).filter((t) => t <= d - step * 0.4 || t === 0);
  const flags = flagsFor(events, clips, d);
  return (
    <div className="sig full" aria-hidden>
      <div className="sig-plot">
        {clips.map((c) => (
          <span key={c.id} className="sig-clip" style={{ left: pct(c.start, d), width: pct(c.end - c.start, d) }} />
        ))}
        {ticks}
      </div>
      {flags.map((f, i) => {
        const p = f.t / Math.max(d, 1);
        const edge = p < 0.06 ? "edge-l" : p > 0.94 ? "edge-r" : "";
        return (
          <span key={i} className={`sig-flag ${f.plain ? "plain" : ""} ${edge}`} style={{ left: pct(f.t, d) }}>
            {f.label}
          </span>
        );
      })}
      <div className="sig-ruler">
        {marks.map((t) => (
          <span key={t} style={{ left: pct(t, d) }}>
            {fmtClock(t)}
          </span>
        ))}
      </div>
    </div>
  );
}
