"use client";

import Link from "next/link";
import { useCallback, useEffect, useState } from "react";

import type { AccountEntitlement, AccountResponse } from "@cia/api-types";

import { brand } from "@/lib/brand";
import { api, ClientApiError, formatDate, relativeTime } from "@/lib/client-api";
import { describeStatus } from "@/lib/entitlements";

type View = { kind: "loading" } | { kind: "email"; error?: string; busy?: boolean } | { kind: "code"; email: string; error?: string; busy?: boolean } | { kind: "dashboard"; data: AccountResponse };

export function AccountClient() {
  const [view, setView] = useState<View>({ kind: "loading" });
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const data = await api<AccountResponse>("GET", "/account");
      setView({ kind: "dashboard", data });
    } catch (err) {
      if (err instanceof ClientApiError && err.status === 401) setView({ kind: "email" });
      else setView({ kind: "email", error: err instanceof Error ? err.message : "Something went wrong." });
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  if (view.kind === "loading") return <p className="text-on-surface-muted">Loading your account.</p>;

  if (view.kind === "email" || view.kind === "code") {
    return <SignIn view={view} setView={setView} onSignedIn={load} />;
  }

  const { data } = view;
  const act = async (fn: () => Promise<void>, done?: string) => {
    setNotice(null);
    try {
      await fn();
      if (done) setNotice(done);
      await load();
    } catch (err) {
      setNotice(err instanceof Error ? err.message : "Something went wrong.");
    }
  };

  return (
    <div className="max-w-3xl">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1>Your account</h1>
          <p className="mt-2 text-on-surface-muted">{data.email}</p>
        </div>
        <div className="flex gap-2">
          <button type="button" className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => act(async () => { await api("POST", "/auth/logout", {}); }, "Signed out.")}>Sign out</button>
          <button type="button" className="smg-btn smg-btn--ghost smg-btn--sm" onClick={() => act(async () => { await api("POST", "/auth/logout", { everywhere: true }); }, "Signed out everywhere.")}>Sign out everywhere</button>
        </div>
      </div>
      {notice && <p role="status" className="smg-card mt-6 p-4 text-on-surface-muted">{notice}</p>}
      {data.entitlements.length === 0 && (
        <p className="smg-card mt-8 p-6 text-on-surface-muted">No {brand.pro_name} on this email yet. <Link href="/buy" className="smg-link">Get {brand.pro_name}</Link>, or check the email you bought with.</p>
      )}
      <div className="mt-8 space-y-6">
        {data.entitlements.map((e) => <EntitlementCard key={e.id} ent={e} act={act} />)}
      </div>
      <p className="mt-10 text-sm text-on-surface-subtle">
        <Link href="/download" className="smg-link">Download {brand.name}</Link> for Windows, macOS or Linux. Questions: <a href={`mailto:${brand.urls.support_email}`} className="smg-link">{brand.urls.support_email}</a>
      </p>
    </div>
  );
}

function SignIn({ view, setView, onSignedIn }: { view: Extract<View, { kind: "email" | "code" }>; setView: (v: View) => void; onSignedIn: () => Promise<void> }) {
  const [email, setEmail] = useState(view.kind === "code" ? view.email : "");
  const [code, setCode] = useState("");

  const send = async (ev: React.FormEvent) => {
    ev.preventDefault();
    setView({ kind: "email", busy: true });
    try {
      await api("POST", "/auth/otp/send", { email });
      setView({ kind: "code", email: email.trim().toLowerCase() });
    } catch (err) {
      setView({ kind: "email", error: err instanceof Error ? err.message : "Something went wrong." });
    }
  };
  const verify = async (ev: React.FormEvent) => {
    ev.preventDefault();
    if (view.kind !== "code") return;
    setView({ ...view, busy: true, error: undefined });
    try {
      await api("POST", "/auth/otp/verify", { email: view.email, code });
      await onSignedIn();
    } catch (err) {
      setView({ kind: "code", email: view.email, error: err instanceof Error ? err.message : "Something went wrong." });
    }
  };

  return (
    <div className="max-w-md">
      <h1>Your account</h1>
      <p className="mt-3 text-on-surface-muted">See your key, remove old computers, manage billing. No password: we email you a code.</p>
      {view.kind === "email" ? (
        <form onSubmit={send} className="smg-card mt-6 space-y-4 p-6">
          <label className="block">
            <span className="smg-label">Email you bought with</span>
            <input type="email" required autoComplete="email" value={email} onChange={(e) => setEmail(e.target.value)} className="smg-input mt-1" />
          </label>
          {view.error && <p role="alert" className="text-sm text-danger">{view.error}</p>}
          <button type="submit" disabled={view.busy} className="smg-btn smg-btn--primary w-full">{view.busy ? "Sending" : "Email me a code"}</button>
        </form>
      ) : (
        <form onSubmit={verify} className="smg-card mt-6 space-y-4 p-6">
          <p className="text-on-surface-muted">If {view.email} has {brand.pro_name}, a 6-digit code is on its way. It works for 10 minutes.</p>
          <label className="block">
            <span className="smg-label">Code</span>
            <input inputMode="numeric" pattern="[0-9]{6}" maxLength={6} required autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))} className="smg-input mt-1 font-mono text-xl tracking-widest" />
          </label>
          {view.error && <p role="alert" className="text-sm text-danger">{view.error}</p>}
          <button type="submit" disabled={view.busy || code.length !== 6} className="smg-btn smg-btn--primary w-full">{view.busy ? "Checking" : "Sign in"}</button>
          <button type="button" className="smg-btn smg-btn--ghost w-full" onClick={() => setView({ kind: "email" })}>Use a different email</button>
        </form>
      )}
    </div>
  );
}

