import { open } from "@tauri-apps/plugin-dialog";
import type { RefObject } from "react";
import { fmtClock, TRACK_LABEL, type Frame, type Fx, type FxMoment, type FxText } from "../api";
import { t } from "../i18n";
import { Icon } from "./Icon";
import { TEXT_COLORS, TEXT_FONT, useContentBox } from "./MobileFrame";

/** Efectos que se están armando en el editor. */
export interface FxState {
  texts: FxText[];
  moment: FxMoment | null;
  music: { path: string; volume: number } | null;
  /** Volumen del audio de la grabación (1 = igual). */
  volume: number;
  /** Fuentes de audio apagadas a mano (las demás van). */
  muted: string[];
}

export const EMPTY_FX: FxState = { texts: [], moment: null, music: null, volume: 1, muted: [] };

/** Lo que se le manda al exportador. `sources` son las fuentes que tiene la grabación. */
export function toFx(s: FxState, sources: string[]): Fx {
  return {
    texts: s.texts.filter((t) => t.text.trim()),
    moment: s.moment,
    music: s.music,
    // Sin fuentes separadas no hay nada que elegir: decide el exportador.
    tracks: sources.length ? sources.filter((k) => !s.muted.includes(k)).map((k) => (k === "system" ? "game" : k)) : null,
    volume: s.volume,
  };
}

export const hasFx = (s: FxState) => s.texts.some((t) => t.text.trim()) || !!s.moment || !!s.music || s.volume !== 1 || s.muted.length > 0;

const SIZES: [string, number][] = [
  ["Chico", 0.045],
  ["Mediano", 0.07],
  ["Grande", 0.1],
  ["Enorme", 0.14],
];
const COLORS: [FxText["color"], string][] = [
  ["white", "Blanco"],
  ["accent", "Verde"],
  ["yellow", "Amarillo"],
];
const pct = (v: number) => t("{n} %", { n: Math.round(v * 100) });
const secs = (v: number) => `${fmtClock(v)}.${Math.floor((v % 1) * 10)}`;
const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;

