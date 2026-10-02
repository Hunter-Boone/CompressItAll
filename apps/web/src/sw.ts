/**
 * Service worker (DESIGN.md 2.7 "Works offline"): precaches the app shell and
 * /engine/* on install, serves immutable assets cache-first and index.html
 * network-first. Built by the `sw` plugin in vite.config.ts, which injects the
 * precache list and a version from the bundle hash.
 */
declare const __PRECACHE__: string[];
declare const __SW_VERSION__: string;

const sw = self as unknown as ServiceWorkerGlobalScope;
const CACHE = `smidge-${__SW_VERSION__}`;
const PRECACHE = __PRECACHE__;

sw.addEventListener("install", (event) => {
  event.waitUntil((async () => {
    const cache = await caches.open(CACHE);
    // Add one at a time so a single missing file does not block install.
    await Promise.all(PRECACHE.map((url) => cache.add(url).catch(() => undefined)));
    await sw.skipWaiting();
  })());
});

sw.addEventListener("activate", (event) => {
  event.waitUntil((async () => {
    for (const key of await caches.keys()) if (key !== CACHE) await caches.delete(key);
    await sw.clients.claim();
  })());
});

const IMMUTABLE = /^\/(assets|engine)\//;

sw.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== sw.location.origin) return;

  if (req.mode === "navigate") {
    event.respondWith((async () => {
      try {
        const fresh = await fetch(req);
        const cache = await caches.open(CACHE);
        void cache.put("/index.html", fresh.clone());
        return fresh;
      } catch {
        return (await caches.match("/index.html")) ?? Response.error();
      }
    })());
    return;
  }

  if (IMMUTABLE.test(url.pathname)) {
    event.respondWith((async () => {
      const hit = await caches.match(req);
      if (hit) return hit;
      const fresh = await fetch(req);
      if (fresh.ok) void (await caches.open(CACHE)).put(req, fresh.clone());
      return fresh;
    })());
    return;
  }

  // Everything else (manifest, icons): cache first, refresh in the background.
  event.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const hit = await cache.match(req);
    const network = fetch(req).then((res) => { if (res.ok) void cache.put(req, res.clone()); return res; }).catch(() => undefined);
    return hit ?? (await network) ?? Response.error();
  })());
});
