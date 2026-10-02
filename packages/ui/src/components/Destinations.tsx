import { Briefcase, ChevronDown, Mail, MessageCircle, MessageSquare, Minimize2, Ruler, Send, Slack, Users, Video } from "lucide-react";
import { clsx } from "clsx";
import { useEffect, useRef, useState } from "react";
import { presetGroups, presetById, type Preset, type PresetGroup } from "@cia/engine-client";
import { useHost } from "../host/HostContext";
import { useStore, type Destination } from "../state/store";

function icon(name: string) {
  const s = 18;
  switch (name) {
    case "discord": return <MessageSquare size={s} />;
    case "mail": return <Mail size={s} />;
    case "briefcase": return <Briefcase size={s} />;
    case "whatsapp": return <MessageCircle size={s} />;
    case "message": return <MessageCircle size={s} />;
    case "slack": return <Slack size={s} />;
    case "teams": return <Users size={s} />;
    case "telegram": return <Send size={s} />;
    case "ruler": return <Ruler size={s} />;
    case "shrink": return <Minimize2 size={s} />;
    default: return <Video size={s} />;
  }
}

function TierMenu({ group, current, onPick, onClose }: { group: PresetGroup; current: Preset; onPick: (p: Preset) => void; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onDoc = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) onClose(); };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("mousedown", onDoc);
    window.addEventListener("keydown", onKey);
    return () => { document.removeEventListener("mousedown", onDoc); window.removeEventListener("keydown", onKey); };
  }, [onClose]);
  return (
    <div ref={ref} role="menu" className="smg-fade-in absolute left-0 top-full z-30 mt-1 min-w-[260px] rounded-card border border-default bg-surface-overlay p-1 shadow-2" data-testid="tier-menu">
      {group.tiers.map((t) => (
        <button key={t.id} role="menuitemradio" aria-checked={t.id === current.id} className={clsx("flex w-full items-center justify-between gap-3 rounded-control px-3 py-2 text-left hover:bg-surface-hover", t.id === current.id && "bg-primary-container text-primary-container-foreground")} onClick={() => { onPick(t); onClose(); }}>
          <span>{t.tier_label}</span>
          <span className="text-xs text-on-surface-muted">{t.tier_summary}</span>
        </button>
      ))}
    </div>
  );
}

export function destinationFromPreset(p: Preset, s: { customBytes: number; customPerMessage: boolean }): Destination {
  if (p.mode === "smaller") return { type: "smaller" };
  if (p.mode === "custom") return { type: "custom", bytes: s.customBytes, perMessage: s.customPerMessage };
  return { type: "preset", presetId: p.id };
}

