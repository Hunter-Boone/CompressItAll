import Link from "next/link";

import { brand } from "@/lib/brand";

export const PLAN_COPY = {
  lifetime: { name: `${brand.pro_name} Lifetime`, price: "One payment", blurb: "Pay once. Every update, for as long as the app exists." },
  yearly: { name: `${brand.pro_name} Yearly`, price: "Per year", blurb: "Cancel from your account any time. Pro stays on until the year is up." },
} as const;

const SHARED = ["Unlimited files", "3 computers and 3 browsers", "Every preset and every format", "Advanced settings and trimming"];
const FREE = ["3 files a day", "Every preset and every format", "No account, no watermark", "The computer or browser you're on"];

export function PlanCards({ prices, cta }: { prices?: Partial<Record<"lifetime" | "yearly", string>>; cta?: (plan: "lifetime" | "yearly") => React.ReactNode }) {
  return (
    <div className="grid gap-5 md:grid-cols-3">
      <div className="smg-card flex flex-col p-6">
        <h3>Free</h3>
        <p className="mt-1 font-display text-3xl font-bold">0</p>
        <p className="mt-1 text-sm text-on-surface-muted">No trial, no card. It just works.</p>
        <ul className="mt-5 flex-1 space-y-2 text-sm text-on-surface-muted">
          {FREE.map((f) => <li key={f} className="flex gap-2"><Check />{f}</li>)}
        </ul>
        <Link href="/download" className="smg-btn smg-btn--secondary mt-6">Download</Link>
      </div>
      {(["lifetime", "yearly"] as const).map((plan) => (
        <div key={plan} className={`smg-card flex flex-col p-6 ${plan === "lifetime" ? "border-primary ring-1 ring-primary" : ""}`}>
          <h3>{PLAN_COPY[plan].name}</h3>
          <p className="mt-1 font-display text-3xl font-bold">{prices?.[plan] ?? PLAN_COPY[plan].price}</p>
          <p className="mt-1 text-sm text-on-surface-muted">{prices?.[plan] ? PLAN_COPY[plan].price.toLowerCase() + ". " : ""}{PLAN_COPY[plan].blurb}</p>
          <ul className="mt-5 flex-1 space-y-2 text-sm text-on-surface-muted">
            {SHARED.map((f) => <li key={f} className="flex gap-2"><Check />{f}</li>)}
          </ul>
          {cta ? cta(plan) : <Link href={`/buy?plan=${plan}`} className={`smg-btn mt-6 ${plan === "lifetime" ? "smg-btn--primary" : "smg-btn--secondary"}`}>Get {plan === "lifetime" ? "Lifetime" : "Yearly"}</Link>}
        </div>
      ))}
    </div>
  );
}

function Check() {
  return (
    <svg aria-hidden="true" viewBox="0 0 20 20" className="mt-1 h-4 w-4 shrink-0 text-success" fill="none" stroke="currentColor" strokeWidth="2.2">
      <path d="M4 10.5l4 4 8-9" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}
