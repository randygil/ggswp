import { convertFileSrc, invoke } from "@tauri-apps/api/core";

export type MatchStatus = "recording" | "processing" | "ready" | "error";

export interface MatchEvent {
  t: number;
  kind: "kill" | "knock" | "death" | "knocked" | "victory" | "highlight" | "bus" | "bookmark" | "chest";
  label: string;
  distance_m?: number;
}

export interface DriveRef {
  file_id: string;
  link: string;
}

export interface Clip {
  id: string;
  file: string;
  thumb?: string | null;
  start: number;
  end: number;
  title: string;
  kind: string;
  created_at: string;
  drive?: DriveRef | null;
  /** Solo está en Drive (la copia local se borró al subirlo). */
  offloaded?: boolean;
}

export interface Match {
  id: string;
  dir: string;
  created_at: string;
  title: string;
  mode?: string | null;
  playlist?: string | null;
  map?: string | null;
  duration_s: number;
  width: number;
  height: number;
  size_bytes: number;
  video?: string | null;
  thumb?: string | null;
  kills: number;
  knocks: number;
  /** Cofres abiertos (leídos de los avisos de PE del video). */
  chests: number;
  died: boolean;
  victory: boolean;
  events: MatchEvent[];
  clips: Clip[];
  drive?: DriveRef | null;
  replay?: string | null;
  status: MatchStatus;
  error?: string | null;
  /** Conservada: no vence. */
  keep: boolean;
  /** El video completo solo está en Drive. */
  offloaded: boolean;
}

/** Totales de todas las partidas jugadas (no bajan cuando una partida vence). */
export interface Totals {
  matches: number;
  kills: number;
  knocks: number;
  chests: number;
  victories: number;
  seconds: number;
  since?: string | null;
}

/** Paso en curso al procesar una partida. */
export interface Step {
  label: string;
  /** 0–100 si el paso se puede medir. */
  pct?: number | null;
}

export interface Status {
  /** Paso en curso de cada partida que se está procesando (por id). */
  steps: Record<string, Step>;
  fortnite_running: boolean;
  game_state: "idle" | "lobby" | "loading" | "in_match";
  recording?: { match_id: string; started_at: string; manual: boolean } | null;
  processing: string[];
  last_error?: string | null;
  account_id?: string | null;
  live: { t: number; kind: "highlight" | "bookmark" | "clip"; label: string }[];
  buffer_active: boolean;
}

export interface Hotkeys {
  save_clip: string;
  bookmark: string;
  toggle_recording: string;
  highlights_menu: string;
}

export interface DriveSettings {
  client_id: string;
  client_secret: string;
  refresh_token?: string | null;
  account_email?: string | null;
  folder_id?: string | null;
  auto_upload: boolean;
  upload_full_match: boolean;
  upload_clips: boolean;
  delete_local_after_upload: boolean;
}

export interface Settings {
  library_dir: string;
  auto_record: boolean;
  start_at: "warmup" | "bus";
  stop_at: "lobby" | "player_finished";
  post_roll_s: number;
  encoder: string;
  quality_cq: number;
  max_bitrate_mbps: number;
  fps: number;
  monitor: number | null;
  system_audio: boolean;
  microphone: boolean;
  mic_volume: number;
  auto_clips: boolean;
  clip_before_s: number;
  clip_after_s: number;
  clip_deaths: boolean;
  ffmpeg_path: string | null;
  share_port: number;
  launch_at_startup: boolean;
  start_minimized: boolean;
  close_to_tray: boolean;
  drive: DriveSettings;
  hotkeys: Hotkeys;
  clip_seconds: number;
  replay_buffer: boolean;
  overlay: boolean;
  overlay_moments: boolean;
  /** Días antes de que una partida no conservada se borre del todo, PC y Drive (0 = nunca). */
  retention_days: number;
  /** Días antes de borrar de la PC lo que ya está en Drive (0 = nunca). */
  local_days: number;
  detect_chests: boolean;
}

export interface Monitor {
  index: number;
  name: string;
  width: number;
  height: number;
  x: number;
  y: number;
}

export interface ShareLink {
  url: string;
  qr_svg: string;
}

export interface SetupCheck {
  ffmpeg: string | null;
  ffmpeg_path: string;
  nvenc: boolean;
  fortnite_log: boolean;
  replays_enabled: boolean | null;
}

export interface UploadProgress {
  key: string;
  sent: number;
  total: number;
  done: boolean;
  error?: string | null;
  download?: boolean;
  /** Subida en pausa porque estás jugando. */
  waiting?: boolean;
}

