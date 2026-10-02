import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { X } from "lucide-react";
import type { EngineHost, EngineEvent, Goal, InputItem, InputSource, Plan, PlanRequest, Suggestion } from "@cia/engine-client";
import { presetById, resolveCustom, resolvePreset } from "@cia/engine-client";
import { clsx } from "clsx";
import { HostProvider, useHost } from "./host/HostContext";
import { StoreProvider, useStore, type Destination } from "./state/store";
import { TOUR_VERSION } from "./lib/settings";
import { tourSteps } from "./lib/tutorial";
import Header from "./components/Header";
import DropZone from "./components/DropZone";
import FileList from "./components/FileList";
import Destinations, { destinationFromPreset } from "./components/Destinations";
import StepThree from "./components/StepThree";
import AdvancedDrawer from "./components/AdvancedDrawer";
import SettingsModal from "./components/SettingsModal";
import LicensesPage, { type LicensesData } from "./components/LicensesPage";
import SpotlightTutorial from "./components/SpotlightTutorial";
import TrimPanel from "./components/TrimPanel";
import { ActivateModal, AllowanceModal, FfmpegSetupModal, HelpModal, Toast, UpgradeModal, WelcomeCard } from "./components/Modals";

export interface AppProps {
  host: EngineHost;
  /** Third-party licence data, or a loader for it (the JSON is about 1 MB, so hosts load it lazily). */
  licenses?: LicensesData | (() => Promise<LicensesData>);
}

export function App({ host, licenses }: AppProps) {
  return (
    <HostProvider host={host}>
      <StoreProvider>
        <Shell licenses={licenses ?? { generated_at: "", groups: [] }} />
      </StoreProvider>
    </HostProvider>
  );
}

const VIDEO_BANNER_KEY = "cia.videoBannerDismissed";

function goalFor(dest: Destination): Goal {
  switch (dest.type) {
    case "preset": return { type: "fit", preset_id: dest.presetId, limit: resolvePreset(presetById(dest.presetId)!) };
    case "custom": return { type: "fit", preset_id: "custom", limit: resolveCustom(dest.bytes, dest.perMessage) };
    case "smaller": return { type: "smaller", level: "keep_quality" };
  }
}

