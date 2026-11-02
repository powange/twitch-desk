// Service worker de la web app Twitch Desk (portée /app/).
// Coquille de l'application gardée en cache : la page s'ouvre même sans réseau (les données
// viennent toujours du serveur). Les fichiers /app/assets/… sont nommés par leur empreinte :
// gardés tels quels ; index.html est relu sur le réseau d'abord. L'API n'est jamais mise en cache.
// Prochaine étape : notifications push (événements « push » et « notificationclick » ici).

const CACHE = 'twitch-desk-v1'
const SHELL = ['/app/', '/app/manifest.webmanifest', '/app/icon-192.png', '/app/icon-512.png', '/app/icon.svg']

self.addEventListener('install', (event) => {
  event.waitUntil(caches.open(CACHE).then(cache => cache.addAll(SHELL)).then(() => self.skipWaiting()))
})

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then(keys => Promise.all(keys.filter(k => k !== CACHE).map(k => caches.delete(k))))
      .then(() => self.clients.claim())
  )
})

self.addEventListener('fetch', (event) => {
  const request = event.request
  const url = new URL(request.url)
  if (request.method !== 'GET' || url.origin !== self.location.origin || !url.pathname.startsWith('/app/')) return
  // Page : réseau d'abord (nouvelle version), cache sans réseau
  if (request.mode === 'navigate') {
    event.respondWith(
      fetch(request)
        .then((response) => {
          if (response.ok) {
            const copy = response.clone()
            caches.open(CACHE).then(cache => cache.put('/app/', copy))
          }
          return response
        })
        .catch(() => caches.match('/app/').then(r => r || Response.error()))
    )
    return
  }
  // Fichiers à empreinte : cache d'abord
  if (url.pathname.startsWith('/app/assets/')) {
    event.respondWith(
      caches.match(request).then(cached => cached || fetch(request).then((response) => {
        if (response.ok) {
          const copy = response.clone()
          caches.open(CACHE).then(cache => cache.put(request, copy))
        }
        return response
      }))
    )
  }
})
