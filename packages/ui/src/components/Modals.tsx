import { useEffect, useRef, useState } from "react";
import brand from "@cia/brand";
import { clockTime, mb, type FfmpegSetupProgress } from "@cia/engine-client";
import { useHost } from "../host/HostContext";
import { useStore } from "../state/store";
import { Wordmark } from "./Header";
import { Modal, Spinner } from "./primitives";
import { normalise as normaliseKey } from "@cia/api-types/product-key";

export function WelcomeCard({ onTour, onSkip }: { onTour: () => void; onSkip: () => void }) {
  return (
    <Modal onClose={onSkip} width={440} testId="welcome">
      <div className="flex flex-col items-center gap-4 py-2 text-center">
        <Wordmark size={30} />
        <p className="m-0 text-base">{brand.tagline}</p>
        <div className="flex gap-2">
          <button className="smg-btn smg-btn--primary" onClick={onTour} autoFocus data-testid="welcome-tour">Show me around</button>
          <button className="smg-btn smg-btn--ghost" onClick={onSkip} data-testid="welcome-skip">Skip</button>
        </div>
      </div>
    </Modal>
  );
}

export function HelpModal({ onTour }: { onTour: () => void }) {
  const { host } = useHost();
  const { dispatch } = useStore();
  const close = () => dispatch({ type: "modal", modal: null });
  return (
    <Modal title="Help" onClose={close} width={420}>
      <div className="flex flex-col gap-2">
        <button className="smg-btn smg-btn--secondary" onClick={() => { close(); onTour(); }}>Show the tour again</button>
        <button className="smg-btn smg-btn--secondary" onClick={() => host.actions.openExternal?.(`${brand.urls.site}/help`)}>Help articles</button>
        <button className="smg-btn smg-btn--secondary" onClick={() => host.actions.openExternal?.(`mailto:${brand.urls.support_email}`)}>Contact support</button>
      </div>
    </Modal>
  );
}

export function UpgradeModal() {
  const { host, refreshLicense } = useHost();
  const { dispatch } = useStore();
  const [waiting, setWaiting] = useState<string | null>(null);
  const [unlocked, setUnlocked] = useState(false);
  const close = () => dispatch({ type: "modal", modal: null });
  const timer = useRef<number | null>(null);
  useEffect(() => () => { if (timer.current) window.clearTimeout(timer.current); }, []);

  const buy = async (plan: "lifetime" | "yearly") => {
    const { buyUrl, claimId } = await host.startPurchase(plan);
    await host.actions.openExternal?.(buyUrl);
    setWaiting(claimId);
    const poll = async () => {
      const s = await host.pollClaim(claimId);
      if (s === "fulfilled") { await refreshLicense(); setUnlocked(true); timer.current = window.setTimeout(close, 4000); }
      else if (s === "pending") timer.current = window.setTimeout(poll, 3000);
      else setWaiting(null);
    };
    void poll();
  };

  if (unlocked) return <Modal onClose={close} width={420} testId="unlocked"><p className="m-0 py-4 text-center text-base">{brand.pro_name} is on. Thanks for supporting {brand.name}.</p></Modal>;
  if (waiting) return (
    <Modal title="Almost there" onClose={close} width={420}>
      <p className="flex items-center gap-2"><Spinner /> Finish your purchase in the browser. {brand.name} will unlock by itself.</p>
      <button className="smg-btn smg-btn--ghost smg-btn--sm mt-3" onClick={() => dispatch({ type: "modal", modal: "activate" })}>I'll enter my key instead</button>
    </Modal>
  );
  return (
    <Modal title={brand.pro_name} onClose={close} width={520} testId="upgrade">
      <p className="mb-4 text-on-surface-muted">Unlimited files, on up to 3 computers and in your browser. The free version does 3 files a day.</p>
      <div className="mb-4 grid grid-cols-2 gap-3 max-sm:grid-cols-1">
        <div className="smg-card p-4"><div className="font-display text-type-2">Lifetime</div><div className="text-on-surface-muted">Pay once, keep it.</div><button className="smg-btn smg-btn--primary mt-3 w-full" onClick={() => buy("lifetime")} data-testid="buy-lifetime">Buy Lifetime</button></div>
        <div className="smg-card p-4"><div className="font-display text-type-2">Yearly</div><div className="text-on-surface-muted">Cancel any time.</div><button className="smg-btn smg-btn--secondary mt-3 w-full" onClick={() => buy("yearly")} data-testid="buy-yearly">Buy Yearly</button></div>
      </div>
      <p className="m-0 text-xs text-on-surface-muted">Prices and tax are shown at checkout. <button className="underline" onClick={() => dispatch({ type: "modal", modal: "activate" })}>I already have a key</button></p>
    </Modal>
  );
}

