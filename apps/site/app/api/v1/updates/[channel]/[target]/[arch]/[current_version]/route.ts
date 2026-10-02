import { UpdateChannelSchema, type UpdateResponse } from "@cia/api-types";

import { route } from "@/lib/api";
import { ApiError, json, noContent } from "@/lib/http";
import { decideUpdate, fetchChannel } from "@/lib/updates";

export const dynamic = "force-dynamic";
export const runtime = "nodejs";

export const GET = route<{ channel: string; target: string; arch: string; current_version: string }>({}, async ({ req, rt, params }) => {
  const channel = UpdateChannelSchema.safeParse(params.channel);
  if (!channel.success) throw new ApiError(404, "not_found", "Unknown update channel.");
  if (!/^[a-z0-9_]{1,20}$/.test(params.target) || !/^[a-z0-9_]{1,20}$/.test(params.arch) || !/^[0-9A-Za-z.+-]{1,40}$/.test(params.current_version)) {
    throw new ApiError(400, "bad_request", "Bad updater parameters.");
  }
  const file = await fetchChannel(rt.fetch, channel.data, rt.now().getTime());
  if (!file) return noContent({ "Cache-Control": "no-store" });
  const decision = decideUpdate(file, params.target, params.arch, params.current_version, req.headers.get("x-smidge-install"));
  if (!decision.update) return noContent({ "Cache-Control": "no-store" });
  const res: UpdateResponse = { version: decision.version, url: decision.url, signature: decision.signature };
  if (decision.notes) res.notes = decision.notes;
  if (decision.pub_date) res.pub_date = decision.pub_date;
  return json(res);
});
