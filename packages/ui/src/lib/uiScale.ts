/** Text size as root `zoom` (ConvertSave's approach). Persisted in settings by the host. */
export const UI_SCALES = [
  { id: "default", label: "100%", value: 1 },
  { id: "large", label: "115%", value: 1.15 },
  { id: "xlarge", label: "130%", value: 1.3 },
] as const;
export type UiScaleId = (typeof UI_SCALES)[number]["id"];

export function applyUiScale(id: string): void {
  const scale = UI_SCALES.find((s) => s.id === id) ?? UI_SCALES[0];
  (document.documentElement.style as CSSStyleDeclaration & { zoom: string }).zoom = scale.value === 1 ? "" : String(scale.value);
}
