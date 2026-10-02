import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

// Mirrors LicensesData in packages/ui/src/components/LicensesPage.tsx.
interface LicenseEntry {
  name: string;
  version?: string;
  license: string;
  copyright?: string;
  url?: string;
  text?: string;
  note?: string;
}
interface LicensesData {
  generated_at: string;
  groups: { title: string; entries: LicenseEntry[] }[];
}

const GROUPS = ["Downloaded with your permission", "Built into Smidge", "Patent licences", "Fonts"];
const ALLOWED = [
  "MIT",
  "Apache-2.0",
  "Apache-2.0 WITH LLVM-exception",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Zlib",
  "0BSD",
  "BSL-1.0",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "CC0-1.0",
  "IJG",
  "MPL-2.0",
  "OFL-1.1",
  "bzip2-1.0.6",
  "CDLA-Permissive-2.0",
];

const here = dirname(fileURLToPath(import.meta.url));
const data = JSON.parse(readFileSync(join(here, "..", "licenses.json"), "utf8")) as LicensesData;
const builtIn = data.groups.find((g) => g.title === "Built into Smidge")!;

describe("licenses.json", () => {
  it("has a date-only generated_at", () => {
    expect(data.generated_at).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it("has the four groups in DESIGN 4.11 order, none empty", () => {
    expect(data.groups.map((g) => g.title)).toEqual(GROUPS);
    for (const g of data.groups) expect(g.entries.length, g.title).toBeGreaterThan(0);
  });

  it("every entry matches the UI shape", () => {
    for (const g of data.groups)
      for (const e of g.entries) {
        expect(typeof e.name, `${g.title}/${e.name}`).toBe("string");
        expect(e.name.length).toBeGreaterThan(0);
        expect(typeof e.license).toBe("string");
        expect(e.license.length).toBeGreaterThan(0);
        for (const k of ["version", "copyright", "url", "text", "note"] as const)
          if (e[k] !== undefined) expect(typeof e[k], `${e.name}.${k}`).toBe("string");
        const extra = Object.keys(e).filter(
          (k) => !["name", "version", "license", "copyright", "url", "text", "note"].includes(k),
        );
        expect(extra, `${e.name} has unknown keys`).toEqual([]);
      }
  });

  it("entries are sorted by name within each group", () => {
    // Same comparator as tools/licenses/merge.mjs.
    const cmp = (a: LicenseEntry, b: LicenseEntry) => {
      const n = a.name.localeCompare(b.name, "en", { sensitivity: "base" });
      if (n) return n;
      if (a.name !== b.name) return a.name < b.name ? -1 : 1;
      return (a.version ?? "").localeCompare(b.version ?? "", "en", { numeric: true });
    };
    for (const g of data.groups) {
      const keys = (xs: LicenseEntry[]) => xs.map((e) => `${e.name} ${e.version ?? ""}`);
      expect(keys(g.entries), g.title).toEqual(keys([...g.entries].sort(cmp)));
    }
  });

  it("every built-in component has a version, an allowed licence and full text", () => {
    for (const e of builtIn.entries) {
      expect(e.version, e.name).toBeTruthy();
      expect(e.text?.trim().length ?? 0, `${e.name} text`).toBeGreaterThan(100);
      for (const id of e.license.split(" AND "))
        expect(ALLOWED, `${e.name} licence ${id}`).toContain(id);
    }
  });

  it("contains the crates and packages DESIGN 10 says ship", () => {
    const names = new Set(builtIn.entries.map((e) => e.name));
    for (const n of [
      "image",
      "mozjpeg-sys",
      "libwebp-sys",
      "oxipng",
      "rav1e",
      "ravif",
      "symphonia",
      "zip",
      "react",
      "mediabunny",
      "idb-keyval",
      "lucide-react",
    ])
      expect(names, n).toContain(n);
    for (const n of [
      "cia-core",
      "cia-cli",
      "xtask",
      "vitest",
      "vite",
      "typescript",
      "@types/react",
      "@fontsource/inter",
    ])
      expect(names, n).not.toContain(n);
  });

  it("MPL-2.0 components say they are used unmodified and link to source", () => {
    for (const e of builtIn.entries.filter((e) => e.license.includes("MPL-2.0")))
      expect(e.note, e.name).toMatch(/Used unmodified\. Source: https?:\/\//);
  });

  it("carries the IJG notice on mozjpeg", () => {
    const m = builtIn.entries.find((e) => e.name === "mozjpeg-sys")!;
    expect(m.license.split(" AND ")).toContain("IJG");
    expect(m.note).toContain(
      "This software is based in part on the work of the Independent JPEG Group",
    );
  });

  it("lists FFmpeg as LGPL in the download group, and both fonts under OFL-1.1", () => {
    const dl = data.groups[0].entries;
    expect(dl.find((e) => e.name === "FFmpeg")?.license).toBe("LGPL-3.0-or-later");
    const fonts = data.groups[3].entries;
    expect(fonts.map((e) => e.name).sort()).toEqual(["Bricolage Grotesque", "Inter"]);
    for (const f of fonts) expect(f.license).toBe("OFL-1.1");
  });
});
