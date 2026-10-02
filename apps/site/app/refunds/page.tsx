import type { Metadata } from "next";

import { Page } from "@/components/site-chrome";
import { brand } from "@/lib/brand";

export const metadata: Metadata = { title: "Refunds" };

// TODO(Hunter): confirm the window matches the Paddle account's refund settings before launch.
export default function RefundsPage() {
  return (
    <Page>
      <h1>Refund policy</h1>
      <div className="smg-prose mt-6">
        <p>Last updated: TODO(Hunter) set the date before launch.</p>
        <p>If {brand.pro_name} isn't for you, ask for a refund within 14 days of buying. No reason needed. Email <a className="smg-link" href={`mailto:${brand.urls.support_email}`}>{brand.urls.support_email}</a> from the address you bought with, or use the link in your Paddle receipt.</p>
        <h2>How it works</h2>
        <ul>
          <li>Paddle, our merchant of record, sends the money back the way you paid. It usually shows within 5 to 10 working days.</li>
          <li>When the refund goes through, {brand.pro_name} turns off on that key at its next check (within a day online). The free version keeps working.</li>
          <li>Yearly renewals: ask within 14 days of the renewal charge. Cancelling from your account stops future charges and keeps Pro on until the paid year ends.</li>
        </ul>
        <h2>After 14 days</h2>
        <p>Write to us anyway. If something is broken for you and we can't fix it, we'd rather refund than keep the money.</p>
        <p>Seller: {brand.seller}.</p>
      </div>
    </Page>
  );
}
