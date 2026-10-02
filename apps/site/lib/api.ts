// Wrapper every /api/v1 route handler uses: lazy runtime, Origin check on POST, CORS for the

import { timingSafeEqual } from "node:crypto";
// routes the web app calls, and one place that maps errors to `{error, message}` bodies.

import { DbUnavailable } from "./db";
import { EmailError } from "./email";
import { EnvError } from "./env";
import { ApiError, checkOrigin, corsHeaders, errorResponse, json, noContent, withHeaders } from "./http";
import { PaddleError } from "./paddle";
import { getRuntime, type Runtime } from "./runtime";

export interface RouteOptions {
  /** Add CORS headers for NEXT_PUBLIC_APP_URL (the (W) routes in DESIGN.md 5.10). */
  cors?: boolean;
  /** POSTs must carry an Origin header (site-browser-only routes). */
  browserOnly?: boolean;
}

export interface Ctx<P = Record<string, string>> {
  req: Request;
  rt: Runtime;
  params: P;
}

type NextCtx<P> = { params: Promise<P> | P };

export function route<P = Record<string, string>>(options: RouteOptions, handler: (ctx: Ctx<P>) => Promise<Response>) {
  return async (req: Request, nextCtx?: NextCtx<P>): Promise<Response> => {
    let rt: Runtime;
    try {
      rt = await getRuntime();
    } catch (err) {
      console.error("[api] runtime", err instanceof EnvError ? err.message : err);
      return json({ error: "internal", message: "The server isn't configured yet." }, { status: 500 });
    }
    let res: Response;
    try {
      if (req.method === "POST") checkOrigin(req, rt.env, { required: options.browserOnly === true });
      const params = (nextCtx ? await nextCtx.params : {}) as P;
      res = await handler({ req, rt, params });
    } catch (err) {
      res = errorResponse(toApiError(err));
      if (res.status >= 500) console.error("[api]", req.method, new URL(req.url).pathname, err);
    }
    return options.cors ? withHeaders(res, corsHeaders(rt.env)) : res;
  };
}

export function toApiError(err: unknown): ApiError {
  if (err instanceof ApiError) return err;
  if (err instanceof DbUnavailable) return new ApiError(503, "unavailable", "Smidge can't reach its database right now. Try again in a minute.", { cause: err });
  if (err instanceof PaddleError) return new ApiError(502, "unavailable", "The payment service didn't answer. Try again in a minute.", { cause: err });
  if (err instanceof EmailError) return new ApiError(502, "unavailable", "The email didn't go out. Try again in a minute.", { cause: err });
  return new ApiError(500, "internal", "Something went wrong on our side. Try again in a minute.", { cause: err });
}

/** OPTIONS handler for CORS routes. */
export function preflight() {
  return async (): Promise<Response> => {
    try {
      const rt = await getRuntime();
      return noContent(corsHeaders(rt.env));
    } catch {
      return noContent();
    }
  };
}

/** Cron auth: `Authorization: Bearer CRON_SECRET`, compared in constant time. */
export function requireCron(rt: Runtime, req: Request): void {
  const header = req.headers.get("authorization") ?? "";
  const expected = `Bearer ${rt.env.CRON_SECRET}`;
  const a = Buffer.from(header, "utf8");
  const b = Buffer.from(expected, "utf8");
  if (a.length !== b.length || !timingSafeEqual(a, b)) {
    throw new ApiError(401, "unauthorized", "Missing or wrong cron secret.");
  }
}
