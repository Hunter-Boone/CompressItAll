import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { AllowanceView, Capabilities, EngineHost, LicenseInfo } from "@cia/engine-client";
import { defaultSettings, mergeSettings, type Settings } from "../lib/settings";
import { applyTheme, watchSystemTheme } from "../lib/theme";
import { applyUiScale } from "../lib/uiScale";

interface HostCtx {
  host: EngineHost;
  caps: Capabilities | null;
  refreshCaps(): Promise<void>;
  settings: Settings;
  updateSettings(patch: Partial<Settings> | ((s: Settings) => Settings)): void;
  license: LicenseInfo;
  refreshLicense(): Promise<void>;
  allowance: AllowanceView;
  refreshAllowance(): Promise<void>;
  ready: boolean;
}

const Ctx = createContext<HostCtx | null>(null);

export function HostProvider({ host, children }: { host: EngineHost; children: ReactNode }) {
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [settings, setSettings] = useState<Settings>(defaultSettings());
  const [license, setLicense] = useState<LicenseInfo>({ status: "free" });
  const [allowance, setAllowance] = useState<AllowanceView>({ remaining: 3, nextFreeAtMs: null });
  const [ready, setReady] = useState(false);
  const saveTimer = useRef<number | null>(null);

  const refreshCaps = useCallback(async () => setCaps(await host.capabilities()), [host]);
  const refreshLicense = useCallback(async () => setLicense(await host.license()), [host]);
  const refreshAllowance = useCallback(async () => setAllowance(await host.allowance()), [host]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [c, s] = await Promise.all([host.capabilities(), host.loadSettings()]);
      if (cancelled) return;
      const merged = mergeSettings(s);
      if (!merged.installId) merged.installId = crypto.randomUUID();
      setCaps(c);
      setSettings(merged);
      applyTheme(merged.theme);
      applyUiScale(merged.textSize);
      setReady(true);
      // License and allowance never block first paint.
      host.license().then((l) => !cancelled && setLicense(l)).catch(() => {});
      host.allowance().then((a) => !cancelled && setAllowance(a)).catch(() => {});
    })();
    return () => {
      cancelled = true;
    };
  }, [host]);

  useEffect(() => watchSystemTheme(() => applyTheme(settings.theme)), [settings.theme]);

  const updateSettings = useCallback(
    (patch: Partial<Settings> | ((s: Settings) => Settings)) => {
      setSettings((prev) => {
        const next = typeof patch === "function" ? patch(prev) : { ...prev, ...patch };
        applyTheme(next.theme);
        applyUiScale(next.textSize);
        if (saveTimer.current) window.clearTimeout(saveTimer.current);
        saveTimer.current = window.setTimeout(() => void host.saveSettings(next as unknown as Record<string, unknown>), 150);
        return next;
      });
    },
    [host],
  );

  const value = useMemo<HostCtx>(
    () => ({ host, caps, refreshCaps, settings, updateSettings, license, refreshLicense, allowance, refreshAllowance, ready }),
    [host, caps, refreshCaps, settings, updateSettings, license, refreshLicense, allowance, refreshAllowance, ready],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useHost(): HostCtx {
  const v = useContext(Ctx);
  if (!v) throw new Error("useHost outside HostProvider");
  return v;
}
