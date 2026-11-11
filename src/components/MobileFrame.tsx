import { useEffect, useRef, useState, type RefObject } from "react";
import type { MobileLayout, Region } from "../api";

/** HUD de Fortnite a escala 100 % en 16:9 (igual que `MobileLayout::default` en Rust). */
export const DEFAULT_MOBILE_LAYOUT: MobileLayout = {
  health: { x: 0.015, y: 0.872, w: 0.29, h: 0.085 },
  weapons: { x: 0.685, y: 0.855, w: 0.302, h: 0.092 },
  focus: 0.5,
  two_rows: false,
};

const OUT_W = 1080;
const OUT_H = 1920;
const MARGIN = 24;
const GAP = 16;
const STRIP_Y = 150;
const BAND_PAD = 14;
const MAX_ROW_H = 220;
const TWO_ROWS_W = 760;

type Box = { x: number; y: number; w: number; h: number };

/**
 * Dónde va cada pieza en el cuadro de 1080x1920. Es la misma cuenta que `mobile_geometry`
 * en `export.rs` (la que usa el render): si cambia una, cambia la otra.
 */
export function mobileGeometry(l: MobileLayout, w: number, h: number) {
  const gw = Math.min((h * 9) / 16, w);
  const gx = Math.max(0, Math.min(l.focus * w - gw / 2, w - gw));
  const crop = (r: Region): Box => ({ x: r.x * w, y: r.y * h, w: r.w * w, h: r.h * h });
  const [hc, wc] = [crop(l.health), crop(l.weapons)];
  const [ah, aw] = [hc.w / hc.h, wc.w / wc.h];
  let health: Box, weapons: Box, bandH: number;
  if (l.two_rows) {
    const row = (a: number) => {
      const rh = Math.min(TWO_ROWS_W / a, MAX_ROW_H);
      return { w: rh * a, h: rh };
    };
    const [hs, ws] = [row(ah), row(aw)];
    health = { ...hs, x: (OUT_W - hs.w) / 2, y: STRIP_Y };
    weapons = { ...ws, x: (OUT_W - ws.w) / 2, y: STRIP_Y + hs.h + GAP };
    bandH = hs.h + GAP + ws.h;
  } else {
    const rh = Math.min((OUT_W - 2 * MARGIN - GAP) / (ah + aw), MAX_ROW_H);
    const x0 = Math.max(0, (OUT_W - (rh * ah + GAP + rh * aw)) / 2);
    health = { x: x0, y: STRIP_Y, w: rh * ah, h: rh };
    weapons = { x: x0 + rh * ah + GAP, y: STRIP_Y, w: rh * aw, h: rh };
    bandH = rh;
  }
  return { game: { x: gx, y: 0, w: gw, h }, band: { y: STRIP_Y - BAND_PAD, h: bandH + 2 * BAND_PAD }, health: { crop: hc, to: health }, weapons: { crop: wc, to: weapons } };
}

/** Rectángulo que ocupa la imagen dentro del elemento <video> (que puede tener bandas). */
function useContentBox(video: RefObject<HTMLVideoElement | null>) {
  const [box, setBox] = useState<Box | null>(null);
  useEffect(() => {
    const v = video.current;
    if (!v) return;
    const measure = () => {
      if (!v.videoWidth) return;
      const s = Math.min(v.clientWidth / v.videoWidth, v.clientHeight / v.videoHeight);
      const [w, h] = [v.videoWidth * s, v.videoHeight * s];
      setBox({ x: v.offsetLeft + (v.clientWidth - w) / 2, y: v.offsetTop + (v.clientHeight - h) / 2, w, h });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(v);
    v.addEventListener("loadedmetadata", measure);
    return () => {
      ro.disconnect();
      v.removeEventListener("loadedmetadata", measure);
    };
  }, [video]);
  return box;
}

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, hi));
const pct = (v: number) => `${v * 100}%`;

/**
 * Zonas del modo móvil sobre el video: la salud y las armas se mueven y se estiran, y el
 * encuadre del juego se corre a los lados. `onCommit` avisa al soltar.
 */
