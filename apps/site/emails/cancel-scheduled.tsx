import { brand } from "../lib/brand";
import type { EmailTemplate } from "../lib/email";
import { esc, layout, para } from "./layout";

export function cancelScheduledEmail(args: { endsOn: string; siteUrl: string }): EmailTemplate {
  const title = `Your ${brand.pro_name} plan won't renew`;
  const body =
    para(`Your yearly plan is set to end on <strong>${esc(args.endsOn)}</strong>. ${esc(brand.pro_name)} stays on until then.`) +
    para(`After that, ${esc(brand.name)} goes back to the free version: 3 files a day, every feature, nothing deleted.`) +
    para(`Changed your mind? Open <a href="${esc(args.siteUrl)}/account" style="color:${brand.accent};">your account</a> and choose Manage billing.`);
  const text = `Your yearly plan is set to end on ${args.endsOn}. ${brand.pro_name} stays on until then.

After that, ${brand.name} goes back to the free version: 3 files a day, every feature, nothing deleted.

Changed your mind? Open ${args.siteUrl}/account and choose Manage billing.`;
  return { subject: title, ...layout({ title, body, text }) };
}
