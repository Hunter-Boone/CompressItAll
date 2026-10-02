import { accountResponse } from "@/lib/account";
import { route } from "@/lib/api";
import { json } from "@/lib/http";
import { requireSession } from "@/lib/session";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const GET = route({}, async ({ req, rt }) => {
  const session = await requireSession(rt, req);
  return json(await accountResponse(rt, session));
});
