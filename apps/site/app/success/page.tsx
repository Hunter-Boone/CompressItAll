import type { Metadata } from "next";
import { Suspense } from "react";

import { Page } from "@/components/site-chrome";

import { SuccessClient } from "./success-client";

export const metadata: Metadata = { title: "Thanks", robots: { index: false } };

export default function SuccessPage() {
  return (
    <Page>
      <Suspense fallback={<p className="text-on-surface-muted">One moment.</p>}>
        <SuccessClient />
      </Suspense>
    </Page>
  );
}
