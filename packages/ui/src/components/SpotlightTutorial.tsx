import { useEffect, useLayoutEffect, useState } from "react";
import { createPortal } from "react-dom";
import { measuredZoomRatio } from "../lib/zoomRatio";

export interface SpotlightStep {
  id: string;
  /** CSS selector to highlight; omit for a centred card. */
  target?: string;
  title: string;
  body: string;
}

interface Props {
  steps: SpotlightStep[];
  stepIndex: number;
  onStepIndexChange: (index: number) => void;
  /** completed=false means the user skipped. */
  onFinish: (completed: boolean) => void;
}

interface Rect { top: number; left: number; width: number; height: number }

const HOLE_PADDING = 8;
const CARD_WIDTH = 330;
const CARD_GAP = 14;

/**
 * Ported from ConvertSave's SpotlightTutorial (class prefix smg-). Dims the app
 * and cuts a spotlight around the current step's target, tracked per frame.
 */
export default function SpotlightTutorial({ steps, stepIndex, onStepIndexChange, onFinish }: Props) {
  const step = steps[stepIndex];
  const [rect, setRect] = useState<Rect | null>(null);

  useLayoutEffect(() => {
    let raf = 0;
    const measure = () => {
      raf = requestAnimationFrame(measure);
      let next: Rect | null = null;
      if (step?.target) {
        const el = document.querySelector(step.target);
        if (el) {
          const zoom = measuredZoomRatio();
          const r = el.getBoundingClientRect();
          if (r.width > 0 && r.height > 0) {
            next = { top: r.top / zoom - HOLE_PADDING, left: r.left / zoom - HOLE_PADDING, width: r.width / zoom + HOLE_PADDING * 2, height: r.height / zoom + HOLE_PADDING * 2 };
          }
        }
      }
      setRect((prev) => {
        if (prev === next) return prev;
        if (prev && next && Math.abs(prev.top - next.top) < 0.5 && Math.abs(prev.left - next.left) < 0.5 && Math.abs(prev.width - next.width) < 0.5 && Math.abs(prev.height - next.height) < 0.5) return prev;
        return next;
      });
    };
    raf = requestAnimationFrame(measure);
    return () => cancelAnimationFrame(raf);
  }, [step]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.stopPropagation(); onFinish(false); }
      if (event.key === "ArrowRight" || event.key === "Enter") {
        event.stopPropagation();
        event.preventDefault();
        if (stepIndex < steps.length - 1) onStepIndexChange(stepIndex + 1);
        else onFinish(true);
      }
      if (event.key === "ArrowLeft" && stepIndex > 0) { event.stopPropagation(); onStepIndexChange(stepIndex - 1); }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [stepIndex, steps.length, onStepIndexChange, onFinish]);

  if (!step) return null;
  const isLast = stepIndex === steps.length - 1;

  let cardStyle: React.CSSProperties;
  if (rect) {
    const vh = document.documentElement.clientHeight;
    const vw = document.documentElement.clientWidth;
    const isTall = rect.height > vh * 0.55;
    if (isTall) {
      const fitsLeft = rect.left - CARD_WIDTH - CARD_GAP > 12;
      const left = fitsLeft ? rect.left - CARD_WIDTH - CARD_GAP : Math.min(rect.left + rect.width + CARD_GAP, vw - CARD_WIDTH - 12);
      cardStyle = { top: Math.max(12, Math.min(rect.top + rect.height / 2 - 110, vh - 240)), left };
    } else {
      const below = rect.top + rect.height + CARD_GAP;
      const fitsBelow = below + 210 < vh;
      const left = Math.max(12, Math.min(rect.left + rect.width / 2 - CARD_WIDTH / 2, vw - CARD_WIDTH - 12));
      cardStyle = fitsBelow ? { top: below, left } : { top: Math.max(12, rect.top - CARD_GAP - 210), left };
    }
  } else {
    cardStyle = { top: "50%", left: "50%", transform: "translate(-50%, -50%)" };
  }

  return createPortal(
    <div className="smg-tour-root" role="dialog" aria-modal="true" aria-label="Smidge tour" data-testid="tour">
      <div className="absolute inset-0" />
      {rect ? <div className="smg-tour-hole" style={{ top: rect.top, left: rect.left, width: rect.width, height: rect.height }} aria-hidden /> : <div className="smg-tour-dim" aria-hidden />}
      <div className="smg-tour-card" style={{ ...cardStyle, width: CARD_WIDTH }}>
        <span className="smg-label">{stepIndex + 1} / {steps.length}</span>
        <h2 className="text-[15px] font-semibold leading-snug font-display">{step.title}</h2>
        <p className="m-0 text-[13px] leading-relaxed text-on-surface-muted">{step.body}</p>
        <div className="mt-1 flex items-center gap-2">
          <button className="smg-btn smg-btn--ghost smg-btn--sm !px-2" onClick={() => onFinish(false)}>Skip</button>
          <span className="flex-1" />
          {stepIndex > 0 && <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => onStepIndexChange(stepIndex - 1)}>Back</button>}
          <button className="smg-btn smg-btn--primary smg-btn--sm" onClick={() => (isLast ? onFinish(true) : onStepIndexChange(stepIndex + 1))} autoFocus>
            {stepIndex === 0 ? "Show me around" : isLast ? "Finish" : "Next"}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
