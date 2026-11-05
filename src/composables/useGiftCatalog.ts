import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from './useSnapshot'
import type { Gift, GiftCatalog } from '../types'

// Catalogue de cadeaux, partagé par les pages (chargé à la demande : les images
// ne passent pas par l'état envoyé 4 fois par seconde)
const catalog = ref<GiftCatalog | null>(null)
let loading: Promise<void> | null = null

async function refresh() {
  catalog.value = await invoke<GiftCatalog>('get_gift_catalog')
}

let watching = false

export function useGiftCatalog() {
  if (!catalog.value && !loading) loading = refresh().finally(() => { loading = null })
  // Cadeaux reçus d'un serveur de synchronisation : catalogue relu
  if (!watching) {
    watching = true
    const { snapshot } = useSnapshot()
    watch(() => snapshot.value?.sync.revision, (now, before) => {
      if (before !== undefined && now !== before) void refresh()
    })
  }
  const gifts = computed(() => catalog.value?.gifts ?? [])
  const groups = computed(() => catalog.value?.groups ?? [])
  const byId = computed(() => new Map(gifts.value.map(g => [g.id, g])))
  const groupName = (id: number | null) => groups.value.find(g => g.id === id)?.name ?? null
  const usage = (id: number) => catalog.value?.usage[String(id)] ?? 0
  return { catalog, gifts, groups, byId, groupName, usage, refresh }
}

// Catégories Twitch déjà connues : cadeaux, groupes, catégories des giveaways, jeux des drops
export function useKnownCategories() {
  const { gifts, groups } = useGiftCatalog()
  const { snapshot } = useSnapshot()
  return computed(() => {
    const map = new Map<string, string>()
    for (const g of gifts.value) if (g.gameId && g.gameName) map.set(g.gameId, g.gameName)
    for (const g of groups.value) if (g.gameId && g.gameName) map.set(g.gameId, g.gameName)
    for (const g of snapshot.value?.giveaways.games ?? []) map.set(g.gameId, g.name)
    for (const a of snapshot.value?.accounts ?? []) for (const g of a.drops.games) map.set(g.gameId, g.name)
    return [...map].map(([id, name]) => ({ id, name })).sort((a, b) => a.name.localeCompare(b.name))
  })
}

// Cadeaux dont le titre ou un mot-clé apparaît dans le texte (titre d'un live, description…),
// sans tenir compte des majuscules ni des accents, au début d'un mot (« key » trouve « keys »,
// pas « monkey » ; un terme qui commence par un symbole, comme « €-card », n'importe où)
export function suggestGifts(gifts: Gift[], text: string | null | undefined): Gift[] {
  if (!text) return []
  const cache = compiled(gifts)
  const hit = cache.results.get(text)
  if (hit) {
    // Plus récemment utilisé : remis en fin de liste
    cache.results.delete(text)
    cache.results.set(text, hit)
    return hit
  }
  const t = normalize(text)
  const found = cache.patterns.filter(p => p.res.some(re => re.test(t))).map(p => p.gift)
  cache.results.set(text, found)
  // Mémoire bornée : les textes les plus anciens oubliés
  if (cache.results.size > MAX_RESULTS) cache.results.delete(cache.results.keys().next().value!)
  return found
}

// Expressions de chaque cadeau compilées une fois par version du catalogue (le tableau de
// cadeaux change quand le catalogue est relu), résultats retenus par texte
const MAX_RESULTS = 500
interface Compiled { patterns: { gift: Gift, res: RegExp[] }[], results: Map<string, Gift[]> }
const compiledCache = new WeakMap<Gift[], Compiled>()
function compiled(gifts: Gift[]): Compiled {
  let c = compiledCache.get(gifts)
  if (!c) {
    c = {
      patterns: gifts.map(gift => ({
        gift,
        res: [gift.title, ...gift.keywords].map(term => wordStart(normalize(term))).filter((re): re is RegExp => !!re)
      })),
      results: new Map()
    }
    compiledCache.set(gifts, c)
  }
  return c
}

function wordStart(term: string): RegExp | null {
  const words = term.trim().split(/\s+/).filter(Boolean)
  if (!words.length) return null
  // Plusieurs mots : séparés par n'importe quels espaces dans le texte
  const pattern = words.map(w => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('\\s+')
  // Terme commençant par un symbole (« €-card ») : trouvé où qu'il soit (« 20€-card »)
  const before = /^[\p{L}\p{N}]/u.test(words[0]!) ? '(?<![\\p{L}\\p{N}])' : ''
  return new RegExp(`${before}${pattern}`, 'u')
}

function normalize(s: string) {
  return s.toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '')
}
