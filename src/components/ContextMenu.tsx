import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createPortal, flushSync } from "react-dom";
import { Icon } from "./Icon";

export type MenuItem = {
  label: string;
  icon?: string;
  onSelect: () => void;
  danger?: boolean;
  disabled?: boolean;
  /** Texto corto a la derecha (atajo, tiempo…). */
  hint?: string;
};
/** Los valores falsos se descartan, para poder armar listas con condiciones en línea. */
export type MenuEntry = MenuItem | "separator" | null | undefined | false;
type Entry = MenuItem | "separator";

/** Lo mínimo de un evento de ratón que hace falta para abrir el menú. */
type Trigger = {
  clientX: number;
  clientY: number;
  target: EventTarget | null;
  currentTarget: EventTarget | null;
  preventDefault(): void;
  stopPropagation(): void;
};

type Api = { open: (e: Trigger, items: MenuEntry[]) => void; close: () => void };

const Ctx = createContext<Api>({ open: () => {}, close: () => {} });
export const useContextMenu = () => useContext(Ctx);

/** Donde el menú nativo sigue haciendo falta (copiar y pegar). */
const NATIVE = 'input, textarea, [contenteditable]:not([contenteditable="false"])';
const FOCUSABLE = 'button, a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])';
const MARGIN = 8;

/** Quita el menú del WebView (Atrás, Recargar, Inspeccionar) salvo en campos de texto. */
export function blockNativeContextMenu() {
  window.addEventListener("contextmenu", (e) => {
    if (import.meta.env.DEV && e.shiftKey) return;
    if (e.target instanceof Element && e.target.closest(NATIVE)) return;
    e.preventDefault();
  });
}

function tidy(entries: MenuEntry[]): Entry[] {
  const out: Entry[] = [];
  for (const e of entries) {
    if (!e) continue;
    if (e === "separator" && (out.length === 0 || out[out.length - 1] === "separator")) continue;
    out.push(e);
  }
  while (out[out.length - 1] === "separator") out.pop();
  return out;
}

type State = { x: number; y: number; items: Entry[]; key: number };

export function ContextMenuProvider({ children }: { children: ReactNode }) {
  const [menu, setMenu] = useState<State | null>(null);
  const anchor = useRef<HTMLElement | null>(null);
  const box = useRef<HTMLDivElement>(null);

  const close = useCallback(() => setMenu(null), []);
  const restoreFocus = useCallback(() => {
    const a = anchor.current;
    if (a?.isConnected) a.focus({ preventScroll: true });
  }, []);
  const dismiss = useCallback(() => {
    flushSync(() => setMenu(null));
    restoreFocus();
  }, [restoreFocus]);

  const open = useCallback((e: Trigger, entries: MenuEntry[]) => {
    const target = e.target instanceof Element ? e.target : null;
    // Campos de texto: menú nativo. Diálogos abiertos: sin menú.
    if (target?.closest(`${NATIVE}, .modal-back`)) return;
    const items = tidy(entries);
    if (!items.length) return;
    e.preventDefault();
    e.stopPropagation();
    const host = e.currentTarget instanceof HTMLElement ? e.currentTarget : null;
    anchor.current = target?.closest<HTMLElement>(FOCUSABLE) ?? (document.activeElement as HTMLElement | null);
    let { clientX: x, clientY: y } = e;
    // Abierto con el teclado (tecla Menú o Mayús+F10): sin coordenadas útiles.
    if (x === 0 && y === 0 && host) {
      const r = host.getBoundingClientRect();
      x = r.left + 12;
      y = r.top + Math.min(r.height, 28);
    }
    setMenu({ x, y, items, key: Date.now() });
  }, []);

  useEffect(() => {
    if (!menu) return;
    const outside = (e: Event) => !(e.target instanceof Node && box.current?.contains(e.target));
    const onDown = (e: PointerEvent) => outside(e) && close();
    const onScroll = (e: Event) => outside(e) && close();
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      dismiss();
    };
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
    };
  }, [menu, close, dismiss]);

  const select = useCallback(
    (item: MenuItem) => {
      if (item.disabled) return;
      // Se cierra antes de actuar: confirm() y los diálogos no deben ver el menú detrás.
      dismiss();
      item.onSelect();
    },
    [dismiss],
  );

  const api = useMemo(() => ({ open, close }), [open, close]);

  return (
    <Ctx.Provider value={api}>
      {children}
      {menu && createPortal(<Menu key={menu.key} state={menu} box={box} onSelect={select} onDismiss={dismiss} />, document.body)}
    </Ctx.Provider>
  );
}

function Menu({
  state,
  box,
  onSelect,
  onDismiss,
}: {
  state: State;
  box: React.RefObject<HTMLDivElement | null>;
  onSelect: (item: MenuItem) => void;
  onDismiss: () => void;
}) {
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let left = state.x;
    let top = state.y;
    if (left + width > vw - MARGIN) left = state.x - width >= MARGIN ? state.x - width : Math.max(MARGIN, vw - MARGIN - width);
    if (top + height > vh - MARGIN) top = state.y - height >= MARGIN ? state.y - height : Math.max(MARGIN, vh - MARGIN - height);
    setPos({ left, top });
  }, [box, state.x, state.y]);

  const enabled = () => Array.from(box.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])') ?? []);

  useEffect(() => {
    if (pos) enabled()[0]?.focus({ preventScroll: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pos]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    // Los atajos del reproductor no deben ver las teclas del menú.
    e.stopPropagation();
    const list = enabled();
    if (!list.length) return;
    const i = list.indexOf(document.activeElement as HTMLElement);
    const go = (n: number) => {
      e.preventDefault();
      list[(n + list.length) % list.length].focus();
    };
    if (e.key === "ArrowDown") go(i + 1);
    else if (e.key === "ArrowUp") go(i < 0 ? list.length - 1 : i - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(list.length - 1);
    else if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      onDismiss();
    }
  };

  return (
    <div
      ref={box}
      className="ctx-menu"
      role="menu"
      aria-orientation="vertical"
      style={pos ? { left: pos.left, top: pos.top } : { left: state.x, top: state.y, visibility: "hidden" }}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      {state.items.map((it, i) =>
        it === "separator" ? (
          <div key={`s${i}`} className="ctx-sep" role="separator" />
        ) : (
          <button
            key={`${i}${it.label}`}
            type="button"
            role="menuitem"
            tabIndex={-1}
            className={`ctx-item ${it.danger ? "danger" : ""}`}
            aria-disabled={it.disabled || undefined}
            onClick={() => onSelect(it)}
            onPointerMove={(e) => !it.disabled && document.activeElement !== e.currentTarget && e.currentTarget.focus({ preventScroll: true })}
          >
            {it.icon ? <Icon name={it.icon} size={16} /> : <span className="ctx-gap" />}
            <span className="ctx-label">{it.label}</span>
            {it.hint && <span className="ctx-hint">{it.hint}</span>}
          </button>
        ),
      )}
    </div>
  );
}
