// Paddle webhook endpoint (DESIGN.md 5.8.1). Raw body first, signature by hand, event-id
// idempotency, 3.5 s budget, 500 on any error so Paddle retries.

import { toApiError } from "@/lib/api";
import { json } from "@/lib/http";
import { verifyPaddleSignature } from "@/lib/paddle-signature";
import { getRuntime } from "@/lib/runtime";
import { parseEnvelope, processEvent } from "@/lib/webhook";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const PROCESSING_BUDGET_MS = 3500;

export async function POST(req: Request): Promise<Response> {
  const rawBody = await req.text();
  let rt;
  try {
    rt = await getRuntime();
  } catch (err) {
    console.error("[webhook] runtime", err);
    return json({ error: "internal", message: "not configured" }, { status: 500 });
  }
  const nowS = Math.floor(rt.now().getTime() / 1000);
  const sig = verifyPaddleSignature(req.headers.get("paddle-signature"), rawBody, rt.env.PADDLE_WEBHOOK_SECRET, nowS);
  if (sig !== "ok") return new Response(null, { status: 401 });

  let parsed: unknown;
  try {
    parsed = JSON.parse(rawBody);
  } catch {
    return json({ error: "bad_request", message: "not JSON" }, { status: 400 });
  }
  const envelope = parseEnvelope(parsed);
  if (!envelope) return json({ error: "bad_request", message: "not a Paddle event" }, { status: 400 });

  const { inserted, row } = await rt.db.insertEventIfAbsent({ event_id: envelope.event_id, event_type: envelope.event_type, occurred_at: envelope.occurred_at, payload: parsed });
  if (!inserted && (row.status === "processed" || row.status === "ignored")) {
    return json({ ok: true, duplicate: true });
  }

  try {
    const outcome = await withBudget(processEvent(rt, row), PROCESSING_BUDGET_MS);
    await rt.db.updateEvent(row.event_id, { status: outcome, processed_at: rt.now().toISOString(), last_error: null, attempts: row.attempts + 1 });
    return json({ ok: true, status: outcome });
  } catch (err) {
    const message = err instanceof Error ? `${err.name}: ${err.message}` : String(err);
    console.error("[webhook]", envelope.event_type, envelope.event_id, message);
    try {
      await rt.db.updateEvent(row.event_id, { status: "error", attempts: row.attempts + 1, last_error: message.slice(0, 2000) });
    } catch (e2) {
      console.error("[webhook] could not record error", e2);
    }
    const api = toApiError(err);
    return json({ error: "internal", message: api.code }, { status: 500 });
  }
}

function withBudget<T>(p: Promise<T>, ms: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`processing exceeded ${ms} ms`)), ms);
  });
  return Promise.race([p, timeout]).finally(() => clearTimeout(timer));
}
