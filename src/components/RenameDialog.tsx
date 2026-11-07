import { useEffect, useId, useState, type FormEvent } from "react";
import { errText } from "../api";
import { Icon } from "./Icon";

export function RenameDialog({
  heading,
  label,
  value,
  onSave,
  onClose,
}: {
  heading: string;
  label: string;
  value: string;
  onSave: (text: string) => Promise<unknown>;
  onClose: () => void;
}) {
  const id = useId();
  const [text, setText] = useState(value);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Al cerrar, el foco vuelve a donde estaba (la tarjeta, normalmente).
  const [prev] = useState(() => document.activeElement as HTMLElement | null);
  useEffect(() => () => void (prev?.isConnected && prev.focus({ preventScroll: true })), [prev]);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const t = text.trim();
    if (!t || t === value) return onClose();
    setBusy(true);
    setError(null);
    try {
      await onSave(t);
      onClose();
    } catch (err) {
      setError(errText(err));
      setBusy(false);
    }
  };

  return (
    <div className="modal-back" onClick={onClose}>
      <form
        className="modal modal-sm"
        role="dialog"
        aria-modal="true"
        aria-labelledby={id}
        onClick={(e) => e.stopPropagation()}
        onSubmit={submit}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Escape") onClose();
        }}
      >
        <header>
          <h2 id={id}>{heading}</h2>
          <button type="button" className="icon-btn" onClick={onClose} aria-label="Cerrar">
            <Icon name="x" />
          </button>
        </header>
        <input
          className="input"
          autoFocus
          aria-label={label}
          value={text}
          maxLength={120}
          onChange={(e) => setText(e.target.value)}
          onFocus={(e) => e.target.select()}
        />
        {error && <p className="error-text">{error}</p>}
        <div className="modal-actions">
          <button type="button" className="btn ghost sm" onClick={onClose}>
            Cancelar
          </button>
          <button type="submit" className="btn primary sm" disabled={busy}>
            {busy ? "Guardando…" : "Guardar"}
          </button>
        </div>
      </form>
    </div>
  );
}
