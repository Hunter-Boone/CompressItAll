import { getDownloads, PLATFORMS } from "@/lib/downloads";

export async function DownloadButtons({ compact = false }: { compact?: boolean }) {
  const d = await getDownloads();
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap gap-3">
        {PLATFORMS.map((p) => (
          <a key={p.key} href={d[p.key]} className={`smg-btn ${compact ? "smg-btn--secondary" : "smg-btn--primary"}`} rel="noopener">
            Download for {p.label}
          </a>
        ))}
      </div>
      <p className="text-sm text-on-surface-subtle">
        {d.live && d.version ? `Version ${d.version}. ` : "Downloads open on the releases page until the first release is out. "}
        Free to use. No account needed.{" "}
        <a href={d.releases} className="smg-link" rel="noopener">All releases</a>
      </p>
    </div>
  );
}
