import type { Step } from "../api";

/** Paso en curso del procesamiento: texto + barra si el paso se puede medir. */
export function StepProgress({ step, fallback }: { step?: Step | null; fallback: string }) {
  const pct = step?.pct ?? null;
  return (
    <span className="step" role="status" aria-live="polite">
      <span className="step-text">
        <span className="spinner" aria-hidden />
        {step?.label ?? fallback}
        {pct !== null && <b>{Math.round(pct)} %</b>}
      </span>
      {pct !== null && (
        <span className="step-bar" aria-hidden>
          <i style={{ width: `${pct}%` }} />
        </span>
      )}
    </span>
  );
}
