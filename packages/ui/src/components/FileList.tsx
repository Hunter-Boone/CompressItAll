import { Archive, FileText, Film, Image as ImageIcon, Music, File as FileIcon, Folder, Presentation, Scissors, X } from "lucide-react";
import { clsx } from "clsx";
import { useMemo, useState } from "react";
import { duration, mb, type InputItem, type Kind } from "@cia/engine-client";
import { useStore, type ItemProgress } from "../state/store";

export function kindIcon(kind: Kind, format?: string) {
  const size = 18;
  switch (kind) {
    case "image": case "animated_image": return <ImageIcon size={size} />;
    case "video": return <Film size={size} />;
    case "audio": return <Music size={size} />;
    case "pdf": case "text": return <FileText size={size} />;
    case "office_doc": return ["pptx", "pptm", "odp"].includes(format ?? "") ? <Presentation size={size} /> : <FileText size={size} />;
    case "archive": return <Archive size={size} />;
    default: return <FileIcon size={size} />;
  }
}

export function itemDetail(item: InputItem): string {
  const d = item.detail;
  switch (item.kind) {
    case "image": return d.width && d.height ? `${d.width} × ${d.height} photo` : "Photo";
    case "animated_image": return d.frame_count ? `${d.frame_count}-frame animation` : "Animated image";
    case "video": return d.duration_ms ? `${duration(d.duration_ms)} video` : "Video";
    case "audio": return d.duration_ms ? `${duration(d.duration_ms)} audio` : "Audio";
    case "pdf": return d.page_count ? `${d.page_count}-page PDF` : "PDF";
    case "office_doc": return ["pptx", "pptm", "odp"].includes(d.format) ? "Presentation" : ["xlsx", "xlsm", "ods"].includes(d.format) ? "Spreadsheet" : "Document";
    case "archive": return d.entry_count ? `Zip with ${d.entry_count} files` : "Zip file";
    case "text": return "Text file";
    default: return "File";
  }
}

function RowState({ p }: { p: ItemProgress | undefined }) {
  if (!p) return null;
  const s = p.state.type;
  const working = s === "encoding" || s === "verifying" || s === "retry" || s === "inspecting" || s === "planning";
  const color = s === "fitted" || s === "kept_original" ? "text-success" : s === "refused" || s === "failed" ? "text-danger" : "text-on-surface-muted";
  const sizeText = p.outcome && (p.outcome.type === "fitted" || p.outcome.type === "kept_original") ? mb(p.outcome.artifact.bytes) : null;
  return (
    <div className="flex min-w-[120px] flex-col items-end gap-1 text-xs" data-testid="row-state">
      <span className={clsx("font-medium", color)}>{sizeText ? `${p.label} · ${sizeText}` : p.label}</span>
      {working && <div className="smg-progress smg-progress--thin w-full"><div style={{ width: `${Math.round(p.fraction * 100)}%` }} /></div>}
    </div>
  );
}

export default function FileList({ needsFfmpeg, onSetupFfmpeg }: { needsFfmpeg: (item: InputItem) => boolean; onSetupFfmpeg: () => void }) {
  const { state, dispatch } = useStore();
  const { items, progress, phase } = state;
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const locked = phase === "running";

  const groups = useMemo(() => {
    const map = new Map<string, InputItem[]>();
    for (const i of items) {
      const key = i.folder ?? `__file__${i.id}`;
      map.set(key, [...(map.get(key) ?? []), i]);
    }
    return [...map.entries()];
  }, [items]);

  const row = (item: InputItem) => {
    const p = progress[item.id];
    const corrupt = item.detail.format === "corrupt";
    const ff = needsFfmpeg(item);
    return (
      <li key={item.id} className="smg-row" data-testid="file-row">
        <span className="text-on-surface-muted">{kindIcon(item.kind, item.detail.format)}</span>
        <div className="min-w-0 flex-1">
          <div className="truncate font-medium" title={item.rel_path}>{item.rel_path.split("/").pop()}</div>
          <div className="truncate text-xs text-on-surface-muted">
            {mb(item.bytes)} · {corrupt ? <span className="text-danger">Damaged file</span> : itemDetail(item)}
            {ff && <> · <span className="text-warning">Needs video support</span> <button className="underline" onClick={onSetupFfmpeg}>Set up</button></>}
          </div>
        </div>
        {phase !== "running" && phase !== "done" && (item.kind === "video" || item.kind === "audio") && !ff && (
          <button className="smg-btn smg-btn--ghost smg-btn--sm" onClick={() => dispatch({ type: "trim", itemId: state.trimItemId === item.id ? null : item.id })} data-testid="trim-button"><Scissors size={14} /> Trim</button>
        )}
        <RowState p={p} />
        {!locked && <button className="smg-btn smg-btn--ghost smg-btn--sm !px-1.5" aria-label={`Remove ${item.rel_path}`} onClick={() => dispatch({ type: "remove_item", id: item.id })}><X size={16} /></button>}
      </li>
    );
  };

  return (
    <ul className="flex flex-col gap-2" data-testid="file-list">
      {groups.map(([key, group]) => {
        if (key.startsWith("__file__")) return row(group[0]!);
        const total = group.reduce((a, i) => a + Number(i.bytes), 0);
        const photos = group.filter((i) => i.kind === "image").length;
        const what = photos === group.length ? `${group.length} photos` : `${group.length} files`;
        const isOpen = open[key] ?? false;
        const done = group.filter((i) => progress[i.id]?.outcome).length;
        return (
          <li key={key} className="flex flex-col gap-2">
            <div className="smg-row cursor-pointer" onClick={() => setOpen({ ...open, [key]: !isOpen })} data-testid="folder-row">
              <span className="text-on-surface-muted"><Folder size={18} /></span>
              <div className="min-w-0 flex-1">
                <div className="truncate font-medium">{key}</div>
                <div className="text-xs text-on-surface-muted">{what} · {mb(total)}{phase === "running" || phase === "done" ? ` · ${done} of ${group.length} done` : ""}</div>
              </div>
              <span className="text-xs text-on-surface-subtle">{isOpen ? "Hide" : "Show"}</span>
              {!locked && <button className="smg-btn smg-btn--ghost smg-btn--sm !px-1.5" aria-label={`Remove folder ${key}`} onClick={(e) => { e.stopPropagation(); for (const i of group) dispatch({ type: "remove_item", id: i.id }); }}><X size={16} /></button>}
            </div>
            {isOpen && <ul className="ml-6 flex flex-col gap-2">{group.map(row)}</ul>}
          </li>
        );
      })}
    </ul>
  );
}
