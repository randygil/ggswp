import { useState } from "react";
import { prettyHotkey } from "../api";
import { t } from "../i18n";

const MODIFIER_CODES = new Set(["ControlLeft", "ControlRight", "AltLeft", "AltRight", "ShiftLeft", "ShiftRight", "MetaLeft", "MetaRight"]);

/** Convierte un evento de teclado al formato del plugin de atajos ("Alt+F10", "Ctrl+Shift+KeyS"). */
function toCombo(e: React.KeyboardEvent): string | null {
  if (MODIFIER_CODES.has(e.code)) return null;
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean) as string[];
  const isFn = /^F\d{1,2}$/.test(e.code);
  // Sin modificador solo se permiten teclas de función, para no robarle teclas al juego.
  if (!mods.length && !isFn) return null;
  return [...mods, e.code].join("+");
}

export function HotkeyInput({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [listening, setListening] = useState(false);
  return (
    <span className="row-control">
      <button
        type="button"
        className={`input hotkey ${listening ? "listening" : ""}`}
        onClick={() => setListening(true)}
        onBlur={() => setListening(false)}
        onKeyDown={(e) => {
          if (!listening) return;
          e.preventDefault();
          if (e.key === "Escape") return setListening(false);
          const combo = toCombo(e);
          if (combo) {
            onChange(combo);
            setListening(false);
          }
        }}
      >
        {listening ? t("Presiona la combinación…") : value ? prettyHotkey(value) : t("Sin asignar")}
      </button>
      {value && (
        <button type="button" className="btn ghost sm" onClick={() => onChange("")}>
          {t("Quitar")}
        </button>
      )}
    </span>
  );
}
