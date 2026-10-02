import { useEffect, useRef, useState } from "react";
import { duration, type InputItem } from "@cia/engine-client";
import { useHost } from "../host/HostContext";

/** Start/end handles on a timeline with a preview frame for the handle being dragged (DESIGN.md 4.4). */
export default function TrimPanel({ item, value, onChange, onClose }: { item: InputItem; value: [number, number] | null; onChange: (range: [number, number] | null) => void; onClose: () => void }) {
  const { host } = useHost();
  const total = Number(item.detail.duration_ms ?? 0);
  const [range, setRange] = useState<[number, number]>(value ?? [0, total]);
  const [preview, setPreview] = useState<string | null>(null);
  const [dragging, setDragging] = useState<"start" | "end" | null>(null);
  const track = useRef<HTMLDivElement>(null);
  const debounce = useRef<number | null>(null);

  useEffect(() => { setRange(value ?? [0, total]); }, [value, total]);

  const requestFrame = (ms: number) => {
    if (!host.actions.previewFrame) return;
    if (debounce.current) window.clearTimeout(debounce.current);
    debounce.current = window.setTimeout(async () => { setPreview(await host.actions.previewFrame!(item.id, ms)); }, 150);
  };

  const pxToMs = (clientX: number) => {
    const r = track.current!.getBoundingClientRect();
    return Math.round(Math.max(0, Math.min(1, (clientX - r.left) / r.width)) * total);
  };
  useEffect(() => {
    if (!dragging) return;
    const move = (e: PointerEvent) => {
      const ms = pxToMs(e.clientX);
      setRange(([s, en]) => (dragging === "start" ? [Math.min(ms, en - 1000), en] : [s, Math.max(ms, s + 1000)]));
      requestFrame(ms);
    };
    const up = () => { setDragging(null); };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); };
  });
  useEffect(() => { if (!dragging) onChange(range[0] === 0 && range[1] === total ? null : range); }, [dragging]); // eslint-disable-line react-hooks/exhaustive-deps

  const pct = (ms: number) => `${(ms / Math.max(1, total)) * 100}%`;
  return (
    <div className="smg-fade-in ml-8 flex flex-col gap-3 rounded-card border border-subtle bg-surface-sunken p-3" data-testid="trim-panel">
      <div className="flex items-center gap-4">
        {item.kind === "video" && <div className="flex h-20 w-32 shrink-0 items-center justify-center overflow-hidden rounded-md bg-surface-base text-xs text-on-surface-subtle">{preview ? <img src={preview} alt="" className="h-full w-full object-contain" /> : "Preview"}</div>}
        <div className="flex-1">
          <div ref={track} className="relative h-8 cursor-pointer rounded-md bg-surface-card" onPointerDown={(e) => { const ms = pxToMs(e.clientX); const nearStart = Math.abs(ms - range[0]) < Math.abs(ms - range[1]); setDragging(nearStart ? "start" : "end"); }}>
            <div className="absolute bottom-0 top-0 rounded-md bg-primary-container" style={{ left: pct(range[0]), right: `calc(100% - ${pct(range[1])})` }} />
            <div role="slider" aria-label="Start" aria-valuenow={range[0]} tabIndex={0} className="absolute top-0 h-8 w-3 -translate-x-1/2 cursor-ew-resize rounded bg-primary" style={{ left: pct(range[0]) }} onPointerDown={(e) => { e.stopPropagation(); setDragging("start"); }} onKeyDown={(e) => { if (e.key === "ArrowLeft") setRange(([s, en]) => [Math.max(0, s - 1000), en]); if (e.key === "ArrowRight") setRange(([s, en]) => [Math.min(en - 1000, s + 1000), en]); }} />
            <div role="slider" aria-label="End" aria-valuenow={range[1]} tabIndex={0} className="absolute top-0 h-8 w-3 -translate-x-1/2 cursor-ew-resize rounded bg-primary" style={{ left: pct(range[1]) }} onPointerDown={(e) => { e.stopPropagation(); setDragging("end"); }} onKeyDown={(e) => { if (e.key === "ArrowLeft") setRange(([s, en]) => [s, Math.max(s + 1000, en - 1000)]); if (e.key === "ArrowRight") setRange(([s, en]) => [s, Math.min(total, en + 1000)]); }} />
          </div>
          <div className="mt-1 flex justify-between text-xs text-on-surface-muted"><span>{duration(range[0])}</span><span>Selected: {duration(range[1] - range[0])}</span><span>{duration(range[1])}</span></div>
        </div>
      </div>
      <div className="flex gap-2">
        <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => { setRange([0, total]); onChange(null); }}>Reset</button>
        <button className="smg-btn smg-btn--ghost smg-btn--sm" onClick={onClose}>Done</button>
      </div>
    </div>
  );
}