export { normalise as normaliseKey } from "@cia/api-types/product-key";
export function formatKey(input: string): string {
  const s = input.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 20);
  return s.replace(/(.{5})(?=.)/g, "$1-");
}

export function ActivateModal() {
  const { host, refreshLicense } = useHost();
  const { dispatch } = useStore();
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [email, setEmail] = useState("");
  const [resent, setResent] = useState(false);
  const close = () => dispatch({ type: "modal", modal: null });
  const submit = async () => {
    const n = normaliseKey(key);
    if (!n) { setError("That key has a typo. Check the email we sent you."); return; }
    setBusy(true); setError(null);
    try {
      await host.activate(n);
      await refreshLicense();
      dispatch({ type: "modal", modal: "upgrade" });
      close();
      dispatch({ type: "toast", message: `${brand.pro_name} is on. Thanks for supporting ${brand.name}.` });
      window.setTimeout(() => dispatch({ type: "toast", message: null }), 4000);
    } catch (e) {
      setError((e as Error).message || "Something went wrong. Try again in a moment.");
    } finally { setBusy(false); }
  };
  return (
    <Modal title="Enter your product key" onClose={close} width={460} testId="activate">
      <input className="smg-input w-full font-mono text-base tracking-wider" placeholder="XXXXX-XXXXX-XXXXX-XXXXX" value={key} onChange={(e) => { setKey(formatKey(e.target.value)); setError(null); }} onKeyDown={(e) => e.key === "Enter" && submit()} autoFocus aria-label="Product key" data-testid="key-input" />
      {error && <p className="mt-2 text-xs text-danger" role="alert" data-testid="key-error">{error}</p>}
      <div className="mt-3 flex items-center gap-2">
        <button className="smg-btn smg-btn--primary" onClick={submit} disabled={busy || key.replace(/-/g, "").length < 20} data-testid="activate-button">{busy ? <Spinner /> : null} Activate</button>
      </div>
      <div className="mt-5 border-t border-subtle pt-3 text-xs text-on-surface-muted">
        Lost your key? {resent ? "If that email bought Smidge, the key is on its way." : <>
          <input className="smg-input ml-1 h-8 w-48 text-xs" placeholder="you@example.com" value={email} onChange={(e) => setEmail(e.target.value)} aria-label="Email" />
          <button className="smg-btn smg-btn--ghost smg-btn--sm ml-1" onClick={async () => { await host.resendKey(email); setResent(true); }} disabled={!email.includes("@")}>Email it to me</button>
        </>}
      </div>
    </Modal>
  );
}

export function AllowanceModal({ nextFreeAtMs, remaining, files, onDoSome }: { nextFreeAtMs: number | null; remaining: number; files: number; onDoSome: (n: number) => void }) {
  const { dispatch } = useStore();
  const close = () => dispatch({ type: "modal", modal: null });
  const tooMany = remaining > 0 && files > remaining;
  return (
    <Modal onClose={close} width={460} testId="allowance">
      <p className="m-0 text-base">{tooMany ? `This has ${files} files. The free version does ${remaining} more today.` : `You've used your 3 free files. Your next free file is ready at ${nextFreeAtMs ? clockTime(nextFreeAtMs) : "tomorrow"}, or get ${brand.pro_name} for unlimited files.`}</p>
      <div className="mt-4 flex flex-wrap gap-2">
        <button className="smg-btn smg-btn--primary" onClick={() => dispatch({ type: "modal", modal: "upgrade" })}>Get {brand.pro_name}</button>
        {tooMany ? <button className="smg-btn smg-btn--secondary" onClick={() => { onDoSome(remaining); close(); }}>Do the first {remaining} now</button> : <button className="smg-btn smg-btn--secondary" onClick={() => dispatch({ type: "modal", modal: "activate" })}>Enter a key</button>}
      </div>
    </Modal>
  );
}

