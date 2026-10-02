import type { Metadata } from "next";
import Link from "next/link";

import { DownloadButtons } from "@/components/download-buttons";
import { Page } from "@/components/site-chrome";
import { brand } from "@/lib/brand";
import { PLATFORMS } from "@/lib/downloads";
import { appUrl } from "@/lib/public-urls";

export const metadata: Metadata = { title: "Download", description: `${brand.name} for Windows, macOS and Linux, or in your browser.` };

export default function DownloadPage() {
  return (
    <Page>
      <h1>Download {brand.name}</h1>
      <p className="mt-3 max-w-2xl text-on-surface-muted">Free, no account. The desktop app handles video with hardware encoders and saves next to the original file.</p>
      <div className="mt-8">
        <DownloadButtons />
      </div>
      <ul className="mt-8 grid gap-4 md:grid-cols-3">
        {PLATFORMS.map((p) => (
          <li key={p.key} className="smg-card p-5">
            <h3>{p.label}</h3>
            <p className="mt-1 text-sm text-on-surface-muted">{p.note}</p>
          </li>
        ))}
      </ul>
      <section className="smg-prose mt-12">
        <h2>Or use it in your browser</h2>
        <p>
          <a href={appUrl()} className="smg-link">{appUrl().replace(/^https?:\/\//, "")}</a> runs the same engine inside your browser. Nothing is installed and nothing is uploaded. Video in the browser depends on what your browser can encode; the desktop app does more.
        </p>
        <h2>Updates</h2>
        <p>The desktop app checks for updates when it starts and every 6 hours. It downloads in the background and asks before restarting; it never restarts during a job. On Linux, the AppImage updates itself; the .deb points you back here.</p>
        <h2>Video needs FFmpeg</h2>
        <p>The first time you compress a video on the desktop, {brand.name} asks to download FFmpeg (about 40 MB) from our own build repository. See <Link href="/help#ffmpeg" className="smg-link">why</Link>.</p>
      </section>
    </Page>
  );
}
