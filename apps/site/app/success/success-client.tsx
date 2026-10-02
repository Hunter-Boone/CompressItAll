"use client";

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";

import { api, ClientApiError } from "@/lib/client-api";
import { brand } from "@/lib/brand";
import { appKeyUrl } from "@/lib/public-urls";

type Status = { kind: "waiting"; since: number } | { kind: "done"; key: string; plan: "lifetime" | "yearly" } | { kind: "gone"; message: string } | { kind: "missing" };

export function SuccessClient() {
  const params = useSearchParams();
  const checkoutId = params.get("c");
  const [status, setStatus] = useState<Status>(() => (checkoutId ? { kind: "waiting", since: Date.now() } : { kind: "missing" }));
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!checkoutId || status.kind !== "waiting") return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    const tick = async () => {
      try {
        const r = await api<{ status: "pending" } | { status: "fulfilled"; product_key: string; plan: "lifetime" | "yearly" }>("GET", `/checkout/${encodeURIComponent(checkoutId)}`);
        if (stopped) return;
        if (r.status === "fulfilled") {
          setStatus({ kind: "done", key: r.product_key, plan: r.plan });
          return;
        }
      } catch (err) {
        if (stopped) return;
        if (err instanceof ClientApiError && (err.status === 401 || err.status === 404 || err.status === 410)) {
          setStatus({ kind: "gone", message: err.message });
          return;
        }
      }
      const elapsed = Date.now() - status.since;
      timer = setTimeout(tick, elapsed < 10 * 60 * 1000 ? 3000 : 10000);
    };
    void tick();
    return () => {
      stopped = true;
      if (timer) clearTimeout(timer);
    };
  }, [checkoutId, status]);

  if (status.kind === "missing") {
    return (
      <div className="max-w-2xl">
        <h1>Nothing to show here</h1>
        <p className="mt-4 text-on-surface-muted">This page needs the link Paddle sends you to after paying. Your key is also in your email. <Link href="/account" className="smg-link">Your account</Link> shows it too.</p>
      </div>
    );
  }
  if (status.kind === "gone") {
    return (
      <div className="max-w-2xl">
        <h1>Your key is in your email</h1>
        <p className="mt-4 text-on-surface-muted">{status.message} You can also see it any time in <Link href="/account" className="smg-link">your account</Link>.</p>
      </div>
    );
  }
  if (status.kind === "waiting") {
    const long = Date.now() - status.since > 60_000;
    return (
      <div className="max-w-2xl" aria-live="polite">
        <h1>Finishing your purchase</h1>
        <p className="mt-4 text-on-surface-muted">Waiting for the payment to confirm. This usually takes a few seconds.</p>
        {long && <p className="mt-2 text-on-surface-muted">Taking longer than usual. Your key will arrive by email either way, and this page keeps checking.</p>}
      </div>
    );
  }

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(status.key);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {}
  };

  return (
    <div className="max-w-2xl">
      <h1>{brand.pro_name} is yours</h1>
      <p className="mt-4 text-on-surface-muted">This is your product key. It's also in your email. {brand.name} on your computer unlocks by itself if you started from the app.</p>
      <div className="smg-card mt-6 p-5">
        <p className="smg-label">Product key · {status.plan === "lifetime" ? "Lifetime" : "Yearly"}</p>
        <p className="smg-key mt-2">{status.key}</p>
        <div className="mt-4 flex flex-wrap gap-3">
          <button type="button" onClick={copy} className="smg-btn smg-btn--secondary smg-btn--sm">{copied ? "Copied" : "Copy key"}</button>
        </div>
      </div>
      <div className="mt-8 flex flex-wrap gap-3">
        <Link href="/download" className="smg-btn smg-btn--primary">Download {brand.name}</Link>
        <a href={appKeyUrl(status.key)} className="smg-btn smg-btn--secondary">Use {brand.name} in your browser</a>
      </div>
      <p className="mt-6 text-sm text-on-surface-subtle">On your computer: open {brand.name}, choose Enter a key, paste it. The browser link carries the key in the address; it never reaches a server. This page shows the key for 2 hours.</p>
    </div>
  );
}
