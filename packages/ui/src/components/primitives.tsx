import { clsx } from "clsx";
import { X } from "lucide-react";
import { useEffect, type ReactNode } from "react";
import { createPortal } from "react-dom";

export function Modal({ title, onClose, children, width = 520, testId }: { title?: string; onClose: () => void; children: ReactNode; width?: number; testId?: string }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); onClose(); } };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-label={title} data-testid={testId}>
      <div className="absolute inset-0 bg-scrim" onClick={onClose} />
      <div className="smg-fade-in relative max-h-[90vh] w-full overflow-auto rounded-panel border border-default bg-surface-overlay p-6 shadow-3" style={{ maxWidth: width }}>
        {title && (
          <div className="mb-4 flex items-start justify-between gap-4">
            <h2 className="font-display text-type-2">{title}</h2>
            <button className="smg-btn smg-btn--ghost smg-btn--sm !px-2" onClick={onClose} aria-label="Close"><X size={16} /></button>
          </div>
        )}
        {children}
      </div>
    </div>,
    document.body,
  );
}

export function Switch({ checked, onChange, label, description, disabled }: { checked: boolean; onChange: (v: boolean) => void; label: string; description?: string; disabled?: boolean }) {
  return (
    <label className={clsx("flex cursor-pointer items-start justify-between gap-4 py-2", disabled && "opacity-50")}>
      <span>
        <span className="block font-medium">{label}</span>
        {description && <span className="block text-xs text-on-surface-muted">{description}</span>}
      </span>
      <button type="button" role="switch" aria-checked={checked} disabled={disabled} onClick={() => onChange(!checked)}
        className={clsx("relative mt-0.5 inline-flex h-6 w-11 shrink-0 rounded-pill border-2 border-transparent transition-colors duration-fast", checked ? "bg-primary" : "bg-border-strong")}>
        <span className={clsx("inline-block h-5 w-5 rounded-full bg-white shadow-subtle transition-transform duration-fast", checked ? "translate-x-5" : "translate-x-0")} />
      </button>
    </label>
  );
}

export function Select<T extends string>({ value, onChange, options, label, description }: { value: T; onChange: (v: T) => void; options: { value: T; label: string }[]; label: string; description?: string }) {
  return (
    <label className="flex items-start justify-between gap-4 py-2">
      <span>
        <span className="block font-medium">{label}</span>
        {description && <span className="block text-xs text-on-surface-muted">{description}</span>}
      </span>
      <select className="smg-input h-9 min-w-[170px]" value={value} onChange={(e) => onChange(e.target.value as T)}>
        {options.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
      </select>
    </label>
  );
}

export function Spinner({ size = 16 }: { size?: number }) {
  return <span className="smg-spin inline-block rounded-full border-2 border-current border-t-transparent" style={{ width: size, height: size }} aria-hidden />;
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="rounded border border-default bg-surface-sunken px-1.5 py-0.5 font-sans text-[11px] text-on-surface-muted">{children}</kbd>;
}
