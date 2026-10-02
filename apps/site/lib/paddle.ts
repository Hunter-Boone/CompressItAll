// Paddle Billing REST calls (DESIGN.md 5.8.3, 5.10). Plain fetch against the sandbox or
// production host chosen by PADDLE_ENV; there is no SDK and no fallback key.

import type { PaddleSubscriptionLike } from "./entitlements";
import type { Runtime } from "./runtime";

export class PaddleError extends Error {
  constructor(public readonly status: number, message: string) {
    super(message);
    this.name = "PaddleError";
  }
}

export function paddleBase(env: Runtime["env"]): string {
  return env.PADDLE_ENV === "production" ? "https://api.paddle.com" : "https://sandbox-api.paddle.com";
}

async function call<T>(rt: Runtime, method: "GET" | "POST", path: string, body?: unknown): Promise<T> {
  const res = await rt.fetch(`${paddleBase(rt.env)}${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${rt.env.PADDLE_API_KEY}`,
      "Content-Type": "application/json",
      Accept: "application/json",
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) {
    let detail = "";
    try {
      const err = (await res.json()) as { error?: { detail?: string; code?: string } };
      detail = err.error?.detail ?? err.error?.code ?? "";
    } catch {}
    throw new PaddleError(res.status, `Paddle ${method} ${path} failed with ${res.status}${detail ? `: ${detail}` : ""}`);
  }
  const json = (await res.json()) as { data: T };
  return json.data;
}

export interface PaddleCustomer {
  id: string;
  email: string;
  name?: string | null;
}

export interface PaddleTransaction {
  id: string;
  status: string;
  customer_id: string | null;
  subscription_id: string | null;
  origin: string;
  custom_data: Record<string, unknown> | null;
  items: { price: { id: string; product_id?: string }; quantity: number }[];
}

export interface PaddleSubscription extends PaddleSubscriptionLike {
  customer_id: string;
  items?: { price: { id: string } }[];
}

export const paddle = {
  createTransaction(rt: Runtime, args: { priceId: string; checkoutId: string; claimId: string | null }) {
    return call<{ id: string }>(rt, "POST", "/transactions", {
      items: [{ price_id: args.priceId, quantity: 1 }],
      custom_data: { checkout_id: args.checkoutId, claim_id: args.claimId },
      collection_mode: "automatic",
    });
  },
  getCustomer(rt: Runtime, customerId: string) {
    return call<PaddleCustomer>(rt, "GET", `/customers/${encodeURIComponent(customerId)}`);
  },
  getSubscription(rt: Runtime, subscriptionId: string) {
    return call<PaddleSubscription>(rt, "GET", `/subscriptions/${encodeURIComponent(subscriptionId)}`);
  },
  getTransaction(rt: Runtime, transactionId: string) {
    return call<PaddleTransaction>(rt, "GET", `/transactions/${encodeURIComponent(transactionId)}`);
  },
  async createPortalSession(rt: Runtime, customerId: string, subscriptionIds: string[]) {
    const data = await call<{ urls: { general: { overview: string }; subscriptions?: { id: string; cancel_subscription: string; update_subscription_payment_method: string }[] } }>(
      rt,
      "POST",
      `/customers/${encodeURIComponent(customerId)}/portal-sessions`,
      subscriptionIds.length > 0 ? { subscription_ids: subscriptionIds } : {},
    );
    return data.urls.general.overview;
  },
};
