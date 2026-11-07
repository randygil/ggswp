import { useCallback, useState, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import { api, errText, fmtClock, type Clip, type Match } from "../api";
import { useContextMenu, type MenuEntry } from "./ContextMenu";
import { RenameDialog } from "./RenameDialog";
import { ShareDialog } from "./ShareDialog";
import { useToast } from "./Toast";

/** Ejecuta una acción y avisa con un toast (el error, siempre; el éxito, si hay texto). */
export type Run = (label: string, fn: () => Promise<unknown>) => Promise<unknown>;

export function useRun(): Run {
  const toast = useToast();
  return useCallback(
    async (label, fn) => {
      try {
        await fn();
        if (label) toast.info(label);
      } catch (e) {
        toast.error(errText(e));
      }
    },
    [toast],
  );
}

const copy = (text: string) => navigator.clipboard.writeText(text);

export function matchMenuItems(
  m: Match,
  a: { open: () => void; share: () => void; rename: () => void; run: Run; busy?: boolean },
): MenuEntry[] {
  const ready = m.status === "ready";
  const inDrive = !!m.drive || m.clips.some((c) => c.drive);
  return [
    { label: "Abrir", icon: "play", onSelect: a.open, disabled: m.status === "recording" },
    {
      label: m.keep ? "Dejar de conservar" : "Conservar",
      icon: "keep",
      onSelect: () => a.run(m.keep ? "Ya no está conservada" : "Partida conservada: no vencerá", () => api.updateMatch(m.id, { keep: !m.keep })),
    },
    { label: "Renombrar…", icon: "edit", onSelect: a.rename },
    { label: "Compartir…", icon: "upload", onSelect: a.share, disabled: !ready },
    m.drive
      ? { label: "Abrir en Drive", icon: "cloud", onSelect: () => a.run("", () => api.driveOpen(m.id)) }
      : { label: "Subir a Drive", icon: "cloud", disabled: !ready || a.busy, onSelect: () => a.run("Subida a Drive completa", () => api.driveUpload(m.id)) },
    m.offloaded && {
      label: "Descargar a la PC",
      icon: "download",
      disabled: a.busy,
      onSelect: () => a.run("Descargado de nuevo a la PC", () => api.driveDownload(m.id)),
    },
    { label: "Mostrar en carpeta", icon: "folder", disabled: m.offloaded, onSelect: () => a.run("", () => api.reveal(m.id)) },
    {
      label: "Volver a analizar",
      icon: "refresh",
      disabled: m.offloaded || !ready,
      onSelect: () => a.run("Partida analizada de nuevo", () => api.reprocess(m.id)),
    },
    "separator",
    {
      label: "Borrar",
      icon: "trash",
      danger: true,
      onSelect: () => {
        const msg = inDrive
          ? "¿Borrar esta partida y sus clips? También se mandan a la papelera de Google Drive."
          : "¿Borrar esta partida y todos sus clips del disco?";
        if (confirm(msg)) a.run("Partida borrada", () => api.deleteMatch(m.id));
      },
    },
  ];
}

export function clipMenuItems(m: Match, c: Clip, a: { play: () => void; share: () => void; run: Run; busy?: boolean }): MenuEntry[] {
  return [
    { label: "Reproducir", icon: "play", onSelect: a.play },
    { label: "Compartir…", icon: "upload", onSelect: a.share },
    c.drive
      ? { label: "Abrir en Drive", icon: "cloud", onSelect: () => a.run("", () => api.driveOpen(m.id, c.id)) }
      : { label: "Subir a Drive", icon: "cloud", disabled: a.busy, onSelect: () => a.run("Clip subido a Drive", () => api.driveUpload(m.id, c.id)) },
    c.offloaded && {
      label: "Descargar a la PC",
      icon: "download",
      disabled: a.busy,
      onSelect: () => a.run("Descargado de nuevo a la PC", () => api.driveDownload(m.id, c.id)),
    },
    { label: "Mostrar en carpeta", icon: "folder", disabled: !!c.offloaded, onSelect: () => a.run("", () => api.reveal(m.id, c.id)) },
    c.drive && { label: "Copiar enlace de Drive", icon: "link", onSelect: () => a.run("Enlace copiado", () => copy(c.drive!.link)) },
    "separator",
    {
      label: "Borrar",
      icon: "trash",
      danger: true,
      onSelect: () => {
        const msg = c.drive ? `¿Borrar el clip "${c.title}"? También se manda a la papelera de Drive.` : `¿Borrar el clip "${c.title}"?`;
        if (confirm(msg)) a.run("Clip borrado", () => api.deleteClip(m.id, c.id));
      },
    },
  ];
}

/** Menú de un evento de la partida. Sin `makeClip`, "Crear clip aquí" queda desactivado. */
export function eventMenuItems(t: number, a: { seek: () => void; makeClip?: () => void; run: Run }): MenuEntry[] {
  return [
    { label: "Ir al momento", icon: "play", hint: fmtClock(t), onSelect: a.seek },
    { label: "Crear clip aquí", icon: "scissors", disabled: !a.makeClip, onSelect: () => a.makeClip?.() },
    { label: "Copiar tiempo", icon: "copy", onSelect: () => a.run("Tiempo copiado", () => copy(fmtClock(t))) },
  ];
}

/** Tramo de clip alrededor de un evento: 10 s antes y 4 s después. El reproductor lo ajusta al video. */
export function selectAround(t: number) {
  window.dispatchEvent(new CustomEvent("ggswp-select", { detail: { start: t - 10, end: t + 4 } }));
}

/**
 * Los diálogos se montan en <body> con un portal; el envoltorio corta la burbuja de clics
 * del árbol de React para que no lleguen a la tarjeta (un <button>) que los abrió.
 */
const portal = (node: React.ReactNode) =>
  createPortal(
    <div style={{ display: "contents" }} onClick={(e) => e.stopPropagation()}>
      {node}
    </div>,
    document.body,
  );

export function useMatchMenu(m: Match, onOpen: () => void) {
  const menu = useContextMenu();
  const run = useRun();
  const [dialog, setDialog] = useState<"share" | "rename" | null>(null);
  const close = () => setDialog(null);
  const onContextMenu = (e: MouseEvent) =>
    menu.open(e, matchMenuItems(m, { open: onOpen, share: () => setDialog("share"), rename: () => setDialog("rename"), run }));
  const dialogs =
    dialog === "share"
      ? portal(<ShareDialog match={m} onClose={close} />)
      : dialog === "rename"
        ? portal(
            <RenameDialog
              heading="Renombrar partida"
              label="Nombre de la partida"
              value={m.title}
              onSave={(title) => api.updateMatch(m.id, { title })}
              onClose={close}
            />,
          )
        : null;
  return { onContextMenu, dialogs };
}

export function useClipMenu(m: Match, c: Clip, onOpen: () => void) {
  const menu = useContextMenu();
  const run = useRun();
  const [share, setShare] = useState(false);
  const onContextMenu = (e: MouseEvent) => menu.open(e, clipMenuItems(m, c, { play: onOpen, share: () => setShare(true), run }));
  const dialogs = share ? portal(<ShareDialog match={m} clip={c} onClose={() => setShare(false)} />) : null;
  return { onContextMenu, dialogs };
}
