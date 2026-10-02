import { brand } from "../lib/brand";
import type { EmailTemplate } from "../lib/email";
import { esc, keyBlock, layout, para } from "./layout";

export function otpEmail(args: { code: string }): EmailTemplate {
  const title = `Your ${brand.name} sign-in code`;
  const body =
    para(`Enter this code on the ${esc(brand.name)} account page. It works for 10 minutes.`) +
    keyBlock(args.code) +
    para(`If you didn't ask for a code, you can ignore this email. Nobody can sign in without it.`);
  const text = `Enter this code on the ${brand.name} account page. It works for 10 minutes.

    ${args.code}

If you didn't ask for a code, you can ignore this email. Nobody can sign in without it.`;
  return { subject: title, ...layout({ title, body, text }) };
}

