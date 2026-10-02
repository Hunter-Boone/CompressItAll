/**
 * Smidge theme tokens. Same key list as ConvertSave's ThemeTokenSet so UI code
 * ports across; values are Smidge's own (DESIGN.md 4.1). `index.css` must match
 * this map; a unit test compares them.
 */
export type ThemeId = "light" | "dark";

export type ThemeTokenSet = {
  "surface-base": string;
  "surface-card": string;
  "surface-overlay": string;
  "surface-sunken": string;
  "surface-hover": string;
  "surface-recessed": string;
  "surface-recessed-hover": string;
  "surface-elevated": string;
  "card-surface-2": string;
  "surface-1": string;
  "surface-2": string;
  "surface-3": string;
  "surface-4": string;
  "surface-5": string;
  "surface-1-hover": string;
  "surface-2-hover": string;
  "surface-3-hover": string;
  "on-surface": string;
  "on-surface-muted": string;
  "on-surface-subtle": string;
  "on-surface-disabled": string;
  primary: string;
  "on-primary": string;
  "primary-container": string;
  "on-primary-container": string;
  secondary: string;
  "on-secondary": string;
  "secondary-container": string;
  "on-secondary-container": string;
  accent: string;
  "on-accent": string;
  "accent-container": string;
  "on-accent-container": string;
  success: string;
  "on-success": string;
  "success-container": string;
  "on-success-container": string;
  "success-border": string;
  warning: string;
  "on-warning": string;
  "warning-container": string;
  "on-warning-container": string;
  danger: string;
  "on-danger": string;
  "danger-container": string;
  "on-danger-container": string;
  "danger-border": string;
  info: string;
  "on-info": string;
  "info-container": string;
  "on-info-container": string;
  "border-subtle": string;
  "border-card-subtle": string;
  "border-default": string;
  "border-strong": string;
  "focus-ring": string;
  "form-accent": string;
  "primary-hover": string;
  "primary-pressed": string;
  "primary-disabled": string;
  "secondary-hover": string;
  "secondary-pressed": string;
  "secondary-disabled": string;
  "danger-hover": string;
  "danger-pressed": string;
  "danger-disabled": string;
  scrim: string;
  "shadow-1": string;
  "shadow-2": string;
  "shadow-3": string;
  "shadow-subtle": string;
};

const VIOLET = "#6D4AFF";
const VIOLET_HOVER = "#7F60FF";
const VIOLET_PRESSED = "#5A37EE";

