// Cache-first for the shell; network-first for API.
const CACHE = 'irrigator-v1';
const SHELL = ['/', '/app.js', '/styles.css', '/manifest.json'];

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)));
  self.skipWaiting();
});

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys().then((keys) => Promise.all(
      keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))
    ))
  );
  self.clients.claim();
});

self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url);
  if (url.pathname.startsWith('/api/')) {
    // Network-first for API; fall through to network always.
    return;
  }
  // Cache-first for shell.
  e.respondWith(
    caches.match(e.request).then((hit) => hit || fetch(e.request).then((r) => {
      const clone = r.clone();
      caches.open(CACHE).then((c) => c.put(e.request, clone));
      return r;
    }).catch(() => caches.match('/')))
  );
});
