// Service worker de la web app Twitch Desk (portée /app/).
// Coquille de l'application gardée en cache : la page s'ouvre même sans réseau (les données
// viennent toujours du serveur). Les fichiers /app/assets/… sont nommés par leur empreinte :
// gardés tels quels ; index.html est relu sur le réseau d'abord. L'API n'est jamais mise en cache.
// Notifications push : envoyées par le serveur (alertes de l'application du PC), affichées
// même web app fermée ; un appui ouvre la web app.

const CACHE = 'twitch-desk-v2'
// Jeton et alertes de l'appareil, écrits par la page (push.ts) : réabonnement sans elle
const PUSH_CACHE = 'twitch-desk-push'
const PUSH_KEY = '/app/__push'
const SHELL = ['/app/', '/app/manifest.webmanifest', '/app/icon-192.png', '/app/icon-512.png', '/app/icon.svg']

self.addEventListener('install', (event) => {
  event.waitUntil(caches.open(CACHE).then(cache => cache.addAll(SHELL)).then(() => self.skipWaiting()))
})

// Fichiers à empreinte des versions précédentes : retirés du cache (sinon il grossit à chaque
// déploiement). Gardés : ceux que cite la page actuelle, et ceux que citent ses scripts (morceaux
// chargés à la demande). Sans réseau : rien n'est retiré
async function dropOldAssets() {
  const response = await fetch('/app/', { cache: 'no-store' })
  if (!response.ok) return
  const page = await response.text()
  const cache = await caches.open(CACHE)
  await cache.put('/app/', new Response(page, { headers: response.headers }))
  const entries = [...new Set(page.match(/\/app\/assets\/[^"'\s)]+\.js/g) || [])]
  const scripts = await Promise.all(entries.map(p =>
    cache.match(p).then(r => r || fetch(p)).then(r => r.ok ? r.text() : '').catch(() => null)))
  // Un script illisible : on ne sait pas ce qu'il charge, rien n'est retiré
  if (scripts.includes(null)) return
  const cited = page + scripts.join('\n')
  const requests = await cache.keys()
  await Promise.all(requests
    .filter((r) => {
      const path = new URL(r.url).pathname
      return path.startsWith('/app/assets/') && !cited.includes(path.slice('/app/assets/'.length))
    })
    .map(r => cache.delete(r)))
}

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then(keys => Promise.all(keys.filter(k => k !== CACHE && k !== PUSH_CACHE).map(k => caches.delete(k))))
      .then(() => dropOldAssets().catch(() => {}))
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
            return response
          }
          // Serveur qui redémarre (502, 503… du proxy) : la web app en cache plutôt que la page
          // d'erreur ; elle réessaie et affiche sa dernière vue
          if (response.status >= 500) return caches.match('/app/').then(r => r || response)
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

self.addEventListener('push', (event) => {
  let data = {}
  try {
    data = event.data ? event.data.json() : {}
  } catch {
    data = { body: event.data ? event.data.text() : '' }
  }
  const title = data.title || 'Twitch Desk'
  // Onglet ouvert par un appui sur la notification
  const tab = data.alert === 'drop' ? 'drops' : data.alert === 'scanner' ? 'scanner' : 'giveaways'
  // Mention : section Mentions de l'accueil
  const focus = data.alert === 'mention' ? 'mentions' : null
  // Cible (serveur 0.8.13) : chaîne du giveaway, du gain, de la mention ou du stream repéré ;
  // compte du drop
  const target = typeof data.target === 'string' && /^[a-z0-9_]{1,25}$/.test(data.target) ? data.target : null
  event.waitUntil(self.registration.showNotification(title, {
    body: data.body || '',
    icon: '/app/icon-192.png',
    lang: 'fr',
    data: { url: `/app/?tab=${tab}${focus ? `&focus=${focus}` : ''}${target ? `&target=${target}` : ''}`, tab, focus, target }
  }))
})

// Abonnement renouvelé par le navigateur (Chrome, Firefox) : nouvel abonnement envoyé au serveur,
// sinon plus aucune notification n'arriverait jusqu'à la réouverture de la web app
function keyBytes(base64url) {
  const b64 = (base64url + '='.repeat((4 - base64url.length % 4) % 4)).replace(/-/g, '+').replace(/_/g, '/')
  return Uint8Array.from(atob(b64), c => c.charCodeAt(0))
}
self.addEventListener('pushsubscriptionchange', (event) => {
  event.waitUntil((async () => {
    const saved = await caches.open(PUSH_CACHE).then(c => c.match(PUSH_KEY)).then(r => r ? r.json() : null).catch(() => null)
    if (!saved || !saved.token) return
    const sub = event.newSubscription || await self.registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: keyBytes(saved.publicKey)
    })
    const json = sub.toJSON()
    await fetch('/api/v1/app/push', {
      method: 'PUT',
      headers: { 'Authorization': `Device ${saved.token}`, 'Content-Type': 'application/json' },
      body: JSON.stringify({ endpoint: json.endpoint, keys: json.keys, events: saved.events })
    })
  })().catch(() => {}))
})

// Appui sur une notification : web app déjà ouverte mise au premier plan, sinon ouverte
self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const data = event.notification.data || {}
  const url = data.url || '/app/'
  event.waitUntil(
    self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then((list) => {
      const client = list.find(c => new URL(c.url).pathname.startsWith('/app/'))
      if (client) {
        if (data.tab) client.postMessage({ tab: data.tab, focus: data.focus, target: data.target })
        return client.focus()
      }
      return self.clients.openWindow(url)
    })
  )
})