export const themeTokens: Record<ThemeId, ThemeTokenSet> = {
  dark: {
    "surface-base": "#14121B",
    "surface-card": "#1D1A27",
    "surface-overlay": "#272336",
    "surface-sunken": "#100E16",
    "surface-hover": "#272336",
    "surface-recessed": "#100E16",
    "surface-recessed-hover": "#1D1A27",
    "surface-elevated": "#1D1A27",
    "card-surface-2": "#191622",
    "surface-1": "#14121B",
    "surface-2": "#191622",
    "surface-3": "#1D1A27",
    "surface-4": "#272336",
    "surface-5": "#272336",
    "surface-1-hover": "#1D1A27",
    "surface-2-hover": "#272336",
    "surface-3-hover": "#272336",
    "on-surface": "#ECEAF4",
    "on-surface-muted": "#B1ACC4",
    "on-surface-subtle": "#77718C",
    "on-surface-disabled": "#4A4566",
    primary: VIOLET,
    "on-primary": "#FFFFFF",
    "primary-container": "#2A2150",
    "on-primary-container": "#DCD4FF",
    secondary: "#272336",
    "on-secondary": "#ECEAF4",
    "secondary-container": "#272336",
    "on-secondary-container": "#ECEAF4",
    accent: VIOLET,
    "on-accent": "#FFFFFF",
    "accent-container": "#2A2150",
    "on-accent-container": "#DCD4FF",
    success: "#2FBF71",
    "on-success": "#06200F",
    "success-container": "#123826",
    "on-success-container": "#BFF0D4",
    "success-border": "#2FBF71",
    warning: "#F2A93B",
    "on-warning": "#2A1A00",
    "warning-container": "#3A2A10",
    "on-warning-container": "#FFE2B0",
    danger: "#F0566A",
    "on-danger": "#FFFFFF",
    "danger-container": "#3D1820",
    "on-danger-container": "#FFD0D6",
    "danger-border": "#F0566A",
    info: VIOLET,
    "on-info": "#FFFFFF",
    "info-container": "#2A2150",
    "on-info-container": "#DCD4FF",
    "border-subtle": "#262233",
    "border-card-subtle": "#221E2E",
    "border-default": "#34304A",
    "border-strong": "#4A4566",
    "focus-ring": VIOLET,
    "form-accent": VIOLET,
    "primary-hover": VIOLET_HOVER,
    "primary-pressed": VIOLET_PRESSED,
    "primary-disabled": "#3A3552",
    "secondary-hover": "#312C44",
    "secondary-pressed": "#3A3552",
    "secondary-disabled": "#1D1A27",
    "danger-hover": "#F7778A",
    "danger-pressed": "#D23E52",
    "danger-disabled": "#4A2A32",
    scrim: "rgba(8, 6, 14, 0.62)",
    "shadow-subtle": "0 1px 2px rgba(0, 0, 0, 0.5), 0 2px 6px rgba(0, 0, 0, 0.35)",
    "shadow-1": "0 1px 2px rgba(0, 0, 0, 0.5), 0 2px 6px rgba(0, 0, 0, 0.35)",
    "shadow-2": "0 3px 14px rgba(0, 0, 0, 0.6), 0 1px 6px rgba(0, 0, 0, 0.45)",
    "shadow-3": "0 8px 28px rgba(0, 0, 0, 0.65), 0 2px 8px rgba(0, 0, 0, 0.5)",
  },
  light: {
    "surface-base": "#F6F4F0",
    "surface-card": "#FFFFFF",
    "surface-overlay": "#FFFFFF",
    "surface-sunken": "#EEEBE6",
    "surface-hover": "#F0EDE8",
    "surface-recessed": "#EEEBE6",
    "surface-recessed-hover": "#E6E2DC",
    "surface-elevated": "#FFFFFF",
    "card-surface-2": "#FBFAF7",
    "surface-1": "#F6F4F0",
    "surface-2": "#FBFAF7",
    "surface-3": "#FFFFFF",
    "surface-4": "#F0EDE8",
    "surface-5": "#EEEBE6",
    "surface-1-hover": "#F0EDE8",
    "surface-2-hover": "#F0EDE8",
    "surface-3-hover": "#F6F4F0",
    "on-surface": "#1C1924",
    "on-surface-muted": "#5C5768",
    "on-surface-subtle": "#8A8496",
    "on-surface-disabled": "#C4BEB6",
    primary: VIOLET,
    "on-primary": "#FFFFFF",
    "primary-container": "#EEEAFF",
    "on-primary-container": "#2B1C7A",
    secondary: "#F0EDE8",
    "on-secondary": "#1C1924",
    "secondary-container": "#F0EDE8",
    "on-secondary-container": "#1C1924",
    accent: VIOLET,
    "on-accent": "#FFFFFF",
    "accent-container": "#EEEAFF",
    "on-accent-container": "#2B1C7A",
    success: "#1E9E5A",
    "on-success": "#FFFFFF",
    "success-container": "#DDF5E8",
    "on-success-container": "#0D4A2A",
    "success-border": "#1E9E5A",
    warning: "#B86E00",
    "on-warning": "#FFFFFF",
    "warning-container": "#FFF1D6",
    "on-warning-container": "#5A3600",
    danger: "#D2304A",
    "on-danger": "#FFFFFF",
    "danger-container": "#FBE0E5",
    "on-danger-container": "#6E1324",
    "danger-border": "#D2304A",
    info: VIOLET,
    "on-info": "#FFFFFF",
    "info-container": "#EEEAFF",
    "on-info-container": "#2B1C7A",
    "border-subtle": "#ECE8E2",
    "border-card-subtle": "#E6E2DC",
    "border-default": "#D6D0C8",
    "border-strong": "#BDB5AB",
    "focus-ring": VIOLET,
    "form-accent": VIOLET,
    "primary-hover": VIOLET_HOVER,
    "primary-pressed": VIOLET_PRESSED,
    "primary-disabled": "#C9C2E8",
    "secondary-hover": "#E6E2DC",
    "secondary-pressed": "#D6D0C8",
    "secondary-disabled": "#F6F4F0",
    "danger-hover": "#E04A62",
    "danger-pressed": "#B0203A",
    "danger-disabled": "#EAB9C2",
    scrim: "rgba(28, 25, 36, 0.32)",
    "shadow-subtle": "0 1px 2px rgba(28, 25, 36, 0.04), 0 8px 20px rgba(28, 25, 36, 0.06)",
    "shadow-1": "0 1px 2px rgba(28, 25, 36, 0.04), 0 8px 20px rgba(28, 25, 36, 0.06)",
    "shadow-2": "0 1px 3px rgba(28, 25, 36, 0.06), 0 16px 36px rgba(28, 25, 36, 0.10)",
    "shadow-3": "0 2px 6px rgba(28, 25, 36, 0.08), 0 24px 48px rgba(28, 25, 36, 0.14)",
  },
};

/** Render one theme as CSS custom properties (`--surface-base: …;`). */
export function themeToCss(theme: ThemeId): string {
  return Object.entries(themeTokens[theme])
    .map(([k, v]) => `  --${k}: ${v};`)
    .join("\n");
}
