import { computed, ref } from 'vue'
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

export function useGiftCatalog() {
  if (!catalog.value && !loading) loading = refresh().finally(() => { loading = null })
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

// Cadeaux dont un mot-clé apparaît dans le texte (titre d'un live)
export function suggestGifts(gifts: Gift[], text: string | null | undefined): Gift[] {
  if (!text) return []
  const t = normalize(text)
  return gifts.filter(g => g.keywords.some(k => k && t.includes(normalize(k))))
}

function normalize(s: string) {
  return s.toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '')
}
