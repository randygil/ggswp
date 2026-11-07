import type { Match } from "./api";

/** Formatos del resumen de fin de partida (puesto, daño, distancia…). */

const num = new Intl.NumberFormat("es");

export const fmtNum = (n: number) => num.format(Math.round(n));

/** 439 m · 3,4 km */
export function fmtDistance(m: number): string {
  if (m < 1000) return `${Math.round(m)} m`;
  return `${(m / 1000).toLocaleString("es", { maximumFractionDigits: m < 10_000 ? 1 : 0 })} km`;
}

/** 0.378 → "38 %" */
export const fmtPct = (x: number) => `${Math.round(x * 100)} %`;

export const hasPlacement = (m: Match): m is Match & { placement: number } => m.placement != null && m.placement > 0;

/** Filas secundarias del resumen, solo las que el replay trajo. */
export function summaryRows(m: Match): [string, string][] {
  const rows: [string, string][] = [];
  if (m.damage_dealt != null) rows.push(["Daño hecho", fmtNum(m.damage_dealt)]);
  if (m.damage_taken != null) rows.push(["Daño recibido", fmtNum(m.damage_taken)]);
  if (m.accuracy != null) rows.push(["Precisión", fmtPct(m.accuracy)]);
  if (m.distance_m != null) rows.push(["Distancia", fmtDistance(m.distance_m)]);
  if (m.assists != null) rows.push(["Asistencias", fmtNum(m.assists)]);
  if (m.revives) rows.push(["Reanimaciones", fmtNum(m.revives)]);
  if (m.damage_to_structures != null) rows.push(["Daño a estructuras", fmtNum(m.damage_to_structures)]);
  if (m.materials_gathered) rows.push(["Materiales juntados", fmtNum(m.materials_gathered)]);
  if (m.materials_used) rows.push(["Materiales usados", fmtNum(m.materials_used)]);
  return rows;
}
