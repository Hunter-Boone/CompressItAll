import type { EmailTemplate } from "../lib/email";
import { esc, layout, para } from "./layout";

export function webhookDeadLetterEmail(args: { eventId: string; eventType: string; attempts: number; lastError: string | null }): EmailTemplate {
  const title = `Paddle webhook ${args.eventId} needs a look`;
  const body =
    para(`Event <code>${esc(args.eventId)}</code> (${esc(args.eventType)}) failed ${args.attempts} times and will not be retried.`) +
    para(`Last error: <code>${esc(args.lastError ?? "none recorded")}</code>`) +
    para(`Fix the cause, then set the row's status back to <code>error</code> and attempts to 0 in paddle_events to retry.`);
  const text = `Event ${args.eventId} (${args.eventType}) failed ${args.attempts} times and will not be retried.
Last error: ${args.lastError ?? "none recorded"}
Fix the cause, then set the row's status back to 'error' and attempts to 0 in paddle_events to retry.`;
  return { subject: title, ...layout({ title, body, text }) };
}
