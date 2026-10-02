// Brand strings and URLs come from packages/brand/brand.json (one file to change on a rename).
import brandJson from "@cia/brand";

export const brand = brandJson as {
  name: string;
  wordmark: string;
  tagline: string;
  promise: string;
  pro_name: string;
  accent: string;
  domain_placeholder: string;
  urls: { site: string; app: string; api: string; downloads_repo: string; libraries_repo: string; support_email: string };
  seller: string;
};

/** `https://raw.githubusercontent.com/<owner>/<repo>/main` for the Downloads repo. */
export function downloadsRawBase(): string {
  const m = /^https:\/\/github\.com\/([^/]+)\/([^/]+?)\/?$/.exec(brand.urls.downloads_repo);
  if (!m) throw new Error("brand.urls.downloads_repo must be a github.com repository URL");
  return `https://raw.githubusercontent.com/${m[1]}/${m[2]}/main`;
}

export function downloadsReleasesUrl(): string {
  return `${brand.urls.downloads_repo.replace(/\/$/, "")}/releases/latest`;
}
