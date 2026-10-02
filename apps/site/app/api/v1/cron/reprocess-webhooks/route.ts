import type { CronResponse } from "@cia/api-types";

import { webhookDeadLetterEmail } from "@/emails/webhook-dead-letter";
import { requireCron, route } from "@/lib/api";
import { sendOnce } from "@/lib/email";
import { json } from "@/lib/http";
import { processEvent } from "@/lib/webhook";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const MAX_ATTEMPTS = 20;
export const STUCK_AFTER_MS = 2 * 60 * 1000;

export const GET = route({}, async ({ req, rt }) => {
  requireCron(rt, req);
  const now = rt.now();
  const rows = await rt.db.listEventsToReprocess(new Date(now.getTime() - STUCK_AFTER_MS).toISOString(), MAX_ATTEMPTS, 50);
  const counts = { reprocessed: 0, failed: 0, dead_lettered: 0 };
  for (const row of rows) {
    try {
      const outcome = await processEvent(rt, row);
      await rt.db.updateEvent(row.event_id, { status: outcome, processed_at: rt.now().toISOString(), last_error: null, attempts: row.attempts + 1 });
      counts.reprocessed++;
    } catch (err) {
      const message = err instanceof Error ? `${err.name}: ${err.message}` : String(err);
      const attempts = row.attempts + 1;
      await rt.db.updateEvent(row.event_id, { status: "error", attempts, last_error: message.slice(0, 2000) });
      counts.failed++;
      if (attempts >= MAX_ATTEMPTS) {
        await sendOnce(rt, "webhook_dead_letter", row.event_id, rt.env.ADMIN_ALERT_EMAIL, webhookDeadLetterEmail({ eventId: row.event_id, eventType: row.event_type, attempts, lastError: message }));
        counts.dead_lettered++;
      }
    }
  }
  const res: CronResponse = { ok: true, counts };
  return json(res);
});
