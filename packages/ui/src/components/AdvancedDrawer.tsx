import { X } from "lucide-react";
import { useEffect } from "react";
import { createPortal } from "react-dom";
import { useHost } from "../host/HostContext";
import { useStore } from "../state/store";
import { Select, Switch } from "./primitives";

export default function AdvancedDrawer() {
  const { host, settings, updateSettings, caps } = useHost();
  const { state, dispatch } = useStore();
  const close = () => dispatch({ type: "drawer", open: false });
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") close(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
  const o = settings.options;
  const set = (patch: Partial<typeof o>) => updateSettings((s) => ({ ...s, options: { ...s.options, ...patch } }));
  const kinds = new Set(state.items.map((i) => i.kind));
  const smaller = state.destination?.type === "smaller";
  const audioTracks = Math.max(0, ...state.items.filter((i) => i.kind === "video").map((i) => i.detail.audio_streams.length));
  const desktop = host.kind === "desktop";

  return createPortal(
    <div className="fixed inset-0 z-40" data-testid="advanced-drawer">
      <div className="absolute inset-0 bg-scrim" onClick={close} />
      <aside className="smg-fade-in absolute bottom-0 right-0 top-0 flex w-full max-w-[420px] flex-col border-l border-default bg-surface-overlay shadow-3 max-sm:top-auto max-sm:max-h-[85vh] max-sm:max-w-none max-sm:rounded-t-panel max-sm:border-l-0 max-sm:border-t" role="dialog" aria-label="Advanced options">
        <div className="flex items-center justify-between border-b border-subtle px-5 py-4">
          <h2 className="font-display text-type-2">Advanced</h2>
          <button className="smg-btn smg-btn--ghost smg-btn--sm !px-2" onClick={close} aria-label="Close"><X size={16} /></button>
        </div>
        <div className="flex-1 overflow-auto px-5 py-3">
          <p className="mb-2 text-xs text-on-surface-muted">These apply to this job and become your defaults.</p>
          {(kinds.has("image") || kinds.has("animated_image") || kinds.has("office_doc") || kinds.has("pdf") || kinds.size === 0) && (
            <section className="mb-4">
              <h3 className="smg-label mb-1">Photos</h3>
              <Select label="Limit photo size" description="Longest edge. Off keeps the original size unless it must shrink." value={String(o.max_long_edge ?? "off")} onChange={(v) => set({ max_long_edge: v === "off" ? null : Number(v) })} options={[{ value: "off", label: "Off" }, { value: "4096", label: "4096 px" }, { value: "3000", label: "3000 px" }, { value: "2048", label: "2048 px" }, { value: "1600", label: "1600 px" }, { value: "1280", label: "1280 px" }]} />
              <Switch label="Keep photo details like date and camera" description="Off removes them, which also makes files a little smaller." checked={o.keep_photo_details} onChange={(v) => set({ keep_photo_details: v, keep_location: v && o.keep_location })} />
              <Switch label="Keep location" description="Where the photo was taken. Only with photo details on." checked={o.keep_location} disabled={!o.keep_photo_details} onChange={(v) => set({ keep_location: v })} />
              <Switch label="Allow format changes" description="For example a PNG photo becomes a JPEG when that fits better." checked={o.allow_format_change} onChange={(v) => set({ allow_format_change: v })} />
              {smaller && <Switch label="Modern formats" description="Use WebP or AVIF when much smaller. Some older apps can't open them." checked={o.modern_formats} onChange={(v) => set({ modern_formats: v })} />}
              <Switch label="Flatten transparency onto white" description="Lets see-through images become JPEGs." checked={o.flatten_transparency} onChange={(v) => set({ flatten_transparency: v })} />
            </section>
          )}
          {(kinds.has("video") || kinds.size === 0) && (
            <section className="mb-4">
              <h3 className="smg-label mb-1">Video</h3>
              <Select label="Format" description="Most compatible plays everywhere. Smaller files plays in Discord and browsers." value={o.video.format} onChange={(v) => set({ video: { ...o.video, format: v } })} options={[{ value: "most_compatible", label: "Most compatible (H.264 MP4)" }, { value: "smaller_files", label: "Smaller files (VP9 WebM)" }]} />
              {desktop && <Switch label="Faster, uses your graphics card" description="Slightly less precise sizes; Smidge checks and retries if needed." checked={o.video.faster} onChange={(v) => set({ video: { ...o.video, faster: v } })} />}
              <Select label="Sound" description="Clips with several tracks are mixed into one by default." value={o.video.audio.type === "track" ? `track:${o.video.audio.index}` : o.video.audio.type} onChange={(v) => set({ video: { ...o.video, audio: v.startsWith("track:") ? { type: "track", index: Number(v.slice(6)) } : v === "remove" ? { type: "remove" } : { type: "mix_all" } } })} options={[{ value: "mix_all", label: "Mix all tracks" }, ...Array.from({ length: audioTracks }, (_, i) => ({ value: `track:${i}`, label: `Track ${i + 1}` })), { value: "remove", label: "Remove sound" }]} />
              <Select label="Frame rate" description="Automatic drops 60 fps to 30 only when it must." value={o.video.frame_rate} onChange={(v) => set({ video: { ...o.video, frame_rate: v } })} options={[{ value: "automatic", label: "Automatic" }, { value: "keep_original", label: "Keep original" }, { value: "fps30", label: "30 fps" }]} />
            </section>
          )}
          {(kinds.has("audio") || kinds.size === 0) && (
            <section className="mb-4">
              <h3 className="smg-label mb-1">Music and audio</h3>
              <Select label="Format" description="Automatic picks what the destination accepts." value={o.audio.format} onChange={(v) => set({ audio: { format: v } })} options={[{ value: "automatic", label: "Automatic" }, ...(caps?.mp3_encode ? [{ value: "mp3" as const, label: "MP3" }] : []), ...(caps?.aac_encode ? [{ value: "aac" as const, label: "AAC" }] : []), { value: "opus", label: "Opus" }, { value: "flac", label: "FLAC (lossless)" }]} />
            </section>
          )}
          {(kinds.has("pdf") || kinds.has("office_doc") || kinds.size === 0) && (
            <section className="mb-4">
              <h3 className="smg-label mb-1">Documents and PDFs</h3>
              <Switch label="Keep document details" description="Author, title and similar information." checked={o.keep_document_details} onChange={(v) => set({ keep_document_details: v })} />
            </section>
          )}
          <section className="mb-4">
            <h3 className="smg-label mb-1">Folders and archives</h3>
            <Select label="Packaging" description="How several files are delivered." value={settings.packaging} onChange={(v) => updateSettings({ packaging: v })} options={[{ value: "auto", label: "Automatic" }, { value: "separate_files", label: "Separate files" }, { value: "zip", label: "Zip (works everywhere)" }, { value: "seven_zip", label: "7z (smaller)" }, { value: "tar_zst", label: ".tar.zst (fastest, technical)" }]} />
            <Switch label="Optimise files inside zip files" description="Opens zips you add and shrinks what's inside." checked={o.optimise_inside_archives} onChange={(v) => set({ optimise_inside_archives: v })} />
          </section>
          {desktop && (
            <section className="mb-4">
              <h3 className="smg-label mb-1">Output</h3>
              <div className="flex items-center justify-between gap-3 py-2">
                <span><span className="block font-medium">Where to save</span><span className="block text-xs text-on-surface-muted">{settings.outputDir.type === "folder" ? settings.outputDir.path : "Next to the original"}</span></span>
                <div className="flex gap-2">
                  {settings.outputDir.type === "folder" && <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => updateSettings({ outputDir: { type: "same_as_source" } })}>Next to original</button>}
                  <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={async () => { const p = await host.actions.chooseOutputFolder?.(); if (p) updateSettings({ outputDir: { type: "folder", path: p } }); }}>Always save to…</button>
                </div>
              </div>
            </section>
          )}
        </div>
      </aside>
    </div>,
    document.body,
  );
}
