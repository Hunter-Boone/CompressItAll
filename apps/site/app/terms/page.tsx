import type { Metadata } from "next";

import { Page } from "@/components/site-chrome";
import { brand } from "@/lib/brand";

export const metadata: Metadata = { title: "Terms" };

// TODO(Hunter): have this reviewed before launch (Paddle's domain review reads it).
export default function TermsPage() {
  return (
    <Page>
      <h1>Terms of service</h1>
      <div className="smg-prose mt-6">
        <p>Last updated: TODO(Hunter) set the date before launch.</p>
        <h2>Who you're dealing with</h2>
        <p>{brand.name} is made by {brand.seller} ("we"). Purchases are sold by Paddle.com Market Ltd as merchant of record, under <a className="smg-link" href="https://www.paddle.com/legal/checkout-buyer-terms">Paddle's buyer terms</a>.</p>
        <h2>The free version</h2>
        <p>You can use {brand.name} free of charge for 3 files per rolling 24 hours per computer or browser. There is no account and no trial.</p>
        <h2>{brand.pro_name}</h2>
        <ul>
          <li>Lifetime: one payment for the current major version and its updates for as long as we publish them.</li>
          <li>Yearly: a subscription that renews every year until you cancel it from your account or Paddle's portal. Cancelling keeps Pro on until the end of the paid year.</li>
          <li>One key works on up to 3 computers and 3 browsers at a time. You may move it between your own devices.</li>
          <li>A key is for one person or household. Please don't share it publicly.</li>
        </ul>
        <h2>Refunds and chargebacks</h2>
        <p>See the <a className="smg-link" href="/refunds">refund policy</a>. A refund or a chargeback turns {brand.pro_name} off on that key; the free version keeps working.</p>
        <h2>What we promise and what we don't</h2>
        <p>{brand.name} checks its results and tells you when a file doesn't fit, but we can't guarantee any particular service will accept any particular file. The software is provided as is, to the extent the law allows. Keep your originals; {brand.name} never deletes or overwrites them.</p>
        <h2>Changes</h2>
        <p>If these terms change in a way that matters, we'll email the address on your account before it takes effect.</p>
        <h2>Contact</h2>
        <p><a className="smg-link" href={`mailto:${brand.urls.support_email}`}>{brand.urls.support_email}</a>. TODO(Hunter): governing law and registered address.</p>
      </div>
    </Page>
  );
}