export function FfmpegSetupModal() {
  const { host, refreshCaps } = useHost();
  const { dispatch } = useStore();
  const [manifest, setManifest] = useState<{ version: string; bytes: number; unpackedBytes: number } | null>(null);
  const [progress, setProgress] = useState<FfmpegSetupProgress | null>(null);
  const [explain, setExplain] = useState(false);
  const [fails, setFails] = useState(0);
  const close = () => dispatch({ type: "modal", modal: null });
  useEffect(() => { void host.ffmpegSetup?.manifest().then(setManifest).catch(() => setManifest(null)); }, [host]);
  const install = async () => {
    setProgress({ phase: "downloading", downloadedBytes: 0, totalBytes: manifest?.bytes });
    try {
      await host.ffmpegSetup!.install(setProgress);
      await refreshCaps();
      setProgress({ phase: "ready" });
      window.setTimeout(close, 1200);
    } catch (e) {
      setFails((f) => f + 1);
      setProgress({ phase: "error", message: (e as Error).message });
    }
  };
  const errorText = (m?: string) => {
    if (fails >= 2 && (m ?? "").includes("damaged")) return `${brand.name} couldn't get a good copy of FFmpeg. Contact support and we'll help.`;
    return m ?? "The download didn't finish. Check your internet connection and try again.";
  };
  return (
    <Modal title="Add video and music support" onClose={close} width={520} testId="ffmpeg-setup">
      {!progress && (
        <div className="flex flex-col gap-3">
          <p className="m-0">{brand.name} uses FFmpeg to work with videos and some music files. FFmpeg is a free, open-source program used by many apps. {brand.name} doesn't include it, so it needs to download it once.</p>
          <ul className="m-0 list-disc pl-5 text-on-surface-muted">
            <li>Download: about {manifest ? mb(manifest.bytes) : "35 MB"}, from {brand.name}'s public build page on GitHub</li>
            <li>FFmpeg runs as a separate program on this computer</li>
            <li>Your files still never leave this computer</li>
          </ul>
          <p className="m-0 text-xs text-on-surface-muted">FFmpeg is licensed under the LGPL. <button className="underline" onClick={() => setExplain(!explain)}>What does that mean?</button></p>
          {explain && <p className="m-0 rounded-card bg-surface-sunken p-3 text-xs text-on-surface-muted">FFmpeg is made by the FFmpeg project, not by {brand.name}. The build {brand.name} downloads leaves out every part that isn't under the LGPL licence. Its exact build settings and source code are published at {brand.urls.libraries_repo.replace("https://", "")}.</p>}
          <div className="flex flex-wrap items-center gap-2">
            <button className="smg-btn smg-btn--primary" onClick={install} data-testid="ffmpeg-download">Download FFmpeg</button>
            <button className="smg-btn smg-btn--ghost" onClick={close}>Not now</button>
          </div>
          <p className="m-0 text-xs text-on-surface-muted">Already have FFmpeg? <button className="underline" onClick={async () => { await host.ffmpegSetup?.useOwnCopy(); await refreshCaps(); close(); }}>Use my own copy…</button></p>
        </div>
      )}
      {progress && progress.phase === "downloading" && (
        <div className="flex flex-col gap-2">
          <p className="m-0">Downloading FFmpeg… {progress.totalBytes ? `${mb(progress.downloadedBytes ?? 0)} of ${mb(progress.totalBytes)}` : ""}</p>
          <div className="smg-progress"><div style={{ width: `${progress.totalBytes ? Math.round(((progress.downloadedBytes ?? 0) / progress.totalBytes) * 100) : 0}%` }} /></div>
          <button className="smg-btn smg-btn--ghost smg-btn--sm self-start" onClick={async () => { await host.ffmpegSetup?.cancel(); setProgress(null); }}>Cancel</button>
        </div>
      )}
      {progress && progress.phase === "checking" && <p className="m-0 flex items-center gap-2"><Spinner /> Checking the download…</p>}
      {progress && progress.phase === "testing" && <p className="m-0 flex items-center gap-2"><Spinner /> Testing video encoders…</p>}
      {progress && progress.phase === "ready" && <p className="m-0 text-success">Video support is ready.</p>}
      {progress && progress.phase === "error" && (
        <div className="flex flex-col gap-3">
          <p className="m-0 text-danger" role="alert">{errorText(progress.message)}</p>
          <div className="flex gap-2">
            <button className="smg-btn smg-btn--primary" onClick={install}>Try again</button>
            <button className="smg-btn smg-btn--secondary" onClick={async () => { await host.ffmpegSetup?.useOwnCopy(); await refreshCaps(); close(); }}>Use my own copy…</button>
          </div>
        </div>
      )}
    </Modal>
  );
}

export function Toast({ message }: { message: string }) {
  return <div className="smg-fade-in fixed bottom-5 left-1/2 z-50 -translate-x-1/2 rounded-pill border border-default bg-surface-overlay px-4 py-2 shadow-2" role="status">{message}</div>;
}
