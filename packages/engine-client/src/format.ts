/** Mirrors cia_core::format (decimal MB, one decimal place). */
export function mb(bytes: number | bigint): string {
  const b = Number(bytes);
  if (b >= 1_000_000_000) return `${(b / 1_000_000_000).toFixed(1)} GB`;
  if (b >= 1_000_000) return `${(b / 1_000_000).toFixed(1)} MB`;
  if (b >= 1_000) return `${Math.round(b / 1_000)} KB`;
  return `${b} bytes`;
}

export function mbWhole(bytes: number | bigint): string {
  const b = Number(bytes);
  if (b >= 1_000_000_000 && b % 100_000_000 === 0) return `${(b / 1_000_000_000).toFixed(1).replace(/\.0$/, "")} GB`;
  if (b >= 1_000_000) return `${Math.floor(b / 1_000_000)} MB`;
  return `${Math.floor(b / 1_000)} KB`;
}

export function duration(ms: number | bigint): string {
  const total = Math.floor((Number(ms) + 500) / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return m > 0 ? `${h} h ${m} min` : `${h} h`;
  if (m > 0) return s > 0 ? `${m} min ${s} s` : `${m} min`;
  return `${s} s`;
}

export function clockTime(unixMs: number): string {
  const d = new Date(unixMs);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export function sizeArrow(before: number | bigint, after: number | bigint): string {
  return `${mb(before)} → ${mb(after)}`;
}
