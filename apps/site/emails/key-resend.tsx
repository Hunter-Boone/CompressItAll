import type { Plan } from "@cia/api-types";

import { brand } from "../lib/brand";
import type { EmailTemplate } from "../lib/email";
import { esc, keyBlock, layout, para } from "./layout";

export function keyResendEmail(args: { keys: { productKey: string; plan: Plan; active: boolean }[]; siteUrl: string }): EmailTemplate {
  const title = `Your ${brand.pro_name} ${args.keys.length === 1 ? "key" : "keys"}`;
  const body =
    para(`You asked for the ${esc(brand.pro_name)} ${args.keys.length === 1 ? "key" : "keys"} for this email address.`) +
    args.keys.map((k) => para(`<strong>${esc(k.plan === "lifetime" ? "Lifetime" : "Yearly")}</strong>${k.active ? "" : " (not active)"}`) + keyBlock(k.productKey)).join("") +
    para(`Open ${esc(brand.name)}, choose Enter a key, and paste it. Manage computers at <a href="${esc(args.siteUrl)}/account" style="color:${brand.accent};">${esc(args.siteUrl)}/account</a>.`);
  const text = `You asked for the ${brand.pro_name} keys for this email address.

${args.keys.map((k) => `${k.plan === "lifetime" ? "Lifetime" : "Yearly"}${k.active ? "" : " (not active)"}: ${k.productKey}`).join("\n")}

Open ${brand.name}, choose Enter a key, and paste it. Manage computers at ${args.siteUrl}/account.`;
  return { subject: title, ...layout({ title, body, text }) };
}
