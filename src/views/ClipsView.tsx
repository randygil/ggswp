import { useMemo, useState } from "react";
import { clipTag, pad2, type Match } from "../api";
import { Icon } from "../components/Icon";
import { t } from "../i18n";
import { ClipCard } from "./Gallery";

type Filter = "all" | "Auto" | "Atajo" | "Editor" | "Resumen" | "Vertical";

export function ClipsView({ matches, onOpen }: { matches: Match[]; onOpen: (id: string, clipId: string) => void }) {
  const [filter, setFilter] = useState<Filter>("all");
  const clips = useMemo(
    () =>
      matches
        .flatMap((m) => m.clips.map((c) => ({ m, c })))
        .sort((a, b) => new Date(b.c.created_at).getTime() - new Date(a.c.created_at).getTime()),
    [matches],
  );
  // `clipTag` devuelve la etiqueta ya traducida: se compara contra el filtro traducido.
  const shown = filter === "all" ? clips : clips.filter(({ c }) => clipTag(c.kind) === t(filter));
  const count = (f: Filter) => (f === "all" ? clips.length : clips.filter(({ c }) => clipTag(c.kind) === t(f)).length);
  const chips: [Filter, string][] = [
    ["all", t("Todos")],
    ["Auto", t("Automáticos")],
    ["Atajo", t("Con atajo")],
    ["Editor", t("Del editor")],
    ["Resumen", t("Resúmenes")],
    ["Vertical", t("Verticales")],
  ];

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1>Clips</h1>
          <p>{t("Recortes automáticos de tus eliminaciones, los que guardas con el atajo y los que exportas desde el editor.")}</p>
        </div>
      </header>
      {clips.length === 0 ? (
        <div className="empty">
          <Icon name="scissors" size={36} />
          <h2>{t("Sin clips todavía")}</h2>
          <p>{t("Se generan solos al terminar cada partida en la que eliminas a alguien.")}</p>
        </div>
      ) : (
        <section className="section">
          <div className="chips" role="group" aria-label={t("Filtrar clips")}>
            {chips.map(([k, label]) => (
              <button key={k} className={`chip ${filter === k ? "on" : ""}`} aria-pressed={filter === k} onClick={() => setFilter(k)}>
                {label} <span>{pad2(count(k))}</span>
              </button>
            ))}
          </div>
          <div className="grid">
            {shown.map(({ m, c }) => (
              <ClipCard key={`${m.id}/${c.id}`} m={m} c={c} onOpen={() => onOpen(m.id, c.id)} />
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
