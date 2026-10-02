import { FolderOpen, Upload } from "lucide-react";
import { clsx } from "clsx";
import { useCallback, useRef, useState, type DragEvent } from "react";
import type { InputSource } from "@cia/engine-client";
import { useHost } from "../host/HostContext";

/** Walk a dropped folder with webkitGetAsEntry (every browser) into File sources with relative paths. */
export async function sourcesFromDataTransfer(dt: DataTransfer): Promise<InputSource[]> {
  const out: InputSource[] = [];
  const entries: FileSystemEntry[] = [];
  for (const item of Array.from(dt.items)) {
    const entry = (item as DataTransferItem & { webkitGetAsEntry?: () => FileSystemEntry | null }).webkitGetAsEntry?.();
    if (entry) entries.push(entry);
  }
  if (entries.length === 0) {
    for (const file of Array.from(dt.files)) out.push({ kind: "file", file });
    return out;
  }
  const walk = async (entry: FileSystemEntry, prefix: string): Promise<void> => {
    if (entry.isFile) {
      const file = await new Promise<File>((res, rej) => (entry as FileSystemFileEntry).file(res, rej));
      if (!file.name.startsWith(".")) out.push({ kind: "file", file, relPath: prefix ? `${prefix}/${file.name}` : file.name });
    } else if (entry.isDirectory) {
      const reader = (entry as FileSystemDirectoryEntry).createReader();
      const all: FileSystemEntry[] = [];
      for (;;) {
        const batch = await new Promise<FileSystemEntry[]>((res, rej) => reader.readEntries(res, rej));
        if (batch.length === 0) break;
        all.push(...batch);
      }
      for (const e of all) await walk(e, prefix ? `${prefix}/${entry.name}` : entry.name);
    }
  };
  for (const e of entries) await walk(e, "");
  return out;
}

export default function DropZone({ compact, onAdd }: { compact: boolean; onAdd: (sources: InputSource[]) => void }) {
  const { host, caps } = useHost();
  const [active, setActive] = useState(false);
  const depth = useRef(0);
  const fileInput = useRef<HTMLInputElement>(null);
  const folderInput = useRef<HTMLInputElement>(null);

  const onDrop = useCallback(async (e: DragEvent) => {
    e.preventDefault();
    depth.current = 0;
    setActive(false);
    onAdd(await sourcesFromDataTransfer(e.dataTransfer));
  }, [onAdd]);

  const chooseFiles = async () => {
    if (host.actions.chooseFiles) onAdd(await host.actions.chooseFiles());
    else fileInput.current?.click();
  };
  const chooseFolder = async () => {
    if (host.actions.chooseFolder) onAdd(await host.actions.chooseFolder());
    else if ("showDirectoryPicker" in window) {
      try {
        const dir = await (window as unknown as { showDirectoryPicker(): Promise<FileSystemDirectoryHandle> }).showDirectoryPicker();
        onAdd([{ kind: "directory", handle: dir }]);
      } catch { /* cancelled */ }
    } else folderInput.current?.click();
  };
  const web = host.kind === "web";
  const canFolder = caps?.can_choose_folder ?? !web;

  return (
    <div
      data-tour="dropzone"
      data-testid="dropzone"
      className={clsx("smg-dropzone flex flex-col items-center justify-center gap-3 rounded-panel text-center transition-all", active && "smg-dropzone--active", compact ? "px-4 py-3" : "px-6 py-12")}
      onDragEnter={(e) => { e.preventDefault(); depth.current++; setActive(true); }}
      onDragOver={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; }}
      onDragLeave={() => { depth.current = Math.max(0, depth.current - 1); if (depth.current === 0) setActive(false); }}
      onDrop={onDrop}
    >
      {compact ? (
        <div className="flex flex-wrap items-center justify-center gap-2 text-on-surface-muted">
          <span>Drop more files here, or</span>
          <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={chooseFiles}><Upload size={14} /> Choose files</button>
          {canFolder && <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={chooseFolder}><FolderOpen size={14} /> Choose folder</button>}
        </div>
      ) : (
        <>
          <div className="flex h-14 w-14 items-center justify-center rounded-full bg-primary-container text-primary-container-foreground"><Upload size={26} /></div>
          <div>
            <div className="font-display text-type-2">Drop files or a folder here</div>
            <div className="mt-1 text-on-surface-muted">{web ? "Your files stay in this browser. Nothing is uploaded." : "Your files stay on this computer."}</div>
          </div>
          <div className="mt-2 flex flex-wrap items-center justify-center gap-2">
            <button className="smg-btn smg-btn--primary" onClick={chooseFiles} data-testid="choose-files"><Upload size={16} /> Choose files</button>
            {canFolder && <button className="smg-btn smg-btn--secondary" onClick={chooseFolder} data-testid="choose-folder"><FolderOpen size={16} /> Choose folder</button>}
          </div>
        </>
      )}
      <input ref={fileInput} type="file" multiple className="hidden" data-testid="file-input" onChange={(e) => { const fs = Array.from(e.target.files ?? []); e.target.value = ""; if (fs.length) onAdd(fs.map((file) => ({ kind: "file", file }))); }} />
      <input ref={folderInput} type="file" multiple className="hidden" data-testid="folder-input" {...({ webkitdirectory: "", directory: "" } as Record<string, string>)}
        onChange={(e) => { const fs = Array.from(e.target.files ?? []); e.target.value = ""; if (fs.length) onAdd(fs.map((file) => ({ kind: "file", file, relPath: (file as File & { webkitRelativePath?: string }).webkitRelativePath || file.name }))); }} />
    </div>
  );
}