export const api = {
  status: () => invoke<Status>("get_status"),
  settings: () => invoke<Settings>("get_settings"),
  /** Devuelve los atajos que no se pudieron registrar. */
  saveSettings: (settings: Settings) => invoke<string[]>("save_settings", { settings }),
  matches: () => invoke<Match[]>("list_matches"),
  totals: () => invoke<Totals>("get_totals"),
  match: (id: string) => invoke<Match>("get_match", { id }),
  deleteMatch: (id: string) => invoke<void>("delete_match", { id }),
  updateMatch: (id: string, patch: { title?: string; keep?: boolean }) =>
    invoke<void>("update_match", { id, title: patch.title ?? null, keep: patch.keep ?? null }),
  deleteClip: (id: string, clipId: string) => invoke<void>("delete_clip", { id, clipId }),
  exportClip: (id: string, start: number, end: number, title: string) =>
    invoke<Clip>("export_clip", { id, start, end, title }),
  reprocess: (id: string) => invoke<void>("reprocess_match", { id }),
  startRecording: () => invoke<void>("start_recording"),
  stopRecording: () => invoke<void>("stop_recording"),
  monitors: () => invoke<Monitor[]>("list_monitors"),
  shareLan: (id: string, clipId?: string) => invoke<ShareLink>("share_lan", { id, clipId: clipId ?? null }),
  driveConnect: () => invoke<string>("drive_connect"),
  driveDisconnect: () => invoke<void>("drive_disconnect"),
  driveUpload: (id: string, clipId?: string) => invoke<DriveRef>("drive_upload", { id, clipId: clipId ?? null }),
  driveShare: (id: string, clipId?: string) => invoke<ShareLink>("drive_share", { id, clipId: clipId ?? null }),
  driveDownload: (id: string, clipId?: string) => invoke<void>("drive_download", { id, clipId: clipId ?? null }),
  driveOpen: (id: string, clipId?: string) => invoke<void>("drive_open", { id, clipId: clipId ?? null }),
  reveal: (id: string, clipId?: string) => invoke<void>("reveal", { id, clipId: clipId ?? null }),
  checkSetup: () => invoke<SetupCheck>("check_setup"),
  saveClipNow: () => invoke<void>("save_clip_now"),
  addBookmark: () => invoke<void>("add_bookmark"),
  toggleRecording: () => invoke<void>("toggle_recording"),
  overlayClose: () => invoke<void>("overlay_close"),
  overlayIdle: () => invoke<void>("overlay_idle"),
  openInMain: (id: string, clipId?: string) => invoke<void>("open_in_main", { id, clipId: clipId ?? null }),
};

export function fileUrl(m: Pick<Match, "dir">, file: string, bust?: string | number) {
  const url = convertFileSrc(`${m.dir}\\${file.replace(/\//g, "\\")}`);
  return bust === undefined ? url : `${url}?v=${bust}`;
}

export function fmtTime(s: number) {
  if (!isFinite(s) || s < 0) s = 0;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = Math.floor(s % 60);
  const mm = h ? String(m).padStart(2, "0") : String(m);
  return `${h ? h + ":" : ""}${mm}:${String(sec).padStart(2, "0")}`;
}

export function fmtDate(iso: string) {
  const d = new Date(iso);
  return d.toLocaleString("es", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

export function fmtSize(bytes: number) {
  if (bytes > 1e9) return `${(bytes / 1e9).toFixed(1).replace(".", ",")} GB`;
  return `${Math.round(bytes / 1e6)} MB`;
}

/** "04:05" / "1:02:03": reloj con ceros, para datos en monoespaciada. */
export function fmtClock(s: number) {
  if (!isFinite(s) || s < 0) s = 0;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = Math.floor(s % 60);
  return `${h ? h + ":" : ""}${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}`;
}

/** "48 min" / "3 h 20 min" */
export function fmtPlayed(s: number) {
  const min = Math.round(s / 60);
  if (min < 60) return `${min} min`;
  const h = Math.floor(min / 60);
  return min % 60 ? `${h} h ${min % 60} min` : `${h} h`;
}

export const pad2 = (n: number) => String(n).padStart(2, "0");

export const EVENT_COLORS: Record<string, string> = {
  kill: "var(--accent)",
  knock: "color-mix(in srgb, var(--accent) 55%, transparent)",
  death: "var(--flare)",
  knocked: "var(--flare)",
  victory: "var(--accent)",
  highlight: "var(--accent)",
  bus: "var(--text)",
  bookmark: "var(--text)",
  chest: "var(--text-2)",
  clip: "var(--accent)",
};

export const EVENT_KIND: Record<string, string> = {
  kill: "Eliminación",
  knock: "Derribo",
  death: "Eliminado",
  knocked: "Derribado",
  victory: "Victoria",
  highlight: "Momento",
  bus: "Autobús",
  bookmark: "Marca",
  chest: "Cofre",
  clip: "Clip",
};

/** Distancia del disparo más largo que terminó en eliminación, si el replay la trae. */
export function longestShot(m: Pick<Match, "events">) {
  const d = m.events.filter((e) => e.kind === "kill" && e.distance_m).map((e) => e.distance_m!);
  return d.length ? Math.max(...d) : null;
}

export function clipTag(kind: string) {
  return kind === "hotkey" ? "Atajo" : kind === "manual" ? "Editor" : "Auto";
}

/** Momento de la caída del jugador (para "Eliminado 14:03"). */
export function deathAt(m: Pick<Match, "events" | "died">) {
  if (!m.died) return null;
  const d = [...m.events].reverse().find((e) => e.kind === "death");
  return d ? d.t : null;
}

/** Espacio ocupado en esta PC (lo que está solo en Drive no cuenta). */
export function totalSize(matches: Match[]) {
  return matches.reduce((a, m) => a + (m.offloaded ? 0 : m.size_bytes || 0), 0);
}

/** Días que le quedan a una partida antes de vencer, o `null` si no vence. */
export function daysLeft(m: Pick<Match, "keep" | "created_at" | "status">, retentionDays: number | undefined) {
  if (!retentionDays || m.keep || m.status === "recording" || m.status === "processing") return null;
  const ms = new Date(m.created_at).getTime() + retentionDays * 86400e3 - Date.now();
  return Math.max(0, Math.ceil(ms / 86400e3));
}

export function expiryText(days: number) {
  return days <= 0 ? "Vence hoy" : days === 1 ? "Vence mañana" : `Vence en ${days} días`;
}

/** "Alt+F10" → "Alt + F10", "KeyG" → "G" */
export function prettyHotkey(h: string) {
  return h
    .split("+")
    .map((k) => k.replace(/^Key([A-Z])$/, "$1").replace(/^Digit(\d)$/, "$1"))
    .join(" + ");
}

export function errText(e: unknown) {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
