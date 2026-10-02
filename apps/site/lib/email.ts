// Email over Resend (DESIGN.md 5.11). Every send goes through `sendOnce`, which inserts into
// email_log first and skips when (kind, ref) already exists, so webhook retries never send twice.

import type { Runtime } from "./runtime";

export interface EmailTemplate {
  subject: string;
  html: string;
  text: string;
}

export type EmailKind = "purchase_key" | "otp" | "key_resend" | "cancel_scheduled" | "access_ended" | "refund_done" | "webhook_dead_letter";

export class EmailError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "EmailError";
  }
}

/** Returns true if an email was sent now, false if (kind, ref) had already been sent. */
export async function sendOnce(rt: Runtime, kind: EmailKind, ref: string, to: string, template: EmailTemplate): Promise<boolean> {
  const log = await rt.db.insertEmailLogIfAbsent(kind, ref, to);
  if (!log.inserted) return false;
  const id = await deliver(rt, to, template);
  await rt.db.setEmailLogResendId(log.id, id);
  return true;
}

/** Send through Resend's REST API; throws EmailError on failure. */
export async function deliver(rt: Runtime, to: string, template: EmailTemplate): Promise<string> {
  const res = await rt.fetch("https://api.resend.com/emails", {
    method: "POST",
    headers: { Authorization: `Bearer ${rt.env.RESEND_API_KEY}`, "Content-Type": "application/json" },
    body: JSON.stringify({
      from: rt.env.EMAIL_FROM,
      to: [to],
      reply_to: rt.env.EMAIL_REPLY_TO,
      subject: template.subject,
      html: template.html,
      text: template.text,
    }),
  });
  if (!res.ok) {
    let detail = "";
    try {
      detail = ((await res.json()) as { message?: string }).message ?? "";
    } catch {}
    throw new EmailError(`Resend answered ${res.status}${detail ? `: ${detail}` : ""}`);
  }
  const body = (await res.json()) as { id?: string };
  return body.id ?? "";
}
