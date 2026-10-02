import { beforeEach, describe, expect, it } from "vitest";

import { GET as updates } from "../app/api/v1/updates/[channel]/[target]/[arch]/[current_version]/route";
import { GET as health } from "../app/api/v1/health/route";
import { clearChannelCache, compareVersions, rolloutBucket } from "../lib/updates";
import { call, makeRequest, makeRuntime, type TestRuntime } from "./helpers";

let rt: TestRuntime;

const channel = {
  version: "1.2.0",
  pub_date: "2026-11-20T15:00:00Z",
  notes: "Faster PDF compression.",
  rollout_percent: 25,
  blocked_versions: ["1.1.9"],
  platforms: {
    "windows-x86_64": { url: "https://github.com/Hunter-Boone/Smidge-Downloads/releases/download/v1.2.0/Smidge_1.2.0_Windows_x64-setup.exe", signature: "sigwin" },
    "darwin-aarch64": { url: "https://github.com/Hunter-Boone/Smidge-Downloads/releases/download/v1.2.0/Smidge_1.2.0_macOS_universal.app.tar.gz", signature: "sigmac" },
    "linux-x86_64": { url: "https://github.com/Hunter-Boone/Smidge-Downloads/releases/download/v1.2.0/Smidge_1.2.0_Linux_x86_64.AppImage", signature: "siglinux" },
  },
};

beforeEach(() => {
  rt = makeRuntime();
  clearChannelCache();
  rt.net.channels.set("stable", channel);
});

function get(version: string, installId: string | null, o: { channel?: string; target?: string; arch?: string } = {}) {
  const params = { channel: o.channel ?? "stable", target: o.target ?? "windows", arch: o.arch ?? "x86_64", current_version: version };
  return call(updates, makeRequest("GET", `/api/v1/updates/${params.channel}/${params.target}/${params.arch}/${version}`, undefined, { origin: null, headers: installId ? { "x-smidge-install": installId } : {} }), params);
}

/** Find install ids on both sides of the 25 percent rollout. */
function installIn(): string {
  for (let i = 0; ; i++) if (rolloutBucket(`inst_${i}`) < 25) return `inst_${i}`;
}
function installOut(): string {
  for (let i = 0; ; i++) if (rolloutBucket(`inst_${i}`) >= 25) return `inst_${i}`;
}

describe("GET /updates", () => {
  it("204 when current or newer", async () => {
    expect((await get("1.2.0", installIn())).status).toBe(204);
    expect((await get("1.3.0", installIn())).status).toBe(204);
  });
  it("rollout_percent gates by sha256(install_id) mod 100", async () => {
    const yes = await get("1.1.0", installIn());
    expect(yes.status).toBe(200);
    expect(await yes.json()).toEqual({ version: "1.2.0", notes: "Faster PDF compression.", pub_date: "2026-11-20T15:00:00Z", url: channel.platforms["windows-x86_64"].url, signature: "sigwin" });
    expect((await get("1.1.0", installOut())).status).toBe(204);
  });
  it("blocked versions always get the update", async () => {
    const res = await get("1.1.9", installOut());
    expect(res.status).toBe(200);
    expect((await res.json()).version).toBe("1.2.0");
  });
  it("picks the platform by target-arch and 204s unknown platforms", async () => {
    const mac = await get("1.0.0", installIn(), { target: "darwin", arch: "aarch64" });
    expect((await mac.json()).signature).toBe("sigmac");
    expect((await get("1.0.0", installIn(), { target: "darwin", arch: "x86_64" })).status).toBe(204);
  });
  it("caches the channel file for 60 s", async () => {
    await get("1.1.9", null);
    await get("1.1.9", null);
    expect(rt.net.calls.filter((c) => c.url.includes("/channels/stable.json"))).toHaveLength(1);
    rt.advance(61_000);
    await get("1.1.9", null);
    expect(rt.net.calls.filter((c) => c.url.includes("/channels/stable.json"))).toHaveLength(2);
  });
  it("204 when the channel file is missing; 404 for an unknown channel; 400 for bad params", async () => {
    expect((await get("1.0.0", null, { channel: "beta" })).status).toBe(204);
    expect((await get("1.0.0", null, { channel: "nightly" })).status).toBe(404);
    expect((await get("1.0.0", null, { target: "../etc" })).status).toBe(400);
  });
  it("reads the raw URL derived from brand.json", async () => {
    await get("1.0.0", null);
    expect(rt.net.calls[0]!.url).toBe("https://raw.githubusercontent.com/Hunter-Boone/Smidge-Downloads/main/channels/stable.json");
  });
});

describe("compareVersions", () => {
  it("orders numerically and puts pre-releases first", () => {
    expect(compareVersions("1.2.0", "1.10.0")).toBe(-1);
    expect(compareVersions("1.2.0-beta.2", "1.2.0")).toBe(-1);
    expect(compareVersions("1.2.0-beta.2", "1.2.0-beta.10")).toBe(-1);
    expect(compareVersions("v1.2.0", "1.2.0")).toBe(0);
    expect(compareVersions("2.0.0", "1.9.9")).toBe(1);
  });
  it("rolloutBucket is in [0, 100)", () => {
    for (let i = 0; i < 200; i++) {
      const b = rolloutBucket(`x${i}`);
      expect(b).toBeGreaterThanOrEqual(0);
      expect(b).toBeLessThan(100);
    }
  });
});

describe("GET /health", () => {
  it("answers ok with a version", async () => {
    const res = await health();
    expect(res.status).toBe(200);
    expect(await res.json()).toMatchObject({ ok: true, version: expect.stringMatching(/^\d+\.\d+\.\d+/) });
  });
});
