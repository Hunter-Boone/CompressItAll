import { brand } from "../lib/brand";
import type { EmailTemplate } from "../lib/email";
import { esc, layout, para } from "./layout";

export function refundDoneEmail(): EmailTemplate {
  const title = `Your refund is done`;
  const body =
    para(`Your refund is done. ${esc(brand.pro_name)} is now off.`) +
    para(`The free version keeps working: 3 files a day, every feature, nothing deleted. The money takes a few days to show on your statement.`);
  const text = `Your refund is done. ${brand.pro_name} is now off.

The free version keeps working: 3 files a day, every feature, nothing deleted. The money takes a few days to show on your statement.`;
  return { subject: title, ...layout({ title, body, text }) };
}
