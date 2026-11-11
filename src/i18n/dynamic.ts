// Textos del backend que llevan datos: [patrón del texto en español, reemplazo en inglés].
import dict from "./en/api";

/** Trozo de patrón sin anclas y su reemplazo; se combina con prefijos y sufijos que no capturan nada. */
type Part = [string, string];

const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
/** Frases fijas del diccionario, para combinarlas con las que llevan datos. */
const fixed = (keys: string[]): Part[] => keys.map((k) => [esc(k), dict[k] ?? k]);
const wrap = (parts: Part[], pre: string, post: string, enPre: string, enPost: string): [RegExp, string][] =>
  parts.map(([src, to]) => [new RegExp(`^${pre}${src}${post}$`), `${enPre}${to}${enPost}`]);
/** Resto técnico (salida de ffmpeg, error del sistema): puede traer saltos de línea. */
const TAIL = "([\\s\\S]*)";

// Títulos de clips y etiquetas de eventos (un clip automático toma la etiqueta de su evento).
const titles: Part[] = [
  ...fixed([
    "Eliminación",
    "Eliminación (bot)",
    "Derribo",
    "Derribo (bot)",
    "Eliminación de la victoria",
    "Eliminación de la victoria (bot)",
    "Remontada · eliminación tras un derribo",
    "Remontada · eliminación tras un derribo (bot)",
    "Te derribaron",
    "Eliminado (tormenta/caída)",
    "Te eliminaron",
    "Momento destacado",
    "Momento marcado",
    "¡Victoria magistral!",
    "Victoria magistral",
    "Doble eliminación",
    "Triple eliminación",
    "Cofre abierto",
    "Clip guardado",
    "Resumen · Victoria",
    "Resumen · Victoria · 1 eliminación",
    "Resumen · 1 eliminación",
    "Resumen de la partida",
  ]),
  ["Tiro largo · (\\d+) m( \\(bot\\))?", "Long shot · $1 m$2"],
  ["Multi eliminación ×(\\d+)", "Multi elimination ×$1"],
  ["Clip de (\\d+) s", "$1 s clip"],
  ["Unión de (\\d+) clips", "$1 clips joined"],
  ["Resumen · Victoria · (\\d+) eliminaciones", "Recap · Victory · $1 eliminations"],
  ["Resumen · (\\d+) eliminaciones", "Recap · $1 eliminations"],
  ["Lo mejor de la semana · (.+)", "Best of the week · $1"],
];

// Errores: frases fijas y con datos, solas o detrás de un prefijo ("No se pudo grabar: …").
const contexts = [
  "ejecutando ffmpeg",
  "intercambiando el código",
  "renovando el acceso a Google Drive (vuelve a conectar la cuenta)",
  "actualizando el archivo en Drive",
  "descargando de Drive",
  "Esta PC no está conectada a una red local",
  "no se pudo ejecutar PowerShell",
  "puerto local",
];

