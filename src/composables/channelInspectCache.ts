// Fiches de chaîne déjà lues par le formulaire de giveaway (profil Twitch, fiche du streamer,
// giveaway StreamElements actif…), gardées pour la session : affichées tout de suite à la
// réouverture, puis remplacées par la relecture
const MAX = 100
const cache = new Map<string, unknown>()

export function cachedInspect<T>(login: string): T | undefined {
  return cache.get(login.trim().toLowerCase()) as T | undefined
}

export function storeInspect<T>(login: string, value: T) {
  const key = login.trim().toLowerCase()
  cache.delete(key)
  cache.set(key, value)
  // Les plus anciennes lues en premier : retirées au-delà de MAX
  while (cache.size > MAX) cache.delete(cache.keys().next().value!)
}
