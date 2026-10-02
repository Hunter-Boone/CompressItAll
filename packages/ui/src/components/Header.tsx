import { CircleHelp, Settings as SettingsIcon } from "lucide-react";
import brand from "@cia/brand";
import { useHost } from "../host/HostContext";
import { useStore } from "../state/store";

export function Wordmark({ size = 22 }: { size?: number }) {
  return (
    <span className="inline-flex items-center gap-2 select-none">
      <span className="relative inline-block rounded-md bg-primary" style={{ width: size, height: size }} aria-hidden>
        <span className="absolute rounded-sm bg-white" style={{ width: size * 0.32, height: size * 0.32, right: size * 0.18, bottom: size * 0.18 }} />
      </span>
      <span className="font-display font-bold tracking-tight" style={{ fontSize: size * 0.95 }}>{brand.wordmark}</span>
    </span>
  );
}

export default function Header() {
  const { license, allowance } = useHost();
  const { dispatch } = useStore();
  const free = license.status !== "pro";
  return (
    <header className="flex h-14 shrink-0 items-center justify-between px-5" data-tour="header">
      <Wordmark />
      <div className="flex items-center gap-2">
        {free && (
          <button className="rounded-pill border border-default bg-surface-card px-3 py-1 text-xs font-medium text-on-surface-muted hover:border-strong hover:text-on-surface" onClick={() => dispatch({ type: "modal", modal: "upgrade" })} data-testid="allowance-pill">
            {allowance.remaining === 1 ? "1 free file left today" : `${allowance.remaining} free files left today`}
          </button>
        )}
        <button className="smg-btn smg-btn--ghost smg-btn--sm !px-2" aria-label="Help" onClick={() => dispatch({ type: "modal", modal: "help" })}><CircleHelp size={18} /></button>
        <button className="smg-btn smg-btn--ghost smg-btn--sm !px-2" aria-label="Settings" onClick={() => dispatch({ type: "modal", modal: "settings" })} data-testid="settings-button"><SettingsIcon size={18} /></button>
      </div>
    </header>
  );
}
