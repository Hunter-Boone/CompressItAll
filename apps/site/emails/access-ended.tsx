import { brand } from "../lib/brand";
import type { EmailTemplate } from "../lib/email";
import { esc, layout, para } from "./layout";

export function accessEndedEmail(args: { endedOn: string; siteUrl: string }): EmailTemplate {
  const title = `Your ${brand.pro_name} yearly plan has ended`;
  const body =
    para(`Your yearly plan ended on <strong>${esc(args.endedOn)}</strong>. ${esc(brand.name)} is back on the free version: 3 files a day, every feature.`) +
    para(`To renew, open <a href="${esc(args.siteUrl)}/buy" style="color:${brand.accent};">${esc(args.siteUrl)}/buy</a>. A new purchase comes with a new key.`);
  const text = `Your yearly plan ended on ${args.endedOn}. ${brand.name} is back on the free version: 3 files a day, every feature.

To renew, open ${args.siteUrl}/buy. A new purchase comes with a new key.`;
  return { subject: title, ...layout({ title, body, text }) };
}
