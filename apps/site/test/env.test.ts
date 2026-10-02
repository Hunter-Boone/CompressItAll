import { describe, expect, it } from "vitest";

import { EnvError, parseEnv } from "../lib/env";
import { testEnvRecord } from "./helpers";

function expectRejects(overrides: Record<string, string | undefined>, pathPart: string) {
  let err: unknown;
  try {
    parseEnv(testEnvRecord(overrides));
  } catch (e) {
    err = e;
  }
  expect(err).toBeInstanceOf(EnvError);
  expect((err as EnvError).issues.join("\n")).toContain(pathPart);
}

describe("env validation", () => {
  it("accepts the test environment", () => {
    const env = parseEnv(testEnvRecord());
    expect(env.PADDLE_ENV).toBe("sandbox");
    expect(env.NEXT_PUBLIC_SITE_URL).toBe("http://localhost:3031");
  });
  it("strips trailing slashes from URLs", () => {
    expect(parseEnv(testEnvRecord({ NEXT_PUBLIC_SITE_URL: "https://www.smidge.example/" })).NEXT_PUBLIC_SITE_URL).toBe("https://www.smidge.example");
  });
  it("rejects every placeholder in .env.example", async () => {
    const { readFileSync } = await import("node:fs");
    const example = readFileSync(new URL("../.env.example", import.meta.url), "utf8");
    const record: Record<string, string> = {};
    for (const line of example.split("\n")) {
      const m = /^([A-Z_]+)=(.*)$/.exec(line);
      if (m) record[m[1]!] = m[2]!;
    }
    expect(() => parseEnv(record)).toThrow(EnvError);
    let err: EnvError | undefined;
    try {
      parseEnv(record);
    } catch (e) {
      err = e as EnvError;
    }
    for (const key of ["PADDLE_API_KEY", "PADDLE_WEBHOOK_SECRET", "RESEND_API_KEY", "LICENSE_SIGNING_KEY", "KEY_ENCRYPTION_KEY", "OTP_PEPPER", "IP_HASH_SALT", "CRON_SECRET"]) {
      expect(err!.issues.some((i) => i.startsWith(`${key}:`))).toBe(true);
    }
  });
  it("rejects placeholder-like secrets", () => {
    expectRejects({ CRON_SECRET: "change-me-please-now" }, "CRON_SECRET");
    expectRejects({ PADDLE_API_KEY: "pdl_sdbx_apikey_placeholder_xx" }, "PADDLE_API_KEY");
    expectRejects({ SUPABASE_SERVICE_ROLE_KEY: "your-secret-key-goes-here", SMG_DB: "supabase", SUPABASE_URL: "https://abc.supabase.co" }, "SUPABASE_SERVICE_ROLE_KEY");
  });
  it("rejects missing variables", () => {
    expectRejects({ RESEND_API_KEY: undefined }, "RESEND_API_KEY");
    expectRejects({ ADMIN_ALERT_EMAIL: "" }, "ADMIN_ALERT_EMAIL");
  });
  it("rejects sandbox/production prefix mismatches", () => {
    expectRejects({ PADDLE_ENV: "production", NEXT_PUBLIC_PADDLE_ENV: "production", NEXT_PUBLIC_PADDLE_CLIENT_TOKEN: "live_01clienttoken", SMG_DB: "supabase", SUPABASE_URL: "https://abc.supabase.co", SUPABASE_SERVICE_ROLE_KEY: "eyJ.service.role.key.value" }, "PADDLE_API_KEY: PADDLE_ENV=production needs a key starting pdl_live_");
    expectRejects({ PADDLE_API_KEY: "pdl_live_apikey_01testkey_abcdefghijklmnop" }, "PADDLE_API_KEY: PADDLE_ENV=sandbox needs a key starting pdl_sdbx_");
    expectRejects({ NEXT_PUBLIC_PADDLE_ENV: "production" }, "NEXT_PUBLIC_PADDLE_ENV: must equal PADDLE_ENV");
    expectRejects({ NEXT_PUBLIC_PADDLE_CLIENT_TOKEN: "live_01clienttoken" }, "NEXT_PUBLIC_PADDLE_CLIENT_TOKEN");
    expectRejects({ PADDLE_WEBHOOK_SECRET: "whsec_not_paddle_format_xx" }, "PADDLE_WEBHOOK_SECRET");
  });
  it("accepts a consistent production configuration", () => {
    const env = parseEnv(
      testEnvRecord({
        PADDLE_ENV: "production",
        NEXT_PUBLIC_PADDLE_ENV: "production",
        PADDLE_API_KEY: "pdl_live_apikey_01realkey_abcdefghijklmnop",
        NEXT_PUBLIC_PADDLE_CLIENT_TOKEN: "live_01clienttoken",
        SMG_DB: "supabase",
        SUPABASE_URL: "https://abc.supabase.co",
        SUPABASE_SERVICE_ROLE_KEY: "eyJ.service.role.key.value",
      }),
    );
    expect(env.PADDLE_ENV).toBe("production");
  });
  it("refuses the memory database in production and requires Supabase otherwise", () => {
    expectRejects({ PADDLE_ENV: "production", NEXT_PUBLIC_PADDLE_ENV: "production", PADDLE_API_KEY: "pdl_live_apikey_01realkey_abcdefghijklmnop", NEXT_PUBLIC_PADDLE_CLIENT_TOKEN: "live_01clienttoken" }, "SMG_DB");
    expectRejects({ SMG_DB: "supabase" }, "SUPABASE_URL");
  });
  it("checks key lengths", () => {
    expectRejects({ KEY_ENCRYPTION_KEY: Buffer.alloc(16, 1).toString("base64") }, "KEY_ENCRYPTION_KEY");
    expectRejects({ IP_HASH_SALT: Buffer.alloc(32, 1).toString("base64") }, "IP_HASH_SALT");
    expectRejects({ LICENSE_SIGNING_KEY: "not base64!!" }, "LICENSE_SIGNING_KEY");
    expectRejects({ LICENSE_SIGNING_KID: "oct" }, "LICENSE_SIGNING_KID");
  });
});
