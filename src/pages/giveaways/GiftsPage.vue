<script setup lang="ts">
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useGiftCatalog } from '../../composables/useGiftCatalog'
import GiftFormModal from '../../components/GiftFormModal.vue'
import GiftGroupsModal from '../../components/GiftGroupsModal.vue'
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog'
import { boxArt } from '../../utils/format'
import type { Gift } from '../../types'
import SolidIcon from '../../components/SolidIcon.vue'
import { useAction } from '../../composables/useAction'

const { gifts, groups, usage, refresh } = useGiftCatalog()
const toast = useToast()

const search = ref('')
// Catégorie : null = toutes, '' = sans catégorie
const categoryFilter = ref<string | null>(null)
const groupsOpen = ref(false)

// Catégories présentes dans le catalogue
const categories = computed(() => {
  const map = new Map<string, string>()
  for (const g of gifts.value) if (g.gameId) map.set(g.gameId, g.gameName ?? g.gameId)
  return [...map].map(([id, name]) => ({ id, name, count: gifts.value.filter(g => g.gameId === id).length }))
    .sort((a, b) => a.name.localeCompare(b.name))
})
const uncategorized = computed(() => gifts.value.filter(g => !g.gameId).length)
function selectCategory(id: string | null) {
  categoryFilter.value = id
}

// Favoris : cadeaux recherchés, propres à ce poste
const favoritesOnly = ref(false)
const favoriteCount = computed(() => gifts.value.filter(g => g.favorite).length)
async function toggleFavorite(g: Gift) {
  const value = !g.favorite
  g.favorite = value
  try {
    await invoke('set_gift_favorite', { id: g.id, favorite: value })
  } catch (e) {
    g.favorite = !value
    toast.add({ title: 'Favori non enregistré', description: String(e), color: 'error' })
  }
}

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  return gifts.value.filter((g) => {
    if (favoritesOnly.value && !g.favorite) return false
    if (categoryFilter.value !== null && (g.gameId ?? '') !== categoryFilter.value) return false
    return !q || g.title.toLowerCase().includes(q) || g.keywords.some(k => k.includes(q))
  })
})

// Affichage : par catégorie (sans catégorie à la fin), puis par groupe (sans groupe à la fin)
interface GroupSection { id: number | null, name: string, loose: boolean, gifts: Gift[] }
interface CategorySection { id: string | null, name: string, count: number, groups: GroupSection[] }
const sections = computed<CategorySection[]>(() => {
  const byCategory = new Map<string, Gift[]>()
  for (const g of filtered.value) {
    const key = g.gameId ?? ''
    if (!byCategory.has(key)) byCategory.set(key, [])
    byCategory.get(key)!.push(g)
  }
  return [...byCategory].map(([key, list]) => {
    const byGroup = new Map<number | null, Gift[]>()
    for (const g of list) {
      const group = groups.value.some(x => x.id === g.groupId) ? g.groupId : null
      if (!byGroup.has(group)) byGroup.set(group, [])
      byGroup.get(group)!.push(g)
    }
    const groupSections = [...byGroup].map(([id, items]) => {
      const group = groups.value.find(x => x.id === id)
      return {
        id,
        name: group?.name ?? 'Sans groupe',
        loose: !!group && !group.gameId,
        gifts: items.sort((a, b) => a.title.localeCompare(b.title))
      }
    }).sort((a, b) => (a.id === null ? 1 : 0) - (b.id === null ? 1 : 0) || a.name.localeCompare(b.name))
    return {
      id: key || null,
      name: key ? (list[0]?.gameName ?? key) : 'Sans catégorie',
      count: list.length,
      groups: groupSections
    }
  }).sort((a, b) => (a.id === null ? 1 : 0) - (b.id === null ? 1 : 0) || a.name.localeCompare(b.name))
})