export default function Destinations() {
  const { settings, updateSettings } = useHost();
  const { state, dispatch } = useStore();
  const [menu, setMenu] = useState<string | null>(null);
  const groups = presetGroups();
  const locked = state.phase === "running";
  const dest = state.destination;

  const currentTier = (g: PresetGroup): Preset => presetById(settings.tierByGroup[g.group] ?? "") ?? g.defaultTier;
  const selectedGroup = dest?.type === "preset" ? presetById(dest.presetId)?.group : dest?.type === "custom" ? "custom" : dest?.type === "smaller" ? "smaller" : null;

  const pick = (p: Preset) => {
    updateSettings((s) => ({ ...s, tierByGroup: { ...s.tierByGroup, [p.group]: p.id }, lastPresetId: p.id }));
    dispatch({ type: "set_destination", destination: destinationFromPreset(p, settings) });
  };

  const customUnits = [{ v: 1_000, l: "KB" }, { v: 1_000_000, l: "MB" }, { v: 1_000_000_000, l: "GB" }];
  const [unit, setUnit] = useState(1_000_000);
  const customValue = dest?.type === "custom" ? dest.bytes : settings.customBytes;
  const setCustom = (bytes: number, perMessage: boolean) => {
    if (bytes <= 0) return;
    updateSettings({ customBytes: bytes, customPerMessage: perMessage });
    dispatch({ type: "set_destination", destination: { type: "custom", bytes, perMessage } });
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap gap-2" data-tour="destinations" data-testid="destinations">
        {groups.filter((g) => g.mode === "fit").map((g) => {
          const tier = currentTier(g);
          const selected = selectedGroup === g.group;
          const hasTiers = g.tiers.length > 1;
          return (
            <div key={g.group} className="relative">
              <div className={clsx("smg-tile min-w-[150px] !flex-row !items-center !gap-2 !py-2", selected && "smg-tile--selected", locked && "pointer-events-none opacity-60")}>
                <button className="flex flex-1 items-center gap-2 text-left" onClick={() => pick(tier)} aria-pressed={selected} data-testid={`tile-${g.group}`}>
                  <span className={clsx(selected ? "text-primary" : "text-on-surface-muted")}>{icon(g.icon)}</span>
                  <span className="flex flex-col leading-tight">
                    <span className="font-medium">{g.tileLabel}</span>
                    <span className="text-[11px] text-on-surface-muted">{tier.tier_label ? `${tier.tier_label} · ${tier.tier_summary ?? ""}` : tier.tier_summary}</span>
                  </span>
                </button>
                {hasTiers && <button className="-mr-1 rounded p-1 text-on-surface-muted hover:bg-surface-hover" aria-label={`${g.tileLabel} options`} aria-haspopup="menu" onClick={() => setMenu(menu === g.group ? null : g.group)}><ChevronDown size={14} /></button>}
              </div>
              {menu === g.group && <TierMenu group={g} current={tier} onPick={pick} onClose={() => setMenu(null)} />}
            </div>
          );
        })}
        {groups.filter((g) => g.mode === "custom").map((g) => (
          <button key={g.group} className={clsx("smg-tile min-w-[150px] !flex-row !items-center !gap-2 !py-2", selectedGroup === "custom" && "smg-tile--selected", locked && "pointer-events-none opacity-60")} onClick={() => pick(g.defaultTier)} aria-pressed={selectedGroup === "custom"} data-testid="tile-custom">
            <span className={clsx(selectedGroup === "custom" ? "text-primary" : "text-on-surface-muted")}>{icon(g.icon)}</span>
            <span className="flex flex-col leading-tight"><span className="font-medium">{g.tileLabel}</span><span className="text-[11px] text-on-surface-muted">Your own limit</span></span>
          </button>
        ))}
        <span className="flex-1" />
        {groups.filter((g) => g.mode === "smaller").map((g) => (
          <button key={g.group} data-tour="smaller" className={clsx("smg-tile min-w-[190px] !flex-row !items-center !gap-2 !py-2", selectedGroup === "smaller" && "smg-tile--selected", locked && "pointer-events-none opacity-60")} onClick={() => pick(g.defaultTier)} aria-pressed={selectedGroup === "smaller"} data-testid="tile-smaller">
            <span className={clsx(selectedGroup === "smaller" ? "text-primary" : "text-on-surface-muted")}>{icon(g.icon)}</span>
            <span className="flex flex-col leading-tight"><span className="font-medium">{g.tileLabel}</span><span className="text-[11px] text-on-surface-muted">No size limit</span></span>
          </button>
        ))}
      </div>
      {selectedGroup === "custom" && (
        <div className="smg-fade-in flex flex-wrap items-center gap-2 rounded-card border border-subtle bg-surface-card px-3 py-2" data-testid="custom-size">
          <span className="text-on-surface-muted">Limit</span>
          <input type="number" min={1} step="any" className="smg-input h-9 w-28" value={Math.round((customValue / unit) * 100) / 100} onChange={(e) => setCustom(Math.round(parseFloat(e.target.value || "0") * unit), dest?.type === "custom" ? dest.perMessage : settings.customPerMessage)} aria-label="Custom size" />
          <select className="smg-input h-9" value={unit} onChange={(e) => setUnit(Number(e.target.value))} aria-label="Unit">{customUnits.map((u) => <option key={u.v} value={u.v}>{u.l}</option>)}</select>
          <select className="smg-input h-9" value={(dest?.type === "custom" ? dest.perMessage : settings.customPerMessage) ? "message" : "file"} onChange={(e) => setCustom(customValue, e.target.value === "message")} aria-label="Applies to">
            <option value="file">Each file</option>
            <option value="message">All files together</option>
          </select>
        </div>
      )}
    </div>
  );
}
