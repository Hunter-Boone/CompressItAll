#!/usr/bin/env node
// Merges cargo-about output, the npm licence scan and extra.json into
// packages/licenses-data/licenses.json and NOTICE.md (DESIGN.md 4.11).
// Invoked by tools/licenses/generate.sh; do not run by hand.
//
// Output is deterministic: entries are sorted, nothing varies except
// `generated_at`, which generate.sh pins in --check mode.

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { basename, join } from "node:path";

const args = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, all) => {
    if (a.startsWith("--"))
      acc.push([a.slice(2), all[i + 1] && !all[i + 1].startsWith("--") ? all[i + 1] : true]);
    return acc;
  }, []),
);
const need = (k) => {
  if (!args[k]) throw new Error(`missing --${k}`);
  return args[k];
};

const ROOT = need("root");
const TEXTS = need("texts");
const extra = JSON.parse(readFileSync(need("extra"), "utf8"));
const generatedAt = need("generated-at");
const rustInputs = need("rust").split(",").filter(Boolean);
const rustTrees = need("rust-trees").split(",").filter(Boolean);
const npmTree = JSON.parse(readFileSync(need("npm-tree"), "utf8"));
const npmLicenses = JSON.parse(readFileSync(need("npm-licenses"), "utf8"));

// DESIGN.md 7.7. Priority order: the first match of an OR expression is the licence we elect.
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
const GROUP_ORDER = [
  "Downloaded with your permission",
  "Built into Smidge",
  "Patent licences",
  "Fonts",
];

const problems = [];
const warnings = [];

// ---------- helpers ----------
const PLACEHOLDER =
  /<year>|\[year\]|<copyright holders?>|\[fullname\]|\[name of copyright owner\]|<owner>|yyyy|<name of author>|\[yyyy\]/i;
