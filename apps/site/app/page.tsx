import Link from "next/link";

import { DownloadButtons } from "@/components/download-buttons";
import { PlanCards } from "@/components/plans";
import { Page } from "@/components/site-chrome";
import { brand } from "@/lib/brand";
import { appUrl } from "@/lib/public-urls";

const STEPS = [
  { n: 1, title: "Drop it in", body: "A photo, a video, a PDF, a folder, a zip. One file or a hundred." },
  { n: 2, title: "Pick where it's going", body: "Discord, email, WhatsApp, a 10 MB upload form. Smidge knows the limits." },
  { n: 3, title: "Send it", body: "It fits, or Smidge tells you why and what will work. Nothing is uploaded; it all happens on your device." },
];

const FAQ = [
  { q: "Does Smidge upload my files?", a: "No. Compression runs on your computer or inside your browser. The only thing that ever reaches our server is your product key check if you buy Pro." },
  { q: "What's the catch with the free version?", a: "3 files per rolling 24 hours per computer or browser. Every feature and preset is there. There's no watermark and no account." },
  { q: "Why does it sometimes say it can't fit?", a: "Some things can't get small enough without becoming useless, like a 20-minute video for a 10 MB limit. Smidge says so before it starts, and tells you the longest clip that would fit." },
  { q: "Do I need an account?", a: "No. Pro is a product key. The account page exists only so you can see your key and remove old computers." },
  { q: "What about video on the desktop app?", a: "Video needs FFmpeg. Smidge downloads a copy with your permission the first time you ask for it. It stays on your computer." },
  { q: "Can I get a refund?", a: "Yes, within 14 days of buying. Payments are handled by Paddle, our merchant of record; refunds come back the way you paid." },
];

export default function Home() {
  return (
    <Page className="py-16">
      <section className="max-w-3xl">
        <p className="smg-label">{brand.name}</p>
        <h1 className="mt-3">{brand.tagline}</h1>
        <p className="mt-5 text-lg text-on-surface-muted">{brand.promise}</p>
        <div className="mt-8">
          <DownloadButtons />
        </div>
        <p className="mt-4 text-on-surface-muted">
          Or <a href={appUrl()} className="smg-link">use it in your browser</a>. Same engine, nothing installed.
        </p>
      </section>

      <section className="mt-24" aria-labelledby="how">
        <h2 id="how">How it works</h2>
        <ol className="mt-6 grid gap-5 md:grid-cols-3">
          {STEPS.map((s) => (
            <li key={s.n} className="smg-card p-6">
              <span className="inline-flex h-8 w-8 items-center justify-center rounded-full bg-primary font-semibold text-primary-foreground">{s.n}</span>
              <h3 className="mt-4">{s.title}</h3>
              <p className="mt-1 text-on-surface-muted">{s.body}</p>
            </li>
          ))}
        </ol>
      </section>

      <section className="mt-24 smg-card p-8 md:p-10" aria-labelledby="honest">
        <h2 id="honest">It fits, or it says why</h2>
        <p className="mt-3 max-w-2xl text-on-surface-muted">
          Smidge checks the result before it hands it back. If a file comes out at 19.6 MB for a 20 MB limit, it says so. If a video can't be made small enough
          without turning to mush, it tells you that before encoding, with the longest cut that would work. You never find out from the error message on the other end.
        </p>
      </section>

      <section className="mt-24" aria-labelledby="pricing">
        <h2 id="pricing">Pricing</h2>
        <p className="mt-2 text-on-surface-muted">Free does the job. Pro removes the daily limit.</p>
        <div className="mt-6">
          <PlanCards />
        </div>
        <p className="mt-4 text-sm text-on-surface-subtle">Prices show at checkout in your currency, tax included. Sold by {brand.seller} through Paddle.</p>
      </section>

      <section className="mt-24" aria-labelledby="faq">
        <h2 id="faq">Questions</h2>
        <dl className="mt-6 grid gap-x-10 gap-y-6 md:grid-cols-2">
          {FAQ.map((f) => (
            <div key={f.q}>
              <dt className="font-semibold">{f.q}</dt>
              <dd className="mt-1 text-on-surface-muted">{f.a}</dd>
            </div>
          ))}
        </dl>
        <p className="mt-6 text-on-surface-muted">More in <Link href="/help" className="smg-link">Help</Link>.</p>
      </section>

      <section className="mt-24 border-t border-subtle pt-10" aria-labelledby="privacy">
        <h2 id="privacy">Privacy, in one line</h2>
        <p className="mt-3 max-w-2xl text-on-surface-muted">
          Your files never leave your device. The site keeps your email and product key if you buy Pro, and nothing about what you compress. <Link href="/privacy" className="smg-link">The full policy</Link>.
        </p>
      </section>
    </Page>
  );
}
