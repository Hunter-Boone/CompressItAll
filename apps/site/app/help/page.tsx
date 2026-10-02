import type { Metadata } from "next";
import Link from "next/link";

import { Page } from "@/components/site-chrome";
import { brand } from "@/lib/brand";

export const metadata: Metadata = { title: "Help", description: `Short answers about ${brand.name}: limits, FFmpeg, keys and refunds.` };

const ARTICLES = [
  { id: "discord", title: "Discord limits" },
  { id: "email", title: "Email limits" },
  { id: "ffmpeg", title: "What FFmpeg is and why the desktop app downloads it" },
  { id: "cant-fit", title: "Why did it say it can't fit?" },
  { id: "keys", title: "Product keys, computers and browsers" },
  { id: "free", title: "The free version" },
];

export default function HelpPage() {
  return (
    <Page>
      <h1>Help</h1>
      <nav aria-label="Articles" className="mt-6">
        <ul className="flex flex-wrap gap-2">
          {ARTICLES.map((a) => <li key={a.id}><a href={`#${a.id}`} className="smg-btn smg-btn--secondary smg-btn--sm">{a.title}</a></li>)}
        </ul>
      </nav>

      <div className="smg-prose mt-4">
        <h2 id="discord">Discord limits</h2>
        <p>Discord Free allows 10 MB per file for most servers as of 2026; Nitro Basic allows 50 MB and Nitro 500 MB. Discord counts decimal megabytes. {brand.name}'s Discord presets aim a little under the limit so the upload goes through on the first try.</p>
        <p>Videos for Discord come out as MP4 with H.264 and AAC so they play inline on every platform. Discord plays WebM too, but mobile previews are less reliable, so {brand.name} only uses WebM when you ask for it in Advanced.</p>

        <h2 id="email">Email limits</h2>
        <p>Most email services allow attachments of 20 to 25 MB in total per message, and the message itself takes a few percent on top. The Email preset in {brand.name} aims for 20 MB across everything you drop in, so a batch of photos is shrunk as a group, not one by one.</p>
        <p>If you have more than that, the Email preset packs the result into a zip so it is one attachment.</p>

        <h2 id="ffmpeg">What FFmpeg is and why the desktop app downloads it</h2>
        <p>FFmpeg is the open-source tool nearly every video app is built on. It is licensed under the LGPL, which means {brand.name} can use it as a separate program but should not bundle it inside the app. So the desktop app asks once, downloads a copy from our own build (about 40 MB, with its source published alongside), and keeps it in {brand.name}'s own folder. It is never shared with other programs and is removed with the app.</p>
        <p>If you already have FFmpeg installed, Advanced lets you point {brand.name} at it instead.</p>
        <p>In the browser, {brand.name} uses your browser's built-in video encoder instead. That works for most clips; some formats are desktop-only.</p>

        <h2 id="cant-fit">Why did it say it can't fit?</h2>
        <p>{brand.name} refuses before it starts when the result would be unusable. A 20-minute 4K video has no honest way into a 10 MB file; it would come out as a smear. Instead of handing you that, the app says what would work: a shorter cut (it tells you the longest that fits), a bigger limit, or splitting into parts.</p>
        <p>For pictures, "can't fit" is rare and means the limit is below what the smallest readable copy needs. For PDFs, it usually means the pages are scans and the limit is below a legible scan.</p>
        <p>The free version also stops a job that has more files than you have free slots left, and offers to do the first 3 now.</p>

        <h2 id="keys">Product keys, computers and browsers</h2>
        <p>A {brand.pro_name} key works on 3 computers and 3 browsers at once. Reinstalling on the same computer does not use a new slot. A fourth browser quietly replaces the one you used least recently. A fourth computer is refused until you remove one in <Link href="/account" className="smg-link">your account</Link>.</p>
        <p>Lost your key? Open the app, choose Enter a key, then "I lost my key", and it is emailed to the address you bought with. The same link is on the account page.</p>
        <p>The desktop app checks its key once a day when online and keeps working offline for 45 days between checks.</p>

        <h2 id="free">The free version</h2>
        <p>3 files per rolling 24 hours per computer or browser. Only files that come out fitted count; refusals and failures never use a slot. Every preset, every format and every setting is in the free version. There is no watermark and no account.</p>

        <h2>Still stuck</h2>
        <p>Email <a href={`mailto:${brand.urls.support_email}`} className="smg-link">{brand.urls.support_email}</a>. Say which app (desktop or browser), what you dropped in and what it said.</p>
      </div>
    </Page>
  );
}