function Shell({ licenses }: { licenses: LicensesData | (() => Promise<LicensesData>) }) {
  const [licenseData, setLicenseData] = useState<LicensesData>(typeof licenses === "function" ? { generated_at: "", groups: [] } : licenses);
  const { host, caps, settings, updateSettings, license, allowance, refreshAllowance, ready } = useHost();
  const { state, dispatch } = useStore();
  const [tourStep, setTourStep] = useState<number | null>(null);
  const [pendingTrim, setPendingTrim] = useState<Record<string, [number, number]>>({});
  const web = host.kind === "web";
  // Web first run (DESIGN.md 4.7): one dismissible banner when the browser cannot make videos smaller.
  const [videoBannerDismissed, setVideoBannerDismissed] = useState(() => { try { return localStorage.getItem(VIDEO_BANNER_KEY) === "1"; } catch { return false; } });
  const showVideoBanner = web && !!caps && !caps.video && !videoBannerDismissed;
  const dismissVideoBanner = () => { setVideoBannerDismissed(true); try { localStorage.setItem(VIDEO_BANNER_KEY, "1"); } catch { /* storage unavailable */ } };
  const steps = useMemo(() => tourSteps(web), [web]);

  // First run: welcome card, then the tour (DESIGN.md 4.7).
  useEffect(() => {
    if (!ready) return;
    if (!settings.welcomeSeen) dispatch({ type: "modal", modal: "welcome" });
    else if (settings.tourVersionSeen < TOUR_VERSION) setTourStep(0);
  }, [ready]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (state.modal === "licenses" && typeof licenses === "function" && licenseData.groups.length === 0) {
      licenses().then(setLicenseData).catch(() => {});
    }
  }, [state.modal, licenses, licenseData.groups.length]);

  const finishTour = useCallback(() => { setTourStep(null); updateSettings({ tourVersionSeen: TOUR_VERSION, welcomeSeen: true }); }, [updateSettings]);
  const startTour = () => { dispatch({ type: "modal", modal: null }); setTourStep(0); };

  const addSources = useCallback(async (sources: InputSource[]) => {
    if (!sources.length) return;
    const items = await host.addInputs(sources);
    if (items.length) dispatch({ type: "add_items", items });
  }, [host, dispatch]);

  // Desktop: OS drops land on the window, not the DOM (DESIGN.md 4.3).
  useEffect(() => host.onExternalInputs?.(addSources), [host, addSources]);

  const needsFfmpeg = useCallback((item: InputItem) => host.kind === "desktop" && !caps?.ffmpeg && (item.kind === "video" || (item.kind === "audio" && ["m4a", "aac", "alac", "wma"].includes(item.detail.format))), [host.kind, caps]);

  const request = useCallback((items: InputItem[], dest: Destination): PlanRequest => ({
    items,
    goal: goalFor(dest),
    packaging: settings.packaging,
    options: { ...settings.options, allow_format_change: dest.type === "smaller" ? false : settings.options.allow_format_change, video: { ...settings.options.video, trims: pendingTrim }, output_dir: settings.outputDir.type === "folder" ? { type: "folder", path: settings.outputDir.path } : { type: "same_as_source" } },
  }), [settings, pendingTrim]);

  // Plan whenever files, destination or options change (debounced 300 ms).
  const planToken = useRef(0);
  useEffect(() => {
    if (state.phase === "running" || state.phase === "done") return;
    if (!state.items.length || !state.destination) return;
    const token = ++planToken.current;
    dispatch({ type: "planning", token });
    const t = window.setTimeout(async () => {
      try {
        const plan: Plan = await host.preview(request(state.items, state.destination!));
        dispatch({ type: "planned", plan, token });
      } catch (e) {
        console.error(e);
        dispatch({ type: "plan_failed", token });
      }
    }, 300);
    return () => window.clearTimeout(t);
  }, [state.items, state.destination, settings.options, settings.packaging, pendingTrim]); // eslint-disable-line react-hooks/exhaustive-deps

  // Remember the last destination; restore it on launch.
  useEffect(() => {
    if (!ready || state.destination) return;
    const p = settings.lastPresetId ? presetById(settings.lastPresetId) : null;
    if (p) dispatch({ type: "set_destination", destination: destinationFromPreset(p, settings) });
  }, [ready]); // eslint-disable-line react-hooks/exhaustive-deps

  const compress = useCallback(async (limitTo?: number) => {
    if (!state.destination || !state.plan) return;
    let items = state.items;
    const free = license.status !== "pro";
    if (free) {
      const a = await host.allowance();
      const countable = items.filter((i) => state.plan!.items.find((p) => p.item_id === i.id)?.strategy.type !== "copy").length;
      if (limitTo !== undefined) items = items.slice(0, limitTo);
      else if (a.remaining === 0 || countable > a.remaining) { dispatch({ type: "modal", modal: "allowance" }); return; }
    }
    const handle = await host.run(request(items, state.destination), (e: EngineEvent) => dispatch({ type: "engine_event", event: e }));
    dispatch({ type: "job_started", jobId: handle.jobId });
    const summary = await handle.done;
    dispatch({ type: "job_finished", summary });
    void refreshAllowance();
  }, [state, license, host, request, dispatch, refreshAllowance]);

  const cancel = useCallback(() => { if (state.jobId) void host.cancel(state.jobId); }, [state.jobId, host]);

  const onSuggestion = useCallback((s: Suggestion) => {
    switch (s.type) {
      case "trim": {
        const v = state.items.find((i) => i.kind === "video" || i.kind === "audio");
        if (v) { setPendingTrim((t) => ({ ...t, [v.id]: [0, Number(s.max_duration_ms) - 1000] })); dispatch({ type: "trim", itemId: v.id }); }
        break;
      }
      case "pick_preset": { const p = presetById(s.preset_id); if (p) { updateSettings((st) => ({ ...st, tierByGroup: { ...st.tierByGroup, [p.group]: p.id }, lastPresetId: p.id })); dispatch({ type: "set_destination", destination: destinationFromPreset(p, settings) }); } break; }
      case "remove_files": for (const id of s.item_ids) dispatch({ type: "remove_item", id }); break;
      case "split_into_messages": dispatch({ type: "toast", message: "Splitting into several messages is coming in the next version. Remove a few files for now." }); window.setTimeout(() => dispatch({ type: "toast", message: null }), 4000); break;
      case "install_ffmpeg": dispatch({ type: "modal", modal: "ffmpeg" }); break;
      case "use_desktop_app": void host.actions.openExternal?.("https://www.smidge.example/download"); break;
      case "use_other_browser": break;
    }
  }, [state.items, settings, updateSettings, dispatch, host]);

  // Keyboard shortcuts (DESIGN.md 4.3).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (mod && e.key.toLowerCase() === "o") { e.preventDefault(); void (host.actions.chooseFiles ? host.actions.chooseFiles().then(addSources) : document.querySelector<HTMLInputElement>("[data-testid=file-input]")?.click()); }
      if (mod && e.key === "Enter" && state.phase === "planned") { e.preventDefault(); void compress(); }
      if (mod && e.key === ",") { e.preventDefault(); dispatch({ type: "modal", modal: "settings" }); }
      if (e.key === "Escape" && state.phase === "running") cancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [host, addSources, compress, cancel, state.phase, dispatch]);

  const canCompress = state.phase === "planned" && !!state.plan && state.plan.verdict.type !== "cannot_fit" && !state.items.some(needsFfmpeg);
  const blockedReason = state.phase === "planned" && state.items.some(needsFfmpeg) ? "Some files need video support before they can be compressed." : null;
  const trimItem = state.trimItemId ? state.items.find((i) => i.id === state.trimItemId) : null;
  const hasFiles = state.items.length > 0;

  return (
    <div className="flex h-full min-h-screen flex-col bg-surface-base text-on-surface">
      <Header />
      {showVideoBanner && (
        <div className="mx-auto w-full max-w-[980px] px-5 pb-2" data-testid="video-banner" role="status">
          <div className="flex items-center justify-between gap-3 rounded-card border border-subtle bg-surface-sunken px-3 py-2 text-xs text-on-surface-muted">
            <span>This browser can't make videos smaller. Chrome, Edge or Safari can, or use the desktop app.</span>
            <button className="smg-btn smg-btn--ghost smg-btn--sm !px-1.5" aria-label="Dismiss" onClick={dismissVideoBanner} data-testid="video-banner-dismiss"><X size={14} /></button>
          </div>
        </div>
      )}
      <main className="mx-auto flex w-full max-w-[980px] flex-1 flex-col gap-6 px-5 pb-8 pt-2">
        <section className="flex flex-col gap-3" data-testid="step-one">
          <div className="flex items-center gap-2"><span className={clsx("smg-step", !hasFiles && "smg-step--active")}>1</span><h2 className="text-type-3 font-sans text-on-surface-muted">Add your files</h2></div>
          {hasFiles && <FileList needsFfmpeg={needsFfmpeg} onSetupFfmpeg={() => dispatch({ type: "modal", modal: "ffmpeg" })} />}
          {trimItem && <TrimPanel item={trimItem} value={pendingTrim[trimItem.id] ?? null} onChange={(r) => setPendingTrim((t) => { const n = { ...t }; if (r) n[trimItem.id] = r; else delete n[trimItem.id]; return n; })} onClose={() => dispatch({ type: "trim", itemId: null })} />}
          {state.phase !== "running" && state.phase !== "done" && <DropZone compact={hasFiles} onAdd={addSources} />}
        </section>
        <section className="flex flex-col gap-3" data-testid="step-two">
          <div className="flex items-center gap-2"><span className={clsx("smg-step", hasFiles && !state.destination && "smg-step--active")}>2</span><h2 className="text-type-3 font-sans text-on-surface-muted">Where are you sending it?</h2></div>
          <Destinations />
        </section>
        <StepThree onCompress={() => void compress()} onCancel={cancel} onSuggestion={onSuggestion} canCompress={canCompress} blockedReason={blockedReason} />
      </main>

      {state.drawerOpen && <AdvancedDrawer />}
      {state.modal === "settings" && <SettingsModal onStartTour={startTour} />}
      {state.modal === "licenses" && <LicensesPage data={licenseData} />}
      {state.modal === "upgrade" && <UpgradeModal />}
      {state.modal === "activate" && <ActivateModal />}
      {state.modal === "ffmpeg" && <FfmpegSetupModal />}
      {state.modal === "help" && <HelpModal onTour={startTour} />}
      {state.modal === "allowance" && <AllowanceModal nextFreeAtMs={allowance.nextFreeAtMs} remaining={allowance.remaining} files={state.items.length} onDoSome={(n) => void compress(n)} />}
      {state.modal === "welcome" && <WelcomeCard onTour={() => { dispatch({ type: "modal", modal: null }); updateSettings({ welcomeSeen: true }); setTourStep(0); }} onSkip={() => { dispatch({ type: "modal", modal: null }); updateSettings({ welcomeSeen: true, tourVersionSeen: TOUR_VERSION }); }} />}
      {tourStep !== null && <SpotlightTutorial steps={steps} stepIndex={tourStep} onStepIndexChange={setTourStep} onFinish={finishTour} />}
      {state.toast && <Toast message={state.toast} />}
    </div>
  );
}
