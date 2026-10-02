import { AlertTriangle, Check, ChevronRight, Copy, Download, ExternalLink, FolderOpen, GripVertical, Play, RotateCcw, Save, XCircle } from "lucide-react";
import { clsx } from "clsx";
import { useEffect, useState } from "react";
import { duration, mb, presetById, type Artifact, type ItemOutcome, type Refusal, type Suggestion } from "@cia/engine-client";
import { useHost } from "../host/HostContext";
import { useStore } from "../state/store";
import { Spinner } from "./primitives";

function suggestionLabel(s: Suggestion): string {
  switch (s.type) {
    case "trim": return `Trim to under ${duration(s.max_duration_ms)}`;
    case "pick_preset": return `Use ${presetById(s.preset_id)?.tier_label ? `${presetById(s.preset_id)!.tile_label} ${presetById(s.preset_id)!.tier_label}` : presetById(s.preset_id)?.tile_label ?? s.preset_id} (about ${mb(s.predicted_bytes)})`;
    case "split_into_messages": return `Split into ${s.groups.length} messages`;
    case "remove_files": return s.item_ids.length === 1 ? "Leave out the biggest file" : `Leave out the ${s.item_ids.length} biggest files`;
    case "install_ffmpeg": return "Set up video support";
    case "use_desktop_app": return "Use the desktop app";
    case "use_other_browser": return `Try ${s.browser}`;
  }
}