export function FxPanel(props: {
  fx: FxState;
  onChange: (fx: FxState) => void;
  /** Fuentes de audio separadas de la grabación (vacío si es una sola pista). */
  sources: string[];
  /** Duración del tramo que se va a exportar. */
  length: number;
  /** Posición del cursor dentro del tramo, en segundos. */
  now: number;
}) {
  const { fx, onChange, sources, length, now } = props;
  const set = (patch: Partial<FxState>) => onChange({ ...fx, ...patch });
  const setText = (i: number, patch: Partial<FxText>) => set({ texts: fx.texts.map((t, n) => (n === i ? { ...t, ...patch } : t)) });
  const here = Math.max(0, Math.min(now, length));
  const m = fx.moment;
  const setMoment = (patch: Partial<FxMoment>) => m && set({ moment: { ...m, ...patch } });

  const pickMusic = async () => {
    const path = await open({ multiple: false, filters: [{ name: t("Audio"), extensions: ["mp3", "wav", "m4a", "aac", "ogg", "flac", "opus"] }] });
    if (typeof path === "string") set({ music: { path, volume: fx.music?.volume ?? 0.6 } });
  };

  return (
    <div className="fx-panel">
      <div className="fx-group">
        <span className="label">{t("Texto")}</span>
        {fx.texts.map((x, i) => (
          <div className="fx-row" key={i}>
            <input className="input" aria-label={t("Texto {n}", { n: i + 1 })} placeholder={t("Escribe el texto")} value={x.text} maxLength={60} autoFocus={!x.text} onChange={(e) => setText(i, { text: e.target.value })} />
            <select aria-label={t("Tamaño")} value={x.size} onChange={(e) => setText(i, { size: Number(e.target.value) })}>
              {SIZES.map(([label, v]) => (
                <option key={v} value={v}>
                  {t(label)}
                </option>
              ))}
            </select>
            <select aria-label={t("Color")} value={x.color} onChange={(e) => setText(i, { color: e.target.value as FxText["color"] })}>
              {COLORS.map(([v, label]) => (
                <option key={v} value={v}>
                  {t(label)}
                </option>
              ))}
            </select>
            <label className="fx-slider" title={t("Altura del texto en el cuadro")}>
              {t("Altura")}
              <input type="range" min={0.04} max={0.96} step={0.01} value={x.y} onChange={(e) => setText(i, { y: Number(e.target.value) })} />
            </label>
            <select
              aria-label={t("Cuándo se ve")}
              value={x.to > 0 ? x.to : 0}
              onChange={(e) => setText(i, { from: 0, to: Number(e.target.value) })}
              title={t("Cuánto tiempo se ve el texto")}
            >
              <option value={0}>{t("Todo el clip")}</option>
              <option value={3}>{t("Primeros {n} s", { n: 3 })}</option>
              <option value={5}>{t("Primeros {n} s", { n: 5 })}</option>
            </select>
            <button className="icon-btn sm bare danger" title={t("Quitar este texto")} aria-label={t("Quitar el texto {n}", { n: i + 1 })} onClick={() => set({ texts: fx.texts.filter((_, n) => n !== i) })}>
              <Icon name="x" size={15} />
            </button>
          </div>
        ))}
        {fx.texts.length < 4 && (
          <button className="btn ghost sm" onClick={() => set({ texts: [...fx.texts, { text: "", y: fx.texts.length ? 0.86 : 0.12, size: 0.07, color: "white", from: 0, to: 0 }] })}>
            {t("+ Texto")}
          </button>
        )}
      </div>

      <div className="fx-group">
        <span className="label">{t("Cámara lenta y zoom")}</span>
        {m ? (
          <div className="fx-row">
            <select aria-label={t("Velocidad")} value={m.speed} onChange={(e) => setMoment({ speed: Number(e.target.value) })}>
              <option value={1}>{t("Velocidad normal")}</option>
              <option value={0.5}>{t("Mitad de velocidad")}</option>
              <option value={0.25}>{t("Un cuarto de velocidad")}</option>
            </select>
            <select aria-label={t("Zoom")} value={m.zoom} onChange={(e) => setMoment({ zoom: Number(e.target.value) })} title={t("Acerca al centro de la pantalla, donde está el punto de mira")}>
              <option value={1}>{t("Sin zoom")}</option>
              <option value={1.5}>{t("Zoom 1,5×")}</option>
              <option value={2}>{t("Zoom 2×")}</option>
              <option value={3}>{t("Zoom 3×")}</option>
            </select>
            <span className="fx-times">
              {secs(m.start)} <i>→</i> {secs(m.end)}
            </span>
            <button className="btn ghost sm" disabled={here >= m.end - 0.2} onClick={() => setMoment({ start: here })} title={t("El efecto empieza donde está el cursor")}>
              {t("Empieza aquí")}
            </button>
            <button className="btn ghost sm" disabled={here <= m.start + 0.2} onClick={() => setMoment({ end: here })} title={t("El efecto termina donde está el cursor")}>
              {t("Termina aquí")}
            </button>
            <button className="icon-btn sm bare danger" title={t("Quitar el efecto")} aria-label={t("Quitar la cámara lenta y el zoom")} onClick={() => set({ moment: null })}>
              <Icon name="x" size={15} />
            </button>
          </div>
        ) : (
          <button
            className="btn ghost sm"
            title={t("Pon el cursor en el momento clave: el efecto toma un segundo antes y uno después")}
            onClick={() => {
              const c = here > 0.2 && here < length - 0.2 ? here : length / 2;
              set({ moment: { start: Math.max(0, c - 1), end: Math.min(length, c + 1), speed: 0.5, zoom: 1 } });
            }}
          >
            {t("+ Cámara lenta en el cursor")}
          </button>
        )}
      </div>

      <div className="fx-group">
        <span className="label">{t("Audio")}</span>
        <div className="fx-row">
          {sources.map((k) => (
            <label className="fx-check" key={k}>
              <input type="checkbox" checked={!fx.muted.includes(k)} onChange={(e) => set({ muted: e.target.checked ? fx.muted.filter((x) => x !== k) : [...fx.muted, k] })} />
              {t(TRACK_LABEL[k] ?? k)}
            </label>
          ))}
          <label className="fx-slider" title={t("Volumen del audio de la grabación")}>
            {t("Volumen {pct}", { pct: pct(fx.volume) })}
            <input type="range" min={0} max={1.5} step={0.05} value={fx.volume} onChange={(e) => set({ volume: Number(e.target.value) })} />
          </label>
        </div>
        <div className="fx-row">
          {fx.music ? (
            <>
              <span className="fx-file" title={fx.music.path}>
                ♪ {baseName(fx.music.path)}
              </span>
              <label className="fx-slider" title={t("Volumen de la música")}>
                {t("Música {pct}", { pct: pct(fx.music.volume) })}
                <input type="range" min={0.05} max={1.5} step={0.05} value={fx.music.volume} onChange={(e) => set({ music: { ...fx.music!, volume: Number(e.target.value) } })} />
              </label>
              <button className="btn ghost sm" onClick={pickMusic}>
                {t("Cambiar")}
              </button>
              <button className="icon-btn sm bare danger" title={t("Quitar la música")} aria-label={t("Quitar la música")} onClick={() => set({ music: null })}>
                <Icon name="x" size={15} />
              </button>
            </>
          ) : (
            <button className="btn ghost sm" onClick={pickMusic} title={t("Un archivo de audio de tu PC; se repite si es más corto que el clip y baja al final")}>
              {t("+ Música de fondo")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * Los textos sobre el video, como van a salir. En vertical se acomodan a la columna central
 * que queda después del recorte. (En modo móvil los dibuja la vista previa.)
 */
export function TextPreview(props: { video: RefObject<HTMLVideoElement | null>; texts: FxText[]; frame: Frame }) {
  const box = useContentBox(props.video);
  const texts = props.texts.filter((t) => t.text.trim());
  if (!box || !texts.length) return null;
  const w = props.frame === "vertical" ? Math.min(box.w, (box.h * 9) / 16) : box.w;
  const short = Math.min(w, box.h);
  return (
    <div className="fx-texts" style={{ left: box.x + (box.w - w) / 2, top: box.y, width: w, height: box.h }}>
      {texts.map((t, i) => {
        const px = short * t.size;
        return (
          <span key={i} style={{ top: `${t.y * 100}%`, fontSize: px, fontFamily: TEXT_FONT, color: TEXT_COLORS[t.color], WebkitTextStroke: `${Math.max(2, px * 0.14)}px rgb(0 0 0 / 0.9)` }}>
            {t.text}
          </span>
        );
      })}
    </div>
  );
}
