// URLs safe to use in client components: NEXT_PUBLIC_* at build time, brand.json as the fallback
// so a build with no environment still renders sensible links.

import { brand } from "./brand";

export function siteUrl(): string {
  return (process.env.NEXT_PUBLIC_SITE_URL || brand.urls.site).replace(/\/+$/, "");
}

export function appUrl(): string {
  return (process.env.NEXT_PUBLIC_APP_URL || brand.urls.app).replace(/\/+$/, "");
}

/** `https://app.<domain>/#key=XXXXX-...`; the fragment never reaches a server. */
export function appKeyUrl(productKey: string): string {
  return `${appUrl()}/#key=${encodeURIComponent(productKey)}`;
}

export interface PaddleClientConfig {
  environment: "sandbox" | "production";
  token: string;
}

/** Paddle.js config, or null while the placeholders are in place. */
export function paddleClientConfig(): PaddleClientConfig | null {
  const env = process.env.NEXT_PUBLIC_PADDLE_ENV;
  const token = process.env.NEXT_PUBLIC_PADDLE_CLIENT_TOKEN ?? "";
  if ((env !== "sandbox" && env !== "production") || !token) return null;
  if (/placeholder|change-?me|your-/i.test(token)) return null;
  if (env === "sandbox" && !token.startsWith("test_")) return null;
  if (env === "production" && !token.startsWith("live_")) return null;
  return { environment: env, token };
}
