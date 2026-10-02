"use client";

import type { Paddle } from "@paddle/paddle-js";
import { useSearchParams } from "next/navigation";
import { useEffect, useRef, useState } from "react";

import { PlanCards } from "@/components/plans";
import { api, ClientApiError } from "@/lib/client-api";
import { paddleClientConfig, siteUrl } from "@/lib/public-urls";

type Plan = "lifetime" | "yearly";

export function BuyClient({ priceIds }: { priceIds: Partial<Record<Plan, string>> }) {
  const PRICE_IDS = priceIds;
  const params = useSearchParams();
  const claimId = params.get("claim");
  const config = paddleClientConfig();
  const paddleRef = useRef<Paddle | null>(null);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState<Plan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [prices, setPrices] = useState<Partial<Record<Plan, string>>>({});

  useEffect(() => {
    if (!config) return;
    let cancelled = false;
    (async () => {
      const { initializePaddle } = await import("@paddle/paddle-js");
      const paddle = await initializePaddle({ environment: config.environment, token: config.token });
      if (cancelled || !paddle) return;
      paddleRef.current = paddle;
      setReady(true);
      const items = (["lifetime", "yearly"] as Plan[]).filter((p) => PRICE_IDS[p]).map((p) => ({ priceId: PRICE_IDS[p]!, quantity: 1 }));
      if (items.length === 0) return;
      try {
        const preview = await paddle.PricePreview({ items });
        const next: Partial<Record<Plan, string>> = {};
        for (const line of preview.data.details.lineItems) {
          const plan = (Object.keys(PRICE_IDS) as Plan[]).find((p) => PRICE_IDS[p] === line.price.id);
          if (plan) next[plan] = line.formattedTotals.total;
        }
        setPrices(next);
      } catch {
        // Price preview is cosmetic; checkout shows the real price.
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config?.environment, config?.token]);

  async function buy(plan: Plan) {
    setError(null);
    setBusy(plan);
    try {
      const { transaction_id, checkout_id } = await api<{ transaction_id: string; checkout_id: string }>("POST", "/checkout", claimId ? { plan, claim_id: claimId } : { plan });
      const paddle = paddleRef.current;
      if (!paddle) throw new Error("Checkout isn't ready yet. Give it a second and try again.");
      paddle.Checkout.open({
        transactionId: transaction_id,
        settings: { displayMode: "overlay", allowLogout: false, successUrl: `${siteUrl()}/success?c=${encodeURIComponent(checkout_id)}` },
      });
    } catch (err) {
      if (err instanceof ClientApiError && err.body.error === "claim_expired") {
        setError("That purchase link has expired. Go back to the app and press Buy again, or buy here and enter the key by hand.");
      } else {
        setError(err instanceof Error ? err.message : "Something went wrong. Try again in a minute.");
      }
    } finally {
      setBusy(null);
    }
  }

  return (
    <div>
      {claimId && (
        <p className="smg-card mb-6 border-primary bg-primary-container p-4 text-primary-container-foreground">
          You started this purchase from the app. When the payment goes through, the app unlocks by itself.
        </p>
      )}
      {!config && (
        <p className="smg-card mb-6 border-warning bg-warning-container p-4 text-warning-container-foreground">
          Checkout isn't set up on this deployment yet. Set NEXT_PUBLIC_PADDLE_ENV and NEXT_PUBLIC_PADDLE_CLIENT_TOKEN to turn it on.
        </p>
      )}
      {error && <p role="alert" className="smg-card mb-6 border-danger-border bg-danger-container p-4 text-danger-container-foreground">{error}</p>}
      <PlanCards
        prices={prices}
        cta={(plan) => (
          <button type="button" onClick={() => buy(plan)} disabled={!config || !ready || busy !== null} className={`smg-btn mt-6 ${plan === "lifetime" ? "smg-btn--primary" : "smg-btn--secondary"}`}>
            {busy === plan ? "Opening checkout" : `Buy ${plan === "lifetime" ? "Lifetime" : "Yearly"}`}
          </button>
        )}
      />
      <p className="mt-6 text-sm text-on-surface-subtle">
        Payment is handled by Paddle, our merchant of record. Card details never touch our servers. Already have a key? Open the app and choose Enter a key.
      </p>
    </div>
  );
}

