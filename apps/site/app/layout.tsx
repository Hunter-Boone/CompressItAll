import type { Metadata } from "next";

import { brand } from "@/lib/brand";
import { siteUrl } from "@/lib/public-urls";

import "./globals.css";
import { Footer, Header } from "@/components/site-chrome";

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl()),
  title: { default: `${brand.name}: ${brand.tagline}`, template: `%s · ${brand.name}` },
  description: brand.promise,
  openGraph: { siteName: brand.name, title: `${brand.name}: ${brand.tagline}`, description: brand.promise, type: "website" },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className="theme-light">
      <body className="flex min-h-full flex-col bg-surface-base text-on-surface">
        <Header />
        <div className="flex-1">{children}</div>
        <Footer />
      </body>
    </html>
  );
}
