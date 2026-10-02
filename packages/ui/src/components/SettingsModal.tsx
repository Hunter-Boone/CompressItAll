import { useEffect, useState } from "react";
import brand from "@cia/brand";
import { clsx } from "clsx";
import { useHost } from "../host/HostContext";
import { useStore } from "../state/store";
import { Modal, Select, Switch } from "./primitives";
import { UI_SCALES } from "../lib/uiScale";
import { TOUR_VERSION } from "../lib/settings";

const SECTIONS = [
  { id: "general", label: "General" },
  { id: "video", label: "Video support", desktopOnly: true },
  { id: "license", label: "License" },
  { id: "updates", label: "Updates", desktopOnly: true },
  { id: "browser", label: "This browser", webOnly: true },
  { id: "about", label: "About" },
];

export default function SettingsModal({ onStartTour }: { onStartTour: () => void }) {
  const { host, caps, settings, updateSettings, license, refreshLicense, refreshCaps } = useHost();
  const { state, dispatch } = useStore();
  const [section, setSection] = useState(state.settingsSection);
  const [version, setVersion] = useState<{ app: string; build: string; engine: string } | null>(null);
  const [update, setUpdate] = useState<{ available: boolean; version?: string } | null>(null);
  const [showKey, setShowKey] = useState(false);
  const desktop = host.kind === "desktop";
  useEffect(() => { void host.version().then(setVersion); }, [host]);
  const close = () => dispatch({ type: "modal", modal: null });
  const sections = SECTIONS.filter((s) => (desktop ? !s.webOnly : !s.desktopOnly));

  return (
    <Modal title="Settings" onClose={close} width={760} testId="settings">
      <div className="flex min-h-[380px] gap-6 max-sm:flex-col">
        <nav className="flex w-40 shrink-0 flex-col gap-1 max-sm:w-full max-sm:flex-row max-sm:overflow-auto" aria-label="Settings sections">
          {sections.map((s) => <button key={s.id} className={clsx("rounded-control px-3 py-2 text-left", section === s.id ? "bg-primary-container text-primary-container-foreground font-medium" : "hover:bg-surface-hover")} onClick={() => setSection(s.id)} aria-current={section === s.id}>{s.label}</button>)}
        </nav>
        <div className="min-w-0 flex-1">
          {section === "general" && (
            <div>
              {desktop && (
                <div className="flex items-center justify-between gap-3 py-2">
                  <span><span className="block font-medium">Where to save</span><span className="block text-xs text-on-surface-muted">{settings.outputDir.type === "folder" ? settings.outputDir.path : "Next to the original"}</span></span>
                  <div className="flex gap-2">
                    {settings.outputDir.type === "folder" && <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => updateSettings({ outputDir: { type: "same_as_source" } })}>Next to original</button>}
                    <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={async () => { const p = await host.actions.chooseOutputFolder?.(); if (p) updateSettings({ outputDir: { type: "folder", path: p } }); }}>Always save to…</button>
                  </div>
                </div>
              )}
              <Select label="Theme" value={settings.theme} onChange={(v) => updateSettings({ theme: v })} options={[{ value: "system", label: "Match system" }, { value: "light", label: "Light" }, { value: "dark", label: "Dark" }]} />
              <Select label="Text size" value={settings.textSize} onChange={(v) => updateSettings({ textSize: v })} options={UI_SCALES.map((s) => ({ value: s.id, label: s.label }))} />
              <div className="flex items-center justify-between gap-3 py-2"><span className="font-medium">Show the tour again</span><button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => { close(); onStartTour(); }}>Start tour</button></div>
            </div>
          )}
          {section === "video" && desktop && (
            <div>
              <p className="mb-3">{caps?.ffmpeg ? <>Ready · FFmpeg {caps.ffmpeg.version}{caps.ffmpeg.working_encoders.length ? ` · H.264 via ${encoderName(caps.ffmpeg.working_encoders)}` : ""}</> : "Not set up. Videos and some music files need FFmpeg."}</p>
              {caps?.ffmpeg && <ul className="mb-3 text-xs text-on-surface-muted">{caps.ffmpeg.working_encoders.map((e) => <li key={e}>{e}</li>)}</ul>}
              <div className="flex flex-wrap gap-2">
                {!caps?.ffmpeg && <button className="smg-btn smg-btn--primary smg-btn--sm" onClick={() => dispatch({ type: "modal", modal: "ffmpeg" })}>Add video support</button>}
                {caps?.ffmpeg && <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={async () => { await host.ffmpegSetup?.retest(); await refreshCaps(); }}>Re-test encoders</button>}
                <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={async () => { await host.ffmpegSetup?.useOwnCopy(); await refreshCaps(); }}>Use a different FFmpeg…</button>
                {caps?.ffmpeg && !caps.ffmpeg.user_supplied && <button className="smg-btn smg-btn--ghost smg-btn--sm" onClick={async () => { await host.ffmpegSetup?.remove(); await refreshCaps(); }}>Remove video support</button>}
              </div>
              <div className="mt-4"><Switch label="Faster, uses your graphics card" description="Default for new jobs." checked={settings.options.video.faster} onChange={(v) => updateSettings((s) => ({ ...s, options: { ...s.options, video: { ...s.options.video, faster: v } } }))} /></div>
            </div>
          )}
          {section === "license" && (
            <div data-testid="license-section">
              <p className="mb-1 font-medium">{license.status === "pro" ? `${brand.pro_name} · ${license.plan === "lifetime" ? "Lifetime" : "Yearly"}` : "Free · 3 files a day"}</p>
              {license.message && <p className="mb-2 text-xs text-warning">{license.message}</p>}
              {license.status === "pro" && (
                <>
                  <p className="mb-1 flex items-center gap-2 text-on-surface-muted"><code className="font-sans">{showKey ? license.keyMasked : `XXXXX-…-${license.key4}`}</code><button className="smg-btn smg-btn--ghost smg-btn--sm" onClick={() => setShowKey(!showKey)}>{showKey ? "Hide" : "Show"}</button></p>
                  {license.devicesUsed !== undefined && <p className="mb-3 text-xs text-on-surface-muted">This {desktop ? "computer" : "browser"} is {license.devicesUsed} of {license.devicesMax}</p>}
                  <div className="flex flex-wrap gap-2">
                    <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={async () => { await host.deactivate(); await refreshLicense(); }}>Deactivate this {desktop ? "computer" : "browser"}</button>
                    <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => host.actions.openExternal?.(`${brand.urls.site}/account`)}>Manage my account</button>
                  </div>
                </>
              )}
              {license.status !== "pro" && (
                <div className="mt-2 flex flex-wrap gap-2">
                  <button className="smg-btn smg-btn--primary smg-btn--sm" onClick={() => dispatch({ type: "modal", modal: "upgrade" })}>Buy {brand.pro_name}</button>
                  <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => dispatch({ type: "modal", modal: "activate" })}>Enter a product key</button>
                </div>
              )}
            </div>
          )}
          {section === "updates" && desktop && (
            <div>
              <Switch label="Check automatically" checked={settings.updatesAuto} onChange={(v) => updateSettings({ updatesAuto: v })} />
              <Select label="Channel" value={settings.updateChannel} onChange={(v) => updateSettings({ updateChannel: v })} options={[{ value: "stable", label: "Stable" }, { value: "beta", label: "Beta" }]} />
              <div className="flex items-center justify-between gap-3 py-2"><span className="text-on-surface-muted">Version {version?.app ?? "…"}</span><button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={async () => setUpdate(await host.updates!.check())}>Check now</button></div>
              {update && <p className="text-xs">{update.available ? <>Update {update.version} ready. <button className="underline" onClick={() => host.updates!.install()}>Restart to finish</button></> : "You're up to date."}</p>}
              <p className="text-xs"><button className="underline text-on-surface-muted" onClick={() => host.actions.openExternal?.(`${brand.urls.downloads_repo}/releases`)}>Release notes</button></p>
            </div>
          )}
          {section === "browser" && !desktop && caps?.web && (
            <ul className="flex flex-col gap-1 text-sm">
              <li>Browser: {caps.web.browser}</li>
              <li>{caps.web.webcodecs && caps.web.h264_encode ? "Can make videos smaller (H.264)." : "Can't make videos smaller. Chrome, Edge or Safari can, or use the desktop app."}</li>
              <li>{caps.web.aac_encode ? "Can make AAC audio." : "Can't make AAC audio in this browser."}</li>
              <li>{caps.web.opus_encode ? "Can make Opus audio." : "Can't make Opus audio in this browser."}</li>
              <li>{caps.heic_input ? "Opens iPhone HEIC photos." : "Can't open iPhone HEIC photos. Use Safari, or the desktop app."}</li>
              <li>{caps.web.directory_picker ? "Can save into a folder you choose." : "Saves several files as one zip download."}</li>
              <li>{caps.web.cross_origin_isolated ? "Running with full speed enabled." : "Running without multi-threading."}</li>
            </ul>
          )}
          {section === "about" && (
            <div className="flex flex-col gap-2">
              <p className="font-medium">{brand.name} {version?.app ?? ""} <span className="text-xs text-on-surface-muted">build {version?.build ?? "…"} · engine {version?.engine ?? "…"}</span></p>
              <p className="text-on-surface-muted">{brand.name} never uploads your files. It only connects to the internet to check your license{desktop ? ", download FFmpeg when you ask, and check for updates." : "."}</p>
              <p className="text-on-surface-muted">{brand.name} uses well-known open-source encoders. What {brand.name} adds is the part that picks the best settings for your limit and checks the result.</p>
              <div className="flex flex-wrap gap-2">
                <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => dispatch({ type: "modal", modal: "licenses" })}>Licenses</button>
                {host.diagnostics && <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => host.diagnostics!.copyReport()}>Copy diagnostic report</button>}
                {desktop && host.diagnostics && <button className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => host.diagnostics!.openLogs()}>Open logs folder</button>}
              </div>
              <p className="text-xs text-on-surface-muted">Support: {brand.urls.support_email}</p>
              <p className="text-xs text-on-surface-subtle">Tour version {TOUR_VERSION}</p>
            </div>
          )}
        </div>
      </div>
    </Modal>
  );
}

function encoderName(list: string[]): string {
  const first = list.find((e) => e.startsWith("h264_") || e === "libx264") ?? list[0] ?? "";
  if (first.includes("nvenc")) return "NVIDIA";
  if (first.includes("amf")) return "AMD";
  if (first.includes("qsv")) return "Intel";
  if (first.includes("videotoolbox")) return "Apple";
  if (first.includes("mf")) return "Windows";
  if (first.includes("vaapi")) return "VA-API";
  if (first === "libx264") return "x264";
  return first;
}
