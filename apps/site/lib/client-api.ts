// Tiny fetch wrapper for the site's own client components. Same-origin, cookies included.

export interface ApiFailure {
  error: string;
  message: string;
  [k: string]: unknown;
}

export class ClientApiError extends Error {
  constructor(public readonly status: number, public readonly body: ApiFailure) {
    super(body.message);
    this.name = "ClientApiError";
  }
}

export async function api<T>(method: "GET" | "POST", path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api/v1${path}`, {
    method,
    credentials: "same-origin",
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {}
  if (!res.ok) {
    const failure = (parsed as ApiFailure | null) ?? { error: "internal", message: "Something went wrong. Try again in a minute." };
    throw new ClientApiError(res.status, failure);
  }
  return parsed as T;
}

export function formatDate(iso: string | null | undefined): string {
  if (!iso) return "";
  return new Date(iso).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

export function relativeTime(iso: string, now = Date.now()): string {
  const diff = now - new Date(iso).getTime();
  const m = Math.round(diff / 60000);
  if (m < 2) return "just now";
  if (m < 60) return `${m} minutes ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} hours ago`;
  const d = Math.round(h / 24);
  if (d < 60) return `${d} days ago`;
  return formatDate(iso);
}
