/**
 * Ported from ConvertSave: the factor getBoundingClientRect() carries relative
 * to layout px under the root `zoom` text-size setting. Chromium/WebView2
 * pre-multiplies client rects by the zoom while fixed placement resolves in
 * layout px; WKWebView already reports layout px (ratio 1). Measured, not assumed.
 */
export function measuredZoomRatio(): number {
  const root = document.documentElement;
  if (root.offsetWidth <= 0) return 1;
  return root.getBoundingClientRect().width / root.offsetWidth || 1;
}
