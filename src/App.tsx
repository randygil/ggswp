import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errText, type Match, type Settings, type Status, type Totals, type UploadProgress } from "./api";
import { Gallery } from "./views/Gallery";
import { ClipsView } from "./views/ClipsView";
import { MatchView } from "./views/MatchView";
import { SettingsView } from "./views/Settings";
import { StatsView } from "./views/StatsView";
import { Sidebar } from "./components/Sidebar";
import { UpdateBanner } from "./components/UpdateBanner";
import { ToastProvider, useToast } from "./components/Toast";
import { ContextMenuProvider, useContextMenu } from "./components/ContextMenu";
import { t, tr } from "./i18n";

export type Route = { name: "gallery" } | { name: "clips" } | { name: "stats" } | { name: "settings" } | { name: "match"; id: string; clipId?: string };

function Shell() {
  const [route, setRoute] = useState<Route>({ name: "gallery" });
  const [status, setStatus] = useState<Status | null>(null);
  const [matches, setMatches] = useState<Match[]>([]);
  const [uploads, setUploads] = useState<Record<string, UploadProgress>>({});
  const [settings, setSettings] = useState<Settings | null>(null);
  const [totals, setTotals] = useState<Totals | null>(null);
  const toast = useToast();
  const { close: closeMenu } = useContextMenu();

  useEffect(closeMenu, [route, closeMenu]);

  const reload = useCallback(() => {
    api.matches().then(setMatches).catch((e) => toast.error(errText(e)));
    api.totals().then(setTotals).catch(() => {});
  }, [toast]);

  // Los ajustes alimentan la barra lateral (atajos, búfer, Drive): se releen al salir de Ajustes.
  useEffect(() => {
    if (route.name !== "settings") api.settings().then(setSettings).catch(() => {});
  }, [route.name]);

  const counts = useMemo(() => ({ matches: matches.length, clips: matches.reduce((a, m) => a + m.clips.length, 0) }), [matches]);

  useEffect(() => {
    api.status().then(setStatus);
    reload();
    const subs = [
      listen<Status>("status", (e) => setStatus(e.payload)),
      listen("library", () => reload()),
      listen<{ id: string; clip_id?: string | null }>("navigate", (e) =>
        setRoute({ name: "match", id: e.payload.id, clipId: e.payload.clip_id ?? undefined }),
      ),
      listen<{ text: string; kind: string }>("toast", (e) =>
        e.payload.kind === "warn" ? toast.error(tr(e.payload.text)) : toast.info(tr(e.payload.text)),
      ),
      listen<UploadProgress>("upload", (e) => {
        const p = e.payload;
        if (p.done) {
          if (p.error) toast.error(t("Subida fallida: {error}", { error: tr(p.error) }));
          setUploads((u) => {
            const { [p.key]: _, ...rest } = u;
            return rest;
          });
        } else {
          setUploads((u) => ({ ...u, [p.key]: p }));
        }
      }),
    ];
    return () => subs.forEach((s) => s.then((un) => un()));
  }, [reload, toast]);

  useEffect(() => {
    if (status?.last_error) toast.error(tr(status.last_error));
  }, [status?.last_error, toast]);

  return (
    <div className="app">
      <UpdateBanner busy={!!status?.recording || (status?.processing.length ?? 0) > 0} />
      <Sidebar route={route} onNavigate={setRoute} status={status} settings={settings} uploads={uploads} counts={counts} />
      <main className="content">
        {route.name === "gallery" && (
          <Gallery
            matches={matches}
            status={status}
            onOpen={(id) => setRoute({ name: "match", id })}
            onOpenClip={(id, clipId) => setRoute({ name: "match", id, clipId })}
            onAllClips={() => setRoute({ name: "clips" })}
            retentionDays={settings?.retention_days}
            totals={totals}
          />
        )}
        {route.name === "clips" && (
          <ClipsView matches={matches} onOpen={(id, clipId) => setRoute({ name: "match", id, clipId })} />
        )}
        {route.name === "match" && (
          <MatchView
            key={route.id}
            id={route.id}
            initialClip={route.clipId}
            matches={matches}
            uploads={uploads}
            retentionDays={settings?.retention_days}
            localDays={settings?.drive.delete_local_after_upload ? undefined : settings?.local_days}
            clipQuality={settings?.clip_quality}
            step={status?.steps?.[route.id]}
            onBack={() => setRoute({ name: "gallery" })}
          />
        )}
        {route.name === "stats" && <StatsView matches={matches} totals={totals} onOpen={(id) => setRoute({ name: "match", id })} />}
        {route.name === "settings" && <SettingsView />}
      </main>
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <ContextMenuProvider>
        <Shell />
      </ContextMenuProvider>
    </ToastProvider>
  );
}
