import type { Plan } from "@cia/api-types";

import { brand } from "../lib/brand";
import type { EmailTemplate } from "../lib/email";
import { button, esc, keyBlock, layout, para } from "./layout";

export function purchaseKeyEmail(args: { productKey: string; plan: Plan; siteUrl: string; appUrl: string }): EmailTemplate {
  const planName = args.plan === "lifetime" ? "Lifetime" : "Yearly";
  const title = `Your ${brand.pro_name} key`;
  const downloadUrl = `${args.siteUrl}/download`;
  const webUrl = `${args.appUrl}/#key=${encodeURIComponent(args.productKey)}`;
  const accountUrl = `${args.siteUrl}/account`;
  const body =
    para(`Thanks for buying ${esc(brand.pro_name)} ${esc(planName)}. This is your product key. Keep this email.`) +
    keyBlock(args.productKey) +
    para(`<strong>On your computer:</strong> open ${esc(brand.name)}, choose Enter a key, and paste it. If you started the purchase from the app, it has already unlocked by itself.`) +
    para(`<strong>In your browser:</strong> open the link below. The key is in the link and never reaches a server.`) +
    button(webUrl, `Use ${esc(brand.name)} in your browser`) +
    para(`<a href="${esc(downloadUrl)}" style="color:${brand.accent};">Download ${esc(brand.name)}</a> for Windows, macOS or Linux.`) +
    para(`The key works on 3 computers and 3 browsers. You can see and remove them at <a href="${esc(accountUrl)}" style="color:${brand.accent};">${esc(accountUrl)}</a>; sign in with this email address.`);
  const text = `Thanks for buying ${brand.pro_name} ${planName}. This is your product key. Keep this email.

    ${args.productKey}

On your computer: open ${brand.name}, choose Enter a key, and paste it. If you started the purchase from the app, it has already unlocked by itself.
In your browser: ${webUrl}
Download ${brand.name}: ${downloadUrl}

The key works on 3 computers and 3 browsers. See and remove them at ${accountUrl}; sign in with this email address.`;
  return { subject: title, ...layout({ title, body, text }) };
}
