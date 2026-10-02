import Link from "next/link";

import { Page } from "@/components/site-chrome";

export default function NotFound() {
  return (
    <Page>
      <h1>That page isn't here</h1>
      <p className="mt-4 text-on-surface-muted">The link may be old. Try the <Link href="/" className="smg-link">home page</Link> or <Link href="/help" className="smg-link">Help</Link>.</p>
    </Page>
  );
}
