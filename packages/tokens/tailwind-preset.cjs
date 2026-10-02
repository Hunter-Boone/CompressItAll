/** Tailwind preset: colours map to CSS variables from theme.css (ConvertSave's approach). */
const v = (name) => `var(--${name})`;
module.exports = {
  darkMode: ["class", ".theme-dark"],
  theme: {
    extend: {
      colors: {
        background: v("surface-base"),
        surface: { DEFAULT: v("surface-base"), base: v("surface-base"), card: v("surface-card"), overlay: v("surface-overlay"), sunken: v("surface-sunken"), hover: v("surface-hover"), recessed: v("surface-recessed"), elevated: v("surface-elevated"), 1: v("surface-1"), 2: v("surface-2"), 3: v("surface-3"), 4: v("surface-4"), 5: v("surface-5") },
        border: { DEFAULT: v("border-subtle"), subtle: v("border-subtle"), edge: v("border-default"), strong: v("border-strong") },
        "on-surface": { DEFAULT: v("on-surface"), muted: v("on-surface-muted"), subtle: v("on-surface-subtle"), disabled: v("on-surface-disabled") },
        primary: { DEFAULT: v("primary"), hover: v("primary-hover"), pressed: v("primary-pressed"), disabled: v("primary-disabled"), foreground: v("on-primary"), container: v("primary-container"), "container-foreground": v("on-primary-container") },
        secondary: { DEFAULT: v("secondary"), hover: v("secondary-hover"), pressed: v("secondary-pressed"), disabled: v("secondary-disabled"), foreground: v("on-secondary"), container: v("secondary-container"), "container-foreground": v("on-secondary-container") },
        accent: { DEFAULT: v("accent"), foreground: v("on-accent"), container: v("accent-container"), "container-foreground": v("on-accent-container") },
        success: { DEFAULT: v("success"), foreground: v("on-success"), container: v("success-container"), "container-foreground": v("on-success-container"), border: v("success-border") },
        warning: { DEFAULT: v("warning"), foreground: v("on-warning"), container: v("warning-container"), "container-foreground": v("on-warning-container") },
        danger: { DEFAULT: v("danger"), hover: v("danger-hover"), pressed: v("danger-pressed"), disabled: v("danger-disabled"), foreground: v("on-danger"), container: v("danger-container"), "container-foreground": v("on-danger-container"), border: v("danger-border") },
        info: { DEFAULT: v("info"), foreground: v("on-info"), container: v("info-container"), "container-foreground": v("on-info-container") },
        scrim: v("scrim"),
      },
      borderColor: { DEFAULT: v("border-subtle"), subtle: v("border-subtle"), edge: v("border-default"), strong: v("border-strong"), primary: v("primary"), "danger-border": v("danger-border"), "success-border": v("success-border") },
      ringColor: { DEFAULT: v("focus-ring"), primary: v("primary") },
      boxShadow: { subtle: v("shadow-subtle"), 1: v("shadow-1"), 2: v("shadow-2"), 3: v("shadow-3") },
      fontFamily: {
        sans: ["Inter", "ui-sans-serif", "system-ui", "-apple-system", "Segoe UI", "sans-serif"],
        display: ["Bricolage Grotesque Variable", "Bricolage Grotesque", "Inter", "ui-sans-serif", "system-ui", "sans-serif"],
      },
      fontSize: {
        "type-1": ["1.875rem", { lineHeight: "2.25rem", fontWeight: "700" }],
        "type-2": ["1.2rem", { lineHeight: "1.75rem", fontWeight: "700" }],
        "type-3": ["0.875rem", { lineHeight: "1.375rem", fontWeight: "500" }],
        xs: ["0.8125rem", { lineHeight: "1.25rem" }],
        sm: ["0.875rem", { lineHeight: "1.375rem" }],
        base: ["1rem", { lineHeight: "1.5rem" }],
        lg: ["1.2rem", { lineHeight: "1.75rem" }],
      },
      borderRadius: { control: "10px", panel: "20px", card: "14px", pill: "999px" },
      transitionTimingFunction: { standard: "cubic-bezier(0.2, 0.8, 0.2, 1)", emphasized: "cubic-bezier(0.2, 0, 0, 1)" },
      transitionDuration: { fast: "170ms", standard: "240ms", slow: "320ms" },
    },
  },
};
