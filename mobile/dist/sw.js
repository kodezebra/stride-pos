// Offline shell. Hand-written: the whole strategy is 80 lines and Workbox is
// 30 KB plus a build step to configure.
//
// Bump CACHE_VERSION on every release. Filenames are not content-hashed, so the
// version string is the only thing that tells a phone its copy is stale.

const CACHE_VERSION = 'v1';
const CACHE = `till-${CACHE_VERSION}`;

// Must match `vite build` output exactly (vite.config.js pins the names). If a
// file here 404s, `install` rejects and the worker never activates — which is
// loud on purpose: a half-cached app that opens offline to a blank screen is
// worse than one that refuses to install.
const PRECACHE = [
  '/',
  '/index.html',
  '/manifest.webmanifest',
  '/assets/main.js',
  '/assets/main.css',
  '/icons/icon-192.png',
  '/icons/icon-512.png',
  '/offline.html',
];

self.addEventListener('install', (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE);
      await cache.addAll(PRECACHE);
      await self.skipWaiting();
    })(),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(
        keys.filter((key) => key.startsWith('till-') && key !== CACHE).map((key) => caches.delete(key)),
      );
      await self.clients.claim();
    })(),
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  if (request.method !== 'GET') return;

  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  // A navigation goes network-first so a deploy is picked up on the next launch
  // when there is signal, and falls back to the cached shell when there is not.
  if (request.mode === 'navigate') {
    event.respondWith(navigation(request));
    return;
  }

  event.respondWith(cacheFirst(request));
});

async function navigation(request) {
  try {
    const response = await fetch(request);
    if (response.ok) {
      const cache = await caches.open(CACHE);
      await cache.put('/index.html', response.clone());
    }
    return response;
  } catch {
    const cache = await caches.open(CACHE);
    return (await cache.match('/index.html')) ?? (await cache.match('/offline.html')) ?? Response.error();
  }
}

async function cacheFirst(request) {
  const cache = await caches.open(CACHE);
  const hit = await cache.match(request);
  if (hit) return hit;

  try {
    const response = await fetch(request);
    if (response.ok) await cache.put(request, response.clone());
    return response;
  } catch {
    return Response.error();
  }
}
