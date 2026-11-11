// Idiomas de la interfaz. Los textos se escriben en español en el código y esa misma frase
// es la clave del diccionario en inglés: `t("Guardar cambios")`.
import api from "./en/api";
import editor from "./en/editor";
import library from "./en/library";
import settings from "./en/settings";
import { dynamic } from "./dynamic";

export type Lang = "es" | "en";
export type LangPref = "auto" | Lang;

const en: Record<string, string> = { ...api, ...library, ...settings, ...editor };
let lang: Lang = "es";

/** Fija el idioma. `auto` usa el de Windows: español si lo es, inglés en cualquier otro caso. */
export function setLang(pref: LangPref | undefined) {
  lang = pref === "es" || pref === "en" ? pref : navigator.language.toLowerCase().startsWith("es") ? "es" : "en";
  document.documentElement.lang = lang;
}

export const getLang = () => lang;

/** Configuración regional para fechas y números. */
export const locale = () => (lang === "en" ? "en" : "es");

const fill = (s: string, vars?: Record<string, string | number>) => (vars ? s.replace(/\{(\w+)\}/g, (_, k) => String(vars[k] ?? "")) : s);

/** Texto fijo de la interfaz. Con `vars`, reemplaza `{nombre}` en la frase ya traducida. */
export function t(es: string, vars?: Record<string, string | number>): string {
  return fill(lang === "en" ? (en[es] ?? es) : es, vars);
}

/**
 * Texto que llega armado desde el backend (títulos de partidas y clips, etiquetas de
 * eventos, pasos, avisos y errores): frase exacta o, si lleva datos, por patrón.
 */
export function tr(text: string): string {
  if (lang !== "en" || !text) return text;
  if (en[text]) return en[text];
  for (const [re, to] of dynamic) {
    if (re.test(text)) return text.replace(re, to);
  }
  return text;
}
