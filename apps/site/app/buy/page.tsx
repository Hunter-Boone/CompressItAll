import type { Metadata } from "next";
import { Suspense } from "react";

import { Page } from "@/components/site-chrome";
import { brand } from "@/lib/brand";

import { BuyClient } from "./buy-client";

// Price ids are read at request time (not validated: a deployment without them still renders,
// with plain "One payment" / "Per year" labels instead of Paddle's price preview).
export const dynamic = "force-dynamic";

export const metadata: Metadata = { title: "Buy", description: `${brand.pro_name}: unlimited files on 3 computers and 3 browsers.` };

export default function BuyPage() {
  return (
    <Page>
      <h1>Get {brand.pro_name}</h1>
      <p className="mt-3 max-w-2xl text-on-surface-muted">
        Unlimited files, 3 computers, 3 browsers. Paid once or yearly. Your key arrives by email the moment the payment goes through.
      </p>
      <div className="mt-8">
        <Suspense fallback={<p className="text-on-surface-muted">Loading plans.</p>}>
          <BuyClient priceIds={{ lifetime: process.env.PADDLE_PRICE_ID_LIFETIME, yearly: process.env.PADDLE_PRICE_ID_YEARLY }} />
        </Suspense>
      </div>
    </Page>
  );
}
