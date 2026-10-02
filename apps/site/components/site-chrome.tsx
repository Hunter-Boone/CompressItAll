import Link from "next/link";

import { brand } from "@/lib/brand";
import { appUrl } from "@/lib/public-urls";

export function Header() {
  return (
    <header className="border-b border-subtle bg-surface-base/90 backdrop-blur">
      <div className="mx-auto flex h-16 max-w-6xl items-center justify-between px-5">
        <Link href="/" className="flex items-center gap-2.5 font-display text-xl font-bold text-on-surface" aria-label={`${brand.name} home`}>
          <span className="smg-mark" aria-hidden="true" />
          {brand.wordmark}
        </Link>
        <nav className="flex items-center gap-1 text-sm" aria-label="Main">
          <Link href="/download" className="smg-btn smg-btn--ghost smg-btn--sm">Download</Link>
          <Link href="/buy" className="smg-btn smg-btn--ghost smg-btn--sm">Pricing</Link>
          <Link href="/help" className="smg-btn smg-btn--ghost smg-btn--sm">Help</Link>
          <Link href="/account" className="smg-btn smg-btn--ghost smg-btn--sm">Account</Link>
          <a href={appUrl()} className="smg-btn smg-btn--secondary smg-btn--sm ml-2">Use it in your browser</a>
        </nav>
      </div>
    </header>
  );
}

export function Footer() {
  return (
    <footer className="mt-24 border-t border-subtle">
      <div className="mx-auto flex max-w-6xl flex-col gap-4 px-5 py-10 text-sm text-on-surface-muted md:flex-row md:items-center md:justify-between">
        <p>
          {brand.name} is made by {brand.seller}. Your files never leave your device.
        </p>
        <nav className="flex flex-wrap gap-x-5 gap-y-2" aria-label="Legal">
          <Link href="/privacy" className="hover:text-on-surface">Privacy</Link>
          <Link href="/terms" className="hover:text-on-surface">Terms</Link>
          <Link href="/refunds" className="hover:text-on-surface">Refunds</Link>
          <a href={`mailto:${brand.urls.support_email}`} className="hover:text-on-surface">{brand.urls.support_email}</a>
        </nav>
      </div>
    </footer>
  );
}

export function Page({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return <main className={`mx-auto w-full max-w-6xl px-5 py-12 ${className}`}>{children}</main>;
}