export function RefusalCard({ refusal, onSuggestion, title }: { refusal: Refusal; onSuggestion: (s: Suggestion) => void; title?: string }) {
  return (
    <div className="smg-fade-in rounded-card border border-danger-border bg-danger-container p-4 text-danger-container-foreground" data-testid="refusal-card" role="alert">
      <div className="flex items-start gap-3">
        <XCircle size={20} className="mt-0.5 shrink-0 text-danger" />
        <div className="flex-1">
          {title && <div className="font-display text-type-2">{title}</div>}
          <p className="m-0">{refusal.message}</p>
          {refusal.suggestions.length > 0 && (
            <div className="mt-3 flex flex-wrap gap-2">
              {refusal.suggestions.map((s, i) => <button key={i} className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => onSuggestion(s)}>{suggestionLabel(s)}</button>)}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ArtifactActions({ artifacts, single }: { artifacts: Artifact[]; single: boolean }) {
  const { host, caps } = useHost();
  const { dispatch } = useStore();
  const ids = artifacts.map((a) => a.id);
  const toast = (m: string) => { dispatch({ type: "toast", message: m }); window.setTimeout(() => dispatch({ type: "toast", message: null }), 2500); };
  const desktop = host.kind === "desktop";
  const video = artifacts[0]?.format === "mp4" || artifacts[0]?.format === "webm";
  return (
    <div className="mt-4 flex flex-wrap items-center gap-2" data-testid="result-actions">
      {desktop && caps?.can_copy_files && <button className="smg-btn smg-btn--primary" onClick={async () => { await host.actions.copyFilesToClipboard?.(ids); toast(single ? "Copied. Paste it into Discord, WhatsApp or an email." : "Copied. Paste them into Discord, WhatsApp or an email."); }} data-testid="copy-file"><Copy size={16} /> {single ? "Copy file" : "Copy files"}</button>}
      {desktop && caps?.can_reveal && <button className="smg-btn smg-btn--secondary" onClick={() => host.actions.revealInFolder?.(ids[0]!)}><FolderOpen size={16} /> {single ? "Show in folder" : "Show folder"}</button>}
      {desktop && single && <button className="smg-btn smg-btn--secondary" onClick={() => host.actions.openFile?.(ids[0]!)}>{video ? <Play size={16} /> : <ExternalLink size={16} />} {video ? "Play" : "Open"}</button>}
      {!desktop && <button className="smg-btn smg-btn--primary" onClick={() => host.actions.download?.(ids)} data-testid="download"><Download size={16} /> {single ? "Download" : "Download all"}</button>}
      {!desktop && host.actions.saveToFolder && <button className="smg-btn smg-btn--secondary" onClick={() => host.actions.saveToFolder?.(ids)}><Save size={16} /> Save into a folder</button>}
      {desktop && caps?.can_drag_out && (
        <span className="ml-auto inline-flex cursor-grab items-center gap-1 text-xs text-on-surface-muted" draggable onDragStart={(e) => { e.preventDefault(); void host.actions.startDragOut?.(ids); }} title="Drag into another app">
          <GripVertical size={14} /> Drag into Discord
        </span>
      )}
    </div>
  );
}

export default function StepThree({ onCompress, onCancel, onSuggestion, canCompress, blockedReason }: { onCompress: () => void; onCancel: () => void; onSuggestion: (s: Suggestion) => void; canCompress: boolean; blockedReason: string | null }) {
  const { state, dispatch } = useStore();
  const { phase, plan, summary, progress, items, destination, jobStartedAt } = state;
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (phase !== "running" || !jobStartedAt) return;
    const t = window.setInterval(() => setElapsed(Date.now() - jobStartedAt), 500);
    return () => window.clearInterval(t);
  }, [phase, jobStartedAt]);

  const label = destination?.type === "preset" ? presetById(destination.presetId)?.output_label ?? "" : destination?.type === "custom" ? "your limit" : "";

  let body: React.ReactNode;
  if (phase === "empty") {
    body = <p className="m-0 text-on-surface-muted">Add files to see what you'll get.</p>;
  } else if (phase === "files") {
    body = <p className="m-0 text-on-surface-muted">Pick where it's going to see what you'll get.</p>;
  } else if (phase === "planning") {
    body = <p className="m-0 flex items-center gap-2 text-on-surface-muted"><Spinner /> Working out the best settings…</p>;
  } else if (phase === "planned" && plan) {
    if (plan.verdict.type === "cannot_fit") {
      body = <RefusalCard refusal={plan.verdict.refusal} onSuggestion={onSuggestion} />;
    } else {
      body = (
        <div className="flex flex-col gap-3">
          <p className="m-0 text-base" data-testid="prediction">{plan.headline}</p>
          {plan.packaging.type === "archive" && <p className="m-0 text-xs text-on-surface-muted">These will go into one zip file: {plan.packaging.file_name}</p>}
          {plan.items.flatMap((p) => p.prediction.notes).slice(0, 3).map((n, i) => <p key={i} className="m-0 text-xs text-warning">{n}</p>)}
          {blockedReason && <p className="m-0 text-xs text-warning">{blockedReason}</p>}
        </div>
      );
    }
  } else if (phase === "running") {
    const fractions = items.map((i) => progress[i.id]?.fraction ?? 0);
    const total = fractions.reduce((a, b) => a + b, 0) / Math.max(1, fractions.length);
    const eta = total > 0.02 ? Math.max(0, (elapsed / total) * (1 - total)) : null;
    body = (
      <div className="flex flex-col gap-3" data-testid="progress">
        <div className="flex items-center justify-between text-sm"><span>Compressing…</span><span className="text-on-surface-muted" aria-live="polite">{eta !== null ? `About ${duration(eta)} left` : "Starting"}</span></div>
        <div className="smg-progress" role="progressbar" aria-valuenow={Math.round(total * 100)} aria-valuemin={0} aria-valuemax={100}><div style={{ width: `${Math.round(total * 100)}%` }} /></div>
        <div><button className="smg-btn smg-btn--secondary" onClick={onCancel} data-testid="cancel">Cancel</button></div>
      </div>
    );
  } else if (phase === "done" && summary) {
    const outcomes = summary.outcomes.map(([, o]) => o);
    const artifacts: Artifact[] = summary.packaged ? [summary.packaged] : outcomes.flatMap((o) => (o.type === "fitted" || o.type === "kept_original" ? [o.artifact] : []));
    const refusals = summary.outcomes.filter(([, o]) => o.type === "refused" || o.type === "failed");
    const headerColor = summary.verdict === "all_fit" ? "text-success" : summary.verdict === "some_fit" ? "text-warning" : "text-danger";
    const Icon = summary.verdict === "all_fit" ? Check : summary.verdict === "some_fit" ? AlertTriangle : XCircle;
    const single = artifacts.length === 1 && items.length === 1;
    const first = artifacts[0];
    const kept = outcomes.length === 1 && outcomes[0]?.type === "kept_original";
    body = (
      <div className="smg-fade-in flex flex-col gap-2" data-testid="result">
        <div className={clsx("flex items-center gap-2 font-display text-type-2", headerColor)} data-testid="result-headline"><Icon size={22} /> {kept ? "Already fits. Nothing to change." : summary.headline}</div>
        {artifacts.length > 0 && !kept && <div className="text-type-1 font-display" data-testid="size-arrow">{mb(summary.input_bytes)} → {mb(summary.total_bytes)}</div>}
        {single && first && <div className="text-on-surface-muted">{first.file_name}{first.summary ? ` · ${first.summary}` : ""}{first.quality ? ` · Quality: ${cap(first.quality)}` : ""}</div>}
        {!single && artifacts.length > 0 && <div className="text-on-surface-muted">{summary.packaged ? summary.packaged.file_name : `${artifacts.length} files saved`}</div>}
        {artifacts.length > 0 && <ArtifactActions artifacts={artifacts} single={single} />}
        {refusals.length > 0 && (
          <ul className="mt-2 flex flex-col gap-2">
            {refusals.map(([id, o]) => {
              const item = items.find((i) => i.id === id);
              return <li key={id} className="rounded-card border border-danger-border bg-danger-container px-3 py-2 text-sm"><span className="font-medium">{item?.rel_path.split("/").pop()}</span>: {o.type === "refused" ? o.refusal.message : o.type === "failed" ? o.failure.message : ""}
                {o.type === "refused" && o.refusal.suggestions.length > 0 && <div className="mt-2 flex flex-wrap gap-2">{o.refusal.suggestions.map((s, i) => <button key={i} className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => onSuggestion(s)}>{suggestionLabel(s)}</button>)}</div>}
              </li>;
            })}
          </ul>
        )}
        {summary.verdict === "none_fit" && outcomes.some((o) => o.type === "failed") && <button className="smg-btn smg-btn--ghost smg-btn--sm self-start" onClick={() => navigator.clipboard?.writeText(JSON.stringify(summary, (_, v) => (typeof v === "bigint" ? Number(v) : v), 2))}>Copy details for support</button>}
        <div className="mt-2"><button className="smg-btn smg-btn--ghost" onClick={() => dispatch({ type: "start_over" })} data-testid="start-over"><RotateCcw size={16} /> Compress something else</button></div>
      </div>
    );
  }

  const stepLabel = phase === "done" ? (summary?.verdict === "cancelled" ? "Stopped" : "Done") : phase === "running" ? "Compressing" : phase === "planned" ? (plan?.verdict.type === "cannot_fit" ? "Can't fit" : "Ready") : "Check";

  return (
    <section className="flex flex-col gap-3" data-tour="prediction" data-testid="step-three">
      <div className="flex items-center gap-2"><span className={clsx("smg-step", (phase === "planned" || phase === "running" || phase === "done") && "smg-step--active")}>3</span><h2 className="text-type-3 font-sans text-on-surface-muted">{stepLabel}</h2></div>
      <div className="smg-card p-4">
        {body}
        {(phase === "planned" || phase === "planning" || phase === "files" || phase === "empty") && (
          <div className="mt-4 flex items-center justify-between gap-3">
            <button className="smg-btn smg-btn--primary min-w-[160px]" disabled={!canCompress} onClick={onCompress} data-testid="compress">Compress{label && canCompress ? ` for ${label}` : ""}</button>
            <button className="smg-btn smg-btn--ghost" data-tour="advanced" data-testid="advanced" onClick={() => dispatch({ type: "drawer", open: true })}>Advanced <ChevronRight size={16} /></button>
          </div>
        )}
      </div>
    </section>
  );
}

function cap(s: string) { return s.charAt(0).toUpperCase() + s.slice(1); }
export type { ItemOutcome };