const errors: Part[] = [
  ...fixed([
    "clip no encontrado",
    "El video está solo en Drive: descárgalo para editarlo",
    "El video está solo en Drive: descárgalo para volver a analizarlo",
    "El video está solo en Drive: descárgalo primero",
    "El clip está solo en Drive: descárgalo primero",
    "Este archivo no está en Drive",
    "este archivo no está en Drive",
    "Este video está solo en Google Drive: compártelo por Drive o descárgalo de nuevo",
    "El modo móvil necesita un video horizontal",
    "Conecta tu cuenta de Google Drive en Ajustes",
    "Ya se está subiendo",
    "Ya se está transfiriendo",
    "la partida no tiene video",
    "ffmpeg falló",
    "Falta el Client ID de Google (Ajustes → Google Drive)",
    "se agotó el tiempo de autorización",
    "Google no devolvió refresh_token",
    "Drive no devolvió el id de la carpeta",
    "Drive no abrió la sesión de subida",
    "Drive no devolvió el id",
    "el dispositivo de audio no respondió",
    "hilo de audio caído",
    "ffmpeg no empezó a grabar en 5 s",
    "no se obtuvo el inicio del video",
    "id inválido",
    "sin duración",
    ...contexts,
  ]),
  ...contexts.map((k): Part => [`${esc(k)}: ${TAIL}`, `${dict[k] ?? k}: $1`]),
  ["No se pudo cambiar el firewall \\(código (.*)\\)", "Couldn't change the firewall (code $1)"],
  [`Google rechazó la autorización: ${TAIL}`, "Google rejected the authorization: $1"],
  [`Drive respondió (.+?): ${TAIL}`, "Drive responded $1: $2"],
  ["la descarga se cortó \\((\\d+) de (\\d+) bytes\\)", "the download was cut off ($1 of $2 bytes)"],
  [`lectura de audio: ${TAIL}`, "audio read: $1"],
  [`ffmpeg terminó al iniciar \\((.*?)\\): ${TAIL}`, "ffmpeg exited on startup ($1): $2"],
  [`no se pudo ejecutar (.+?): ${TAIL}`, "couldn't run $1: $2"],
  [`no se pudo abrir (.+?): ${TAIL}`, "couldn't open $1: $2"],
  ["clip (\\S+) no encontrado", "clip $1 not found"],
  ['El clip "(.*)" está solo en Drive: descárgalo primero', 'The clip "$1" is only in Drive: download it first'],
  ["Se pausó porque empezó una partida: (\\d+) clips comprimidos hasta ahora", "Paused because a match started: $1 clips compressed so far"],
  [`puerto (\\d+): ${TAIL}`, "port $1: $2"],
];

const prefixes: Part[] = [
  ["No se pudo grabar: ", "Couldn't record: "],
  ["No se pudo iniciar el búfer: ", "Couldn't start the buffer: "],
  ["No se pudo borrar de Drive, la partida sigue aquí: ", "Couldn't delete from Drive, the match is still here: "],
  ["Guardado en la PC, pero no se pudo actualizar en Drive: ", "Saved on the PC, but couldn't update it in Drive: "],
  ["No se pudo borrar de Drive: ", "Couldn't delete from Drive: "],
];

export const dynamic: [RegExp, string][] = [
  // Títulos de partidas
  [/^Victoria · (\d+) eliminaciones$/, "Victory · $1 eliminations"],
  [/^(\d+) eliminaciones$/, "$1 eliminations"],

  // Títulos de clips y etiquetas de eventos, solos y con sus variantes
  ...wrap(titles, "", "", "", ""),
  ...wrap(titles, "Mejor jugada · ", "", "Best play · ", ""),
  ...wrap(titles, "", " · vertical", "", " · vertical"),
  ...wrap(titles, "", " · móvil", "", " · mobile"),
  ...wrap(titles, "Mejor jugada · ", " · vertical", "Best play · ", " · vertical"),
  ...wrap(titles, "Mejor jugada · ", " · móvil", "Best play · ", " · mobile"),

  // Pasos
  [/^Creando clips \((\d+)\/(\d+)\)$/, "Creating clips ($1/$2)"],
  [/^Comprimiendo clips \((\d+)\/(\d+)\)$/, "Compressing clips ($1/$2)"],

  // Avisos del overlay
  [/^Clip de (\d+) s guardado · se exporta al terminar$/, "$1 s clip saved · it's exported when recording ends"],
  [/^Clip de (\d+) s guardado$/, "$1 s clip saved"],
  [/^Guardando los últimos (\d+) s…$/, "Saving the last $1 s…"],
  [/^Grabando (.+)$/, "Recording $1"],

  // Errores
  ...wrap(errors, "", "", "", ""),
  ...prefixes.flatMap(([pre, enPre]) => wrap(errors, esc(pre), "", enPre, "")),
  ...prefixes.map(([pre, enPre]): [RegExp, string] => [new RegExp(`^${esc(pre)}${TAIL}$`), `${enPre}$1`]),

  // Títulos con un nombre propio (del juego, o el que escribió el usuario)
  [/^Mejor jugada · (.+) · móvil$/, "Best play · $1 · mobile"],
  [/^Mejor jugada · (.+)$/, "Best play · $1"],
  [/^(.+) · móvil$/, "$1 · mobile"],
  [/^(.+) · sesión$/, "$1 · session"],
];