function copyrightLine(text) {
  if (!text) return undefined;
  for (const raw of text.split(/\r?\n/)) {
    let line = raw.replace(/^[\s*#/-]+/, "").trim();
    // A real notice: "Copyright (c) 2020 Someone", "Copyright © Unicode, Inc.", "Copyright 2016 The Authors".
    if (!/^(copyright\s+(\(c\)|©|\d{4}|[A-Z])|©\s*\S)/i.test(line)) continue;
    // Not the Apache definition ("copyright notice that is...") or the Unicode header ("COPYRIGHT AND PERMISSION NOTICE").
    if (/^copyright\s+(and|notice|holder|owner|law|in|of|to|license|licence)\b/i.test(line))
      continue;
    if (PLACEHOLDER.test(line)) continue;
    if (line.length < 12) continue;
    // Font licences repeat the notice per file: keep the first.
    const second = line.slice(10).search(/copyright/i);
    if (second > 0)
      line = line
        .slice(0, second + 10)
        .trim()
        .replace(/\s+\S+:$/, "");
    return line.replace(/\s+/g, " ").slice(0, 200);
  }
  return undefined;
}
function canonicalText(id) {
  const p = join(TEXTS, `${id}.txt`);
  if (!existsSync(p)) {
    problems.push(`no cached licence text for ${id} (tools/licenses/texts/${id}.txt)`);
    return "";
  }
  return readFileSync(p, "utf8").replace(/\r\n/g, "\n").trim();
}
function matches(pattern, name) {
  return pattern.endsWith("*") ? name.startsWith(pattern.slice(0, -1)) : name === pattern;
}
function lookup(map, name) {
  for (const [k, v] of Object.entries(map || {})) if (matches(k, name)) return v;
  return undefined;
}
function stripAuthor(a) {
  return a
    .replace(/\s*<[^>]*>/g, "")
    .replace(/\s*\([^)]*\)/g, "")
    .trim();
}
// Resolve a simple SPDX expression (parentheses, AND, OR, WITH) to the ids we use the package under.
function resolveExpression(expr, where) {
  const flat = expr.replace(/[()]/g, " ").replace(/\s+/g, " ").trim();
  const ands = flat.split(/ AND /i);
  const chosen = [];
  for (const term of ands) {
    const options = term.split(/ OR /i).map((s) => s.trim());
    const pick = ALLOWED.find((a) => options.includes(a));
    if (!pick) {
      problems.push(`${where}: licence "${expr}" offers nothing in the allowed list`);
      chosen.push(options[0]);
    } else chosen.push(pick);
  }
  return [...new Set(chosen)];
}
function declaredNote(declared, resolved) {
  if (!declared) return undefined;
  const norm = (s) =>
    s
      .replace(/[()]/g, "")
      .replace(/\s*\/\s*/g, " OR ")
      .replace(/\s+/g, " ")
      .trim();
  if (norm(declared) === resolved.join(" AND ")) return undefined;
  return `Declared ${norm(declared)}; used under ${resolved.join(" AND ")}.`;
}
function joinNotes(...parts) {
  const xs = parts.filter(Boolean);
  return xs.length ? xs.join(" ") : undefined;
}
function entry(o) {
  const { text_file, ...ov } = lookup(extra.overrides, o.name) || {};
  o = { ...o, ...ov, note: joinNotes(ov.note, o.note) };
  if (text_file) o.text = canonicalText(text_file.replace(/\.txt$/, ""));
  const e = { name: o.name };
  if (o.version) e.version = o.version;
  e.license = o.license;
  if (o.copyright) e.copyright = o.copyright;
  if (o.url) e.url = o.url;
  if (o.text) e.text = o.text;
  if (o.note) e.note = o.note;
  return e;
}
function cmp(a, b) {
  const n = a.name.localeCompare(b.name, "en", { sensitivity: "base" });
  if (n) return n;
  if (a.name !== b.name) return a.name < b.name ? -1 : 1;
  return (a.version || "").localeCompare(b.version || "", "en", { numeric: true });
}

// ---------- Rust ----------
// The set of crates that reach a shipped artefact (cargo tree per target), minus our own.
const shipped = new Set();
for (const f of rustTrees) {
  for (const line of readFileSync(f, "utf8").split("\n")) {
    const m = /^(\S+) v(\S+)/.exec(line.trim());
    if (!m) continue;
    if (m[1].startsWith("cia-") || m[1] === "xtask") continue;
    shipped.add(`${m[1]}@${m[2]}`);
  }
}
if (!shipped.size) problems.push("cargo tree produced no crates");

// crate@version -> { meta, licences: Map<id, text[]> }
const crates = new Map();
for (const f of rustInputs) {
  const about = JSON.parse(readFileSync(f, "utf8"));
  for (const lic of about.licenses) {
    for (const u of lic.used_by) {
      const key = `${u.name}@${u.version}`;
      if (!shipped.has(key)) continue;
      let c = crates.get(key);
      if (!c) crates.set(key, (c = { meta: u, licences: new Map() }));
      const texts = c.licences.get(lic.id) || [];
      texts.push(lic.text.replace(/\r\n/g, "\n").trim());
      c.licences.set(lic.id, texts);
    }
  }
}
for (const key of shipped)
  if (!crates.has(key)) problems.push(`shipped crate ${key} has no licence from cargo-about`);

const rustEntries = [];
for (const [key, c] of crates) {
  const ids = [...c.licences.keys()].sort((a, b) => ALLOWED.indexOf(a) - ALLOWED.indexOf(b));
  for (const id of ids)
    if (!ALLOWED.includes(id)) problems.push(`${key}: licence ${id} is not in the allowed list`);
  // One text per licence id: prefer a text carrying a copyright line, then the shortest.
  const pickText = (texts) =>
    [...texts].sort(
      (a, b) =>
        (copyrightLine(b) ? 1 : 0) - (copyrightLine(a) ? 1 : 0) ||
        a.length - b.length ||
        a.localeCompare(b),
    )[0];
  const texts = ids.map((id) => pickText(c.licences.get(id)));
  const text =
    ids.length === 1
      ? texts[0]
      : ids.map((id, i) => `${id}\n${"-".repeat(id.length)}\n\n${texts[i]}`).join("\n\n\n");
  const copyright =
    texts.map(copyrightLine).find(Boolean) ||
    (c.meta.authors?.length
      ? c.meta.authors.map(stripAuthor).filter(Boolean).join(", ")
      : undefined);
  const license = ids.join(" AND ");
  const [name, version] = [c.meta.name, c.meta.version];
  const mpl = ids.includes("MPL-2.0");
  const note = joinNotes(lookup(extra.notes, name), declaredNote(c.meta.license, ids));
  if (mpl && !/Used unmodified\. Source: /.test(note || ""))
    problems.push(
      `${key} is MPL-2.0 but has no "Used unmodified. Source: <url>" note in extra.json`,
    );
  rustEntries.push(
    entry({
      name,
      version,
      license,
      copyright,
      url: c.meta.repository || c.meta.homepage || undefined,
      text,
      note,
    }),
  );
}

// ---------- npm ----------
// Production dependencies reachable from apps/web, skipping excluded subtrees and our own packages.
const npmShipped = new Map(); // name@version -> name
function walkNpm(node) {
  for (const [name, dep] of Object.entries(node.dependencies || {})) {
    // npm 10 installs some platform-gated optional packages (e.g. @img/sharp-wasm32) and lists
    // them at the root as extraneous; they are not reachable from apps/web and not shipped.
    if (dep.extraneous) continue;
    if (!dep.version) {
      problems.push(`npm ls: ${name} has no version (not installed?)`);
      continue;
    }
    if (lookup(extra.exclude, name)) continue;
    if (!name.startsWith("@cia/")) npmShipped.set(`${name}@${dep.version}`, name);
    walkNpm(dep);
  }
}
// `npm ls -w apps/web` puts the workspace under the root; walk only that subtree.
const webRoot = (npmTree.dependencies || {})["@cia/web"];
if (!webRoot) problems.push("npm ls: @cia/web missing from the tree");
walkNpm(webRoot || npmTree);
if (!npmShipped.size) problems.push("npm ls produced no packages");

const npmEntries = [];
const fontEntries = [];
for (const [key, name] of npmShipped) {
  const info = npmLicenses[key];
  if (!info) {
    problems.push(`npm package ${key} missing from the license-checker scan`);
    continue;
  }
  const version = key.slice(name.length + 1);
  const declared = Array.isArray(info.licenses)
    ? info.licenses.join(" OR ")
    : String(info.licenses || "");
  if (!declared || /UNKNOWN|UNLICENSED/i.test(declared))
    problems.push(`${key}: licence unknown ("${declared}")`);
  const ids = resolveExpression(declared, key);
  let text = "";
  let copyright;
  const file =
    info.licenseFile && /licen[cs]e|copying/i.test(basename(info.licenseFile))
      ? info.licenseFile
      : undefined;
  if (file && existsSync(file)) {
    text = readFileSync(file, "utf8").replace(/\r\n/g, "\n").trim();
    copyright = copyrightLine(text);
  } else {
    text = ids.map((id) => canonicalText(id)).join("\n\n\n");
  }
  if (!copyright && info.publisher) copyright = `Copyright (c) ${stripAuthor(info.publisher)}`;
  const url = info.repository || info.url || undefined;
  const fontName = extra.fonts?.[name];
  const mpl = ids.includes("MPL-2.0");
  const note = joinNotes(lookup(extra.notes, name), declaredNote(declared, ids));
  if (mpl && !/Used unmodified\. Source: /.test(note || ""))
    problems.push(
      `${key} is MPL-2.0 but has no "Used unmodified. Source: <url>" note in extra.json`,
    );
  const e = entry({
    name: fontName || name,
    version,
    license: ids.join(" AND "),
    copyright,
    url,
    text,
    note: fontName ? joinNotes(`Packaged by Fontsource as ${name}.`, note) : note,
  });
  (fontName ? fontEntries : npmEntries).push(e);
}

// ---------- extra ----------
function extraEntries(title) {
  return (extra.groups[title] || []).map((o) => {
    const { text_file, ...rest } = o;
    const text = text_file ? canonicalText(text_file.replace(/\.txt$/, "")) : rest.text;
    return entry({ ...rest, text });
  });
}

const groups = GROUP_ORDER.map((title) => {
  let entries = extraEntries(title);
  if (title === "Built into Smidge") entries = entries.concat(rustEntries, npmEntries);
  if (title === "Fonts") entries = entries.concat(fontEntries);
  entries.sort(cmp);
  if (!entries.length) problems.push(`group "${title}" is empty`);
  return { title, entries };
});

for (const g of groups)
  for (const e of g.entries) {
    if (!e.text) problems.push(`${g.title} / ${e.name}: no licence text`);
    if (g.title === "Built into Smidge" && !e.copyright)
      warnings.push(
        `${e.name}@${e.version}: no copyright line in its licence file and no authors in Cargo.toml`,
      );
  }

for (const w of warnings) console.error("warning: " + w);
if (problems.length) {
  console.error("licence generation failed:");
  for (const p of [...new Set(problems)]) console.error("  - " + p);
  process.exit(1);
}

const data = { generated_at: generatedAt, groups };
writeFileSync(need("out-json"), JSON.stringify(data, null, 2) + "\n");

// ---------- NOTICE.md ----------
const line = (e) => {
  const parts = [`${e.name}${e.version ? " " + e.version : ""}`, e.license];
  if (e.copyright) parts.push(e.copyright);
  const note = (e.note || "").replace(/\s*Declared [^;]+; used under [^.]+\.$/, "").trim();
  if (note) parts.push(note);
  return `- ${parts.join(" — ")}`;
};
const byTitle = Object.fromEntries(groups.map((g) => [g.title, g.entries]));
const ffmpeg = byTitle["Downloaded with your permission"].find((e) => e.name === "FFmpeg");
const bundled = byTitle["Downloaded with your permission"].filter((e) => e.name !== "FFmpeg");
const counts = groups.map((g) => `${g.entries.length} in "${g.title}"`).join(", ");

const notice = `# Smidge - Third-Party Software Notices and Information

Generated by \`cargo xtask licenses\` from \`packages/licenses-data/licenses.json\` on ${generatedAt}.
Do not edit by hand: change \`tools/licenses/extra.json\` or the dependency manifests and regenerate.
The same data, with every licence text, is shown in the app under Settings → About → Licenses.

Smidge compiles permissively licensed Rust crates and npm packages into the desktop app and the
web engine. Video work on the desktop uses FFmpeg, which is never embedded in the application: it is
downloaded on demand with the user's consent and invoked as a separate command-line process. This
document lists each component, its licence, and how Smidge complies with that licence for commercial
distribution (${counts}).

## Downloaded with your permission

### FFmpeg

**Website**: https://ffmpeg.org/
**License**: GNU Lesser General Public License (LGPL) version 3 or later (LGPL v2.1+ sources built
with \`--enable-version3\`)
**License URL**: https://www.gnu.org/licenses/lgpl-3.0.html
**Build system and sources**: ${ffmpeg.url}

FFmpeg is a complete, cross-platform solution to record, convert and stream audio, video, and
subtitles. Smidge downloads FFmpeg binaries that are built from unmodified official FFmpeg sources
by the Smidge-Libraries build system, only after the user agrees to the download.

#### License compliance

- The build is **LGPL-only**: \`--enable-gpl\` and \`--enable-nonfree\` are NOT used, and no
  GPL-licensed codecs (x264, x265, fdk-aac, etc.) are included. The exact configure flags are
  published in \`Smidge-Libraries/ffmpeg/BUILD_CONFIG.txt\`.
- External codec libraries included in the build (listed below) are all under LGPL/BSD-style
  licenses compatible with LGPL binary distribution.
- The **corresponding source tarball is published alongside the binaries** in every
  Smidge-Libraries release, satisfying the LGPL source-availability requirement.
- Smidge invokes \`ffmpeg\` and \`ffprobe\` as standalone CLI processes and does not link against
  FFmpeg's libraries, statically or dynamically. FFmpeg remains the property of its authors; see
  https://www.ffmpeg.org/legal.html.
- The H.264 and HEVC encoders FFmpeg uses come from the operating system or the GPU driver
  (Media Foundation, VideoToolbox, NVENC, AMF, Quick Sync), never from GPL or patent-encumbered
  software libraries.

${line(ffmpeg)}

### Libraries inside the FFmpeg build

${bundled.map(line).join("\n")}

## Built into Smidge

Rust crates compiled into the desktop binary and the WASM engine, and npm packages in the web
bundle. Dual-licensed crates are used under the first licence named; MPL-2.0 components are used
unmodified and their source is linked. The notice required by the Independent JPEG Group licence
is attached to mozjpeg-sys. Development-only tools are not listed.

${byTitle["Built into Smidge"].map(line).join("\n")}

## Patent licences

${byTitle["Patent licences"].map(line).join("\n")}

## Fonts

${byTitle["Fonts"].map(line).join("\n")}

## Compliance summary

1. **Separate processes**: FFmpeg runs as a standalone executable; Smidge does not statically or
   dynamically link to its libraries.
2. **No GPL components**: nothing GPL, AGPL or SSPL is compiled into Smidge, and the downloaded
   FFmpeg build excludes all GPL and non-free components, so commercial distribution of Smidge is
   not subject to GPL obligations.
3. **Source availability**: FFmpeg build sources and configuration are published with every
   Smidge-Libraries release, and MPL-2.0 components link to their unmodified upstream source.
4. **Attribution**: every licence text and copyright notice above is reproduced inside the
   application (Settings → About → Licenses).
5. **Transparency**: download sources are pinned to signed Smidge-Libraries releases and documented
   in \`docs/DESIGN.md\` section 6.4.
`;
writeFileSync(need("out-notice"), notice);
console.log(`licences: ${counts}`);