export function MobileOverlay(props: { video: RefObject<HTMLVideoElement | null>; layout: MobileLayout; onChange: (l: MobileLayout) => void; onCommit: (l: MobileLayout) => void }) {
  const { video, layout, onChange, onCommit } = props;
  const box = useContentBox(video);
  const drag = useRef<{ what: "health" | "weapons" | "focus"; resize: boolean; x: number; y: number; start: MobileLayout } | null>(null);
  const latest = useRef(layout);
  latest.current = layout;
  if (!box) return null;

  const begin = (e: React.PointerEvent, what: "health" | "weapons" | "focus", resize = false) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag.current = { what, resize, x: e.clientX, y: e.clientY, start: layout };
  };
  const move = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const [dx, dy] = [(e.clientX - d.x) / box.w, (e.clientY - d.y) / box.h];
    if (d.what === "focus") {
      const half = ((video.current!.videoHeight * 9) / 16 / video.current!.videoWidth) / 2;
      onChange({ ...d.start, focus: clamp(d.start.focus + dx, half, 1 - half) });
      return;
    }
    const r = d.start[d.what];
    const next: Region = d.resize
      ? { ...r, w: clamp(r.w + dx, 0.04, 1 - r.x), h: clamp(r.h + dy, 0.03, 1 - r.y) }
      : { ...r, x: clamp(r.x + dx, 0, 1 - r.w), y: clamp(r.y + dy, 0, 1 - r.h) };
    onChange({ ...d.start, [d.what]: next });
  };
  const end = () => {
    if (drag.current) onCommit(latest.current);
    drag.current = null;
  };

  const v = video.current!;
  const gw = (v.videoHeight * 9) / 16 / v.videoWidth;
  const gx = clamp(layout.focus - gw / 2, 0, 1 - gw);
  const region = (what: "health" | "weapons", label: string) => {
    const r = layout[what];
    return (
      <div
        className="mf-region"
        style={{ left: pct(r.x), top: pct(r.y), width: pct(r.w), height: pct(r.h) }}
        onPointerDown={(e) => begin(e, what)}
        onPointerMove={move}
        onPointerUp={end}
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={(e) => e.stopPropagation()}
        title={`Zona de ${label.toLowerCase()}: arrástrala para moverla`}
      >
        <span className="mf-label">{label}</span>
        <span className="mf-grip" onPointerDown={(e) => begin(e, what, true)} onPointerMove={move} onPointerUp={end} title="Arrastra para cambiar el tamaño" />
      </div>
    );
  };

  return (
    <div className="mf-overlay" style={{ left: box.x, top: box.y, width: box.w, height: box.h }}>
      <div className="mf-shade" style={{ left: 0, width: pct(gx) }} />
      <div className="mf-shade" style={{ left: pct(gx + gw), right: 0 }} />
      <div className="mf-game" style={{ left: pct(gx), width: pct(gw) }}>
        <span
          className="mf-label mf-handle"
          onPointerDown={(e) => begin(e, "focus")}
          onPointerMove={move}
          onPointerUp={end}
          onClick={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
          title="Arrastra para correr el encuadre del juego"
        >
          ↔ Encuadre
        </span>
      </div>
      {region("health", "Salud")}
      {region("weapons", "Armas")}
    </div>
  );
}

/** Vista previa en vivo del cuadro vertical, dibujada con los mismos recortes que el render. */
export function MobilePreview({ video, layout }: { video: RefObject<HTMLVideoElement | null>; layout: MobileLayout }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const latest = useRef(layout);
  latest.current = layout;
  const [height, setHeight] = useState(0);

  // Tan alta como el video que tiene al lado.
  useEffect(() => {
    const v = video.current;
    if (!v) return;
    const ro = new ResizeObserver(() => setHeight(v.clientHeight));
    ro.observe(v);
    setHeight(v.clientHeight);
    return () => ro.disconnect();
  }, [video]);

  useEffect(() => {
    let raf = 0;
    const draw = () => {
      raf = requestAnimationFrame(draw);
      const [v, c] = [video.current, canvas.current];
      const ctx = c?.getContext("2d");
      if (!v || !c || !ctx || v.readyState < 2 || !v.videoWidth) return;
      const g = mobileGeometry(latest.current, v.videoWidth, v.videoHeight);
      const k = c.width / OUT_W;
      ctx.drawImage(v, g.game.x, g.game.y, g.game.w, g.game.h, 0, 0, c.width, c.height);
      ctx.fillStyle = "rgb(0 0 0 / 0.45)";
      ctx.fillRect(0, g.band.y * k, c.width, g.band.h * k);
      for (const p of [g.health, g.weapons]) ctx.drawImage(v, p.crop.x, p.crop.y, p.crop.w, p.crop.h, p.to.x * k, p.to.y * k, p.to.w * k, p.to.h * k);
    };
    draw();
    return () => cancelAnimationFrame(raf);
  }, [video]);

  return (
    <canvas
      ref={canvas}
      className="mf-preview"
      width={OUT_W / 2}
      height={OUT_H / 2}
      style={{ height, width: (height * 9) / 16 }}
      role="img"
      aria-label="Vista previa del clip en modo móvil"
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
    />
  );
}
