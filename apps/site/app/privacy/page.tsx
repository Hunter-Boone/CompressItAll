import type { Metadata } from "next";

import { Page } from "@/components/site-chrome";
import { brand } from "@/lib/brand";

export const metadata: Metadata = { title: "Privacy" };

// TODO(Hunter): have this reviewed before launch (Paddle's domain review reads it).
export default function PrivacyPage() {
  return (
    <Page>
      <h1>Privacy</h1>
      <div className="smg-prose mt-6">
        <p>Last updated: TODO(Hunter) set the date before launch.</p>
        <h2>The short version</h2>
        <p>Your files never leave your device. {brand.name} compresses on your computer or in your browser. We never see what you compress.</p>
        <h2>What we keep</h2>
        <ul>
          <li>If you buy {brand.pro_name}: your email address, your product key (encrypted), the plan you bought, and the Paddle ids for the purchase so refunds work.</li>
          <li>For each computer or browser you activate: a hashed device id (we never see the raw machine id), the name your computer reports, its platform, the app version and when it last checked in.</li>
          <li>When you sign in to the account page: a session cookie (<code>smg_session</code>) for 30 days and your browser's user agent string.</li>
          <li>Rate-limit counters keyed on a salted hash of your IP address, deleted after a day.</li>
          <li>A log of which emails we sent you, so we never send the same one twice.</li>
        </ul>
        <h2>What we don't keep</h2>
        <ul>
          <li>Your files, their names, or anything about what you compress.</li>
          <li>Card details. Paddle is the merchant of record and handles payment.</li>
          <li>Analytics or advertising trackers. There are none on this site or in the apps.</li>
        </ul>
        <h2>Who else sees data</h2>
        <ul>
          <li>Paddle.com Market Ltd (merchant of record): payment, tax, receipts. <a className="smg-link" href="https://www.paddle.com/legal/privacy">Paddle's privacy policy</a>.</li>
          <li>Resend: delivers our emails.</li>
          <li>Supabase and Vercel: host the database and this site.</li>
          <li>GitHub: hosts the downloads and update files. Downloading the app makes a request to GitHub.</li>
        </ul>
        <h2>Your rights</h2>
        <p>Email <a className="smg-link" href={`mailto:${brand.urls.support_email}`}>{brand.urls.support_email}</a> to see or delete what we hold about you. Deleting your record ends {brand.pro_name} on that key.</p>
        <h2>Who we are</h2>
        <p>{brand.seller}. TODO(Hunter): add the registered address and the country whose law applies.</p>
      </div>
    </Page>
  );
}