function EntitlementCard({ ent, act }: { ent: AccountEntitlement; act: (fn: () => Promise<void>, done?: string) => Promise<void> }) {
  const [shown, setShown] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const planName = ent.plan === "lifetime" ? "Lifetime" : "Yearly";
  const line = describeStatus({ plan: ent.plan, status: ent.status, access_until: ent.access_until, cancel_at: ent.cancel_at, revoked_at: (ent as { revoked_at?: string | null }).revoked_at ?? null, created_at: ent.created_at }, formatDate);

  const reveal = async () => {
    if (shown) {
      setShown(null);
      return;
    }
    const r = await api<{ product_key: string }>("POST", "/account/reveal-key", { entitlement_id: ent.id });
    setShown(r.product_key);
  };
  const copy = async () => {
    const key = shown ?? (await api<{ product_key: string }>("POST", "/account/reveal-key", { entitlement_id: ent.id })).product_key;
    await navigator.clipboard.writeText(key);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <section className="smg-card p-6" aria-label={`${brand.pro_name} ${planName}`}>
      <h2>{brand.pro_name} · {planName} · {line}</h2>
      {ent.status === "past_due" && <p className="mt-2 text-warning">Pro stays on for 30 days while the card is sorted out.</p>}

      <div className="mt-5">
        <p className="smg-label">Product key</p>
        <p className="smg-key mt-1">{shown ?? ent.key_masked}</p>
        <div className="mt-3 flex flex-wrap gap-2">
          <button type="button" className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => act(reveal)}>{shown ? "Hide" : "Show"}</button>
          <button type="button" className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => act(copy)}>{copied ? "Copied" : "Copy"}</button>
          <EmailItButton />
        </div>
      </div>

      <div className="mt-6">
        <p className="smg-label">Computers</p>
        <p className="mt-1 text-sm text-on-surface-muted">{ent.devices.length} of {ent.devices_max} computers in use.</p>
        {ent.devices.length > 0 && (
          <ul className="mt-2 divide-y divide-border-subtle">
            {ent.devices.map((d) => (
              <li key={d.id} className="flex items-center justify-between gap-3 py-2">
                <div>
                  <p className="font-medium">{d.name ?? "Unnamed computer"}</p>
                  <p className="text-sm text-on-surface-subtle">{platformLabel(d.platform)} · last used {relativeTime(d.last_seen_at)}</p>
                </div>
                <button type="button" className="smg-btn smg-btn--ghost smg-btn--sm" onClick={() => act(async () => { await api("POST", `/account/devices/${d.id}/deactivate`); }, "Computer removed. Its copy of Smidge drops to the free version at its next check.")}>Remove</button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="mt-6 flex flex-wrap items-center justify-between gap-3">
        <p className="text-sm text-on-surface-muted">{ent.web_count === 0 ? "Not used in any browser." : `Used in ${ent.web_count} ${ent.web_count === 1 ? "browser" : "browsers"}.`}</p>
        {ent.web_count > 0 && (
          <button type="button" className="smg-btn smg-btn--ghost smg-btn--sm" onClick={() => act(async () => { await api("POST", "/account/web/signout", { entitlement_id: ent.id }); }, "Signed out of all browsers.")}>Sign out all browsers</button>
        )}
      </div>

      <div className="mt-6 flex flex-wrap gap-2 border-t border-subtle pt-5">
        <button type="button" className="smg-btn smg-btn--secondary smg-btn--sm" onClick={() => act(async () => {
          const r = await api<{ url: string }>("POST", "/account/portal", { entitlement_id: ent.id });
          window.location.href = r.url;
        })}>Manage billing</button>
        <Link href="/download" className="smg-btn smg-btn--ghost smg-btn--sm">Download {brand.name}</Link>
      </div>
    </section>
  );
}

function EmailItButton() {
  const [state, setState] = useState<"idle" | "sent">("idle");
  return (
    <button type="button" className="smg-btn smg-btn--ghost smg-btn--sm" onClick={async () => {
      // The signed-in email is the one on the account; /license/resend needs it in the body.
      const me = await api<AccountResponse>("GET", "/account");
      await api("POST", "/license/resend", { email: me.email });
      setState("sent");
      setTimeout(() => setState("idle"), 3000);
    }}>
      {state === "sent" ? "Sent" : "Email it to me"}
    </button>
  );
}

function platformLabel(p: string | null): string {
  return p === "windows" ? "Windows" : p === "macos" ? "macOS" : p === "linux" ? "Linux" : p === "web" ? "Browser" : "Unknown";
}
