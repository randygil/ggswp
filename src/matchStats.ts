import type { Match } from "./api";
import { locale, t } from "./i18n";

/** Formatos del resumen de fin de partida (puesto, daño, distancia…). */

export const fmtNum = (n: number) => Math.round(n).toLocaleString(locale());

/** 439 m · 3,4 km */
export function fmtDistance(m: number): string {
  if (m < 1000) return `${Math.round(m)} m`;
  return `${(m / 1000).toLocaleString(locale(), { maximumFractionDigits: m < 10_000 ? 1 : 0 })} km`;
}

/** 0.378 → "38 %" */
export const fmtPct = (x: number) => `${Math.round(x * 100)} %`;

export const hasPlacement = (m: Match): m is Match & { placement: number } => m.placement != null && m.placement > 0;

/** Filas secundarias del resumen, solo las que el replay trajo. */
export function summaryRows(m: Match): [string, string][] {
  const rows: [string, string][] = [];
  if (m.damage_dealt != null) rows.push([t("Daño hecho"), fmtNum(m.damage_dealt)]);
  if (m.damage_taken != null) rows.push([t("Daño recibido"), fmtNum(m.damage_taken)]);
  if (m.accuracy != null) rows.push([t("Precisión"), fmtPct(m.accuracy)]);
  if (m.distance_m != null) rows.push([t("Distancia"), fmtDistance(m.distance_m)]);
  if (m.assists != null) rows.push([t("Asistencias"), fmtNum(m.assists)]);
  if (m.revives) rows.push([t("Reanimaciones"), fmtNum(m.revives)]);
  if (m.damage_to_structures != null) rows.push([t("Daño a estructuras"), fmtNum(m.damage_to_structures)]);
  if (m.materials_gathered) rows.push([t("Materiales juntados"), fmtNum(m.materials_gathered)]);
  if (m.materials_used) rows.push([t("Materiales usados"), fmtNum(m.materials_used)]);
  return rows;
}