// --- Export / import ---
async function exportGifts() {
  const path = await saveDialog({
    defaultPath: `cadeaux-twitch-desk-${new Date().toISOString().slice(0, 10)}.json`,
    filters: [{ name: 'Catalogue de cadeaux', extensions: ['json'] }]
  })
  if (!path) return
  try {
    const count = await invoke<number>('export_gifts', { path })
    toast.add({ title: `${count} cadeau(x) exporté(s)`, description: path, color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Export impossible', description: String(e), color: 'error' })
  }
}

async function importGifts() {
  const path = await openDialog({ multiple: false, filters: [{ name: 'Catalogue de cadeaux', extensions: ['json'] }] })
  if (!path || Array.isArray(path)) return
  try {
    const r = await invoke<{ added: number, updated: number, skipped: number }>('import_gifts', { path })
    await refresh()
    const parts = [`${r.added} ajouté(s)`, `${r.updated} mis à jour`]
    if (r.skipped) parts.push(`${r.skipped} ignoré(s)`)
    toast.add({ title: 'Catalogue importé', description: parts.join(' · '), color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Import impossible', description: String(e), color: 'error' })
  }
}

const formOpen = ref(false)
const editing = ref<Gift | null>(null)
function openForm(g: Gift | null) {
  editing.value = g
  formOpen.value = true
}

const { run, busy } = useAction()
const toRemove = ref<Gift | null>(null)
async function confirmRemove() {
  const gift = toRemove.value
  if (!gift) return
  if (await run('remove', 'Cadeau non supprimé', () => invoke('delete_gift', { id: gift.id }))) {
    toRemove.value = null
    await refresh().catch(() => {})
  }
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <div class="flex items-center gap-3">
      <h1 class="text-xl font-semibold flex-1">Cadeaux</h1>
      <UButton label="Importer" icon="i-lucide-upload" color="neutral" variant="soft" @click="importGifts" />
      <UButton label="Exporter" icon="i-lucide-download" color="neutral" variant="soft" :disabled="gifts.length === 0" @click="exportGifts" />
      <UButton label="Gérer les groupes" icon="i-lucide-folder-cog" color="neutral" variant="soft" @click="groupsOpen = true" />
      <UButton label="Nouveau cadeau" icon="i-lucide-plus" @click="openForm(null)" />
    </div>

    <div class="flex flex-col gap-2">
      <div class="flex items-center gap-2 flex-wrap">
        <UInput id="gift-search" v-model="search" icon="i-lucide-search" placeholder="Rechercher un cadeau ou un mot-clé" class="w-72" />
        <UButton
          icon="i-lucide-star"
          :label="`Favoris (${favoriteCount})`"
          size="sm"
          :color="favoritesOnly ? 'warning' : 'neutral'"
          :variant="favoritesOnly ? 'soft' : 'ghost'"
          title="Seulement les cadeaux recherchés"
          @click="favoritesOnly = !favoritesOnly"
        />
        <UButton label="Toutes les catégories" size="sm" :color="categoryFilter === null ? 'primary' : 'neutral'" :variant="categoryFilter === null ? 'soft' : 'ghost'" @click="selectCategory(null)" />
        <UButton
          v-for="c in categories"
          :key="c.id"
          size="sm"
          :color="categoryFilter === c.id ? 'primary' : 'neutral'"
          :variant="categoryFilter === c.id ? 'soft' : 'ghost'"
          @click="selectCategory(c.id)"
        >
          <img :src="boxArt(null, c.id)" alt="" class="w-4 h-5 rounded-sm object-cover">
          {{ c.name }} <span class="text-muted">{{ c.count }}</span>
        </UButton>
        <UButton v-if="uncategorized" size="sm" :color="categoryFilter === '' ? 'primary' : 'neutral'" :variant="categoryFilter === '' ? 'soft' : 'ghost'" @click="selectCategory('')">
          Sans catégorie <span class="text-muted">{{ uncategorized }}</span>
        </UButton>
      </div>
    </div>

    <div v-if="filtered.length" class="flex flex-col gap-6">
      <section v-for="cs in sections" :key="cs.id ?? ''" class="flex flex-col gap-3">
        <h2 class="flex items-center gap-2 text-base font-semibold border-b border-default pb-1.5">
          <img v-if="cs.id" :src="boxArt(null, cs.id)" alt="" class="w-6 h-8 rounded object-cover">
          <UIcon v-else name="i-lucide-package" class="size-5 text-muted" />
          {{ cs.name }} <span class="text-sm font-normal text-muted">{{ cs.count }}</span>
        </h2>
        <div v-for="gs in cs.groups" :key="gs.id ?? 'none'" class="flex flex-col gap-2 pl-2">
          <h3 class="flex items-center gap-1.5 text-sm font-medium" :class="gs.id === null ? 'text-muted' : ''">
            <UIcon :name="gs.id === null ? 'i-lucide-folder-x' : 'i-lucide-folder'" class="size-4 text-muted" />
            {{ gs.name }} <span class="text-xs font-normal text-muted">{{ gs.gifts.length }}</span>
            <UBadge v-if="gs.loose" label="tous les jeux" size="sm" color="neutral" variant="subtle" />
          </h3>
          <div class="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-3">
            <div v-for="g in gs.gifts" :key="g.id" class="border border-default rounded-lg overflow-hidden flex flex-col group">
              <div class="relative aspect-square bg-elevated flex items-center justify-center">
                <img v-if="g.image" :src="g.image" referrerpolicy="no-referrer" alt="" class="w-full h-full object-contain">
                <UIcon v-else name="i-lucide-gift" class="size-10 text-muted" />
                <button
                  type="button"
                  class="absolute top-1.5 right-1.5 rounded-full bg-default/80 p-1.5 transition-colors"
                  :class="g.favorite ? 'text-warning' : 'text-muted hover:text-warning'"
                  :aria-pressed="g.favorite"
                  :aria-label="g.favorite ? `Retirer ${g.title} des favoris` : `Mettre ${g.title} en favori`"
                  :title="g.favorite ? 'Cadeau recherché (favori)' : 'Mettre en favori : cadeau recherché'"
                  @click="toggleFavorite(g)"
                >
                  <SolidIcon v-if="g.favorite" name="star" class="size-4 block" />
                  <UIcon v-else name="i-lucide-star" class="size-4 block" />
                </button>
              </div>
              <div class="p-3 flex flex-col gap-1.5 flex-1">
                <div class="font-medium leading-tight">{{ g.title }}</div>
                <div class="text-xs text-muted">{{ usage(g.id) }} giveaway(s)</div>
                <div v-if="g.keywords.length" class="flex flex-wrap gap-1">
                  <UBadge v-for="k in g.keywords" :key="k" :label="k" size="sm" color="neutral" variant="subtle" />
                </div>
                <div class="flex justify-end gap-1 mt-auto pt-1">
                  <UButton icon="i-lucide-pencil" size="xs" color="neutral" variant="ghost" aria-label="Modifier" @click="openForm(g)" />
                  <UButton icon="i-lucide-trash-2" size="xs" color="error" variant="ghost" aria-label="Supprimer" @click="toRemove = g" />
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>
    </div>
    <p v-else class="text-sm text-muted">
      {{ gifts.length ? 'Aucun cadeau ne correspond.' : 'Catalogue vide. Ajoute ton premier cadeau : il pourra ensuite être rattaché aux giveaways.' }}
    </p>

    <GiftFormModal v-model:open="formOpen" :gift="editing" />
    <GiftGroupsModal v-model:open="groupsOpen" :game-id="categoryFilter || null" :game-name="categories.find(c => c.id === categoryFilter)?.name ?? null" />

    <UModal :open="toRemove !== null" title="Supprimer le cadeau ?" @update:open="v => { if (!v) toRemove = null }">
      <template #body>
        <p>{{ toRemove?.title }} sera retiré du catalogue et des giveaways auxquels il est rattaché.</p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="toRemove = null" />
          <UButton label="Supprimer" color="error" :loading="busy('remove')" @click="confirmRemove" />
        </div>
      </template>
    </UModal>
  </div>
</template>
