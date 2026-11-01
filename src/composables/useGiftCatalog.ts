import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
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

// Cadeaux dont un mot-clé apparaît dans le texte (titre d'un live)
export function suggestGifts(gifts: Gift[], text: string | null | undefined): Gift[] {
  if (!text) return []
  const t = normalize(text)
  return gifts.filter(g => g.keywords.some(k => k && t.includes(normalize(k))))
}

function normalize(s: string) {
  return s.toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '')
}
