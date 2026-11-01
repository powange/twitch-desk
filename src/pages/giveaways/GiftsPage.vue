<script setup lang="ts">
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useGiftCatalog } from '../../composables/useGiftCatalog'
import GiftFormModal from '../../components/GiftFormModal.vue'
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog'
import { boxArt } from '../../utils/format'
import type { Gift, GiftGroup } from '../../types'

const { gifts, groups, groupName, usage, refresh } = useGiftCatalog()
const toast = useToast()

const search = ref('')
// Catégorie : null = toutes, '' = sans catégorie ; groupe (dans la catégorie) : null = tous
const categoryFilter = ref<string | null>(null)
const groupFilter = ref<number | null>(null)

// Catégories présentes dans le catalogue
const categories = computed(() => {
  const map = new Map<string, string>()
  for (const g of gifts.value) if (g.gameId) map.set(g.gameId, g.gameName ?? g.gameId)
  return [...map].map(([id, name]) => ({ id, name, count: gifts.value.filter(g => g.gameId === id).length }))
    .sort((a, b) => a.name.localeCompare(b.name))
})
const uncategorized = computed(() => gifts.value.filter(g => !g.gameId).length)
// Groupes de la catégorie choisie
const categoryGroups = computed(() =>
  categoryFilter.value === null ? [] : groups.value.filter(g => (g.gameId ?? '') === categoryFilter.value)
)

function selectCategory(id: string | null) {
  categoryFilter.value = id
  groupFilter.value = null
}

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  return gifts.value.filter((g) => {
    if (categoryFilter.value !== null && (g.gameId ?? '') !== categoryFilter.value) return false
    if (groupFilter.value !== null && g.groupId !== groupFilter.value) return false
    return !q || g.title.toLowerCase().includes(q) || g.keywords.some(k => k.includes(q))
  })
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

const toRemove = ref<Gift | null>(null)
async function confirmRemove() {
  if (!toRemove.value) return
  await invoke('delete_gift', { id: toRemove.value.id })
  toRemove.value = null
  await refresh()
}

async function removeGroup(g: GiftGroup) {
  try {
    await invoke('delete_gift_group', { id: g.id })
    if (groupFilter.value === g.id) groupFilter.value = null
    await refresh()
  } catch (e) {
    toast.add({ title: 'Groupe non supprimé', description: String(e), color: 'error' })
  }
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <div class="flex items-center gap-3">
      <h1 class="text-xl font-semibold flex-1">Cadeaux</h1>
      <UButton label="Importer" icon="i-lucide-upload" color="neutral" variant="soft" @click="importGifts" />
      <UButton label="Exporter" icon="i-lucide-download" color="neutral" variant="soft" :disabled="gifts.length === 0" @click="exportGifts" />
      <UButton label="Nouveau cadeau" icon="i-lucide-plus" @click="openForm(null)" />
    </div>

    <div class="flex flex-col gap-2">
      <div class="flex items-center gap-2 flex-wrap">
        <UInput id="gift-search" v-model="search" icon="i-lucide-search" placeholder="Rechercher un cadeau ou un mot-clé" class="w-72" />
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
      <div v-if="categoryGroups.length" class="flex items-center gap-2 flex-wrap pl-1">
        <span class="text-xs uppercase text-muted tracking-wide">Groupes</span>
        <UButton label="Tous" size="xs" :color="groupFilter === null ? 'primary' : 'neutral'" :variant="groupFilter === null ? 'soft' : 'ghost'" @click="groupFilter = null" />
        <UFieldGroup v-for="g in categoryGroups" :key="g.id">
          <UButton :label="g.name" size="xs" :color="groupFilter === g.id ? 'primary' : 'neutral'" :variant="groupFilter === g.id ? 'soft' : 'ghost'" @click="groupFilter = g.id" />
          <UButton icon="i-lucide-x" size="xs" color="neutral" variant="ghost" :aria-label="`Supprimer le groupe ${g.name}`" title="Supprimer le groupe (ses cadeaux restent, sans groupe)" @click="removeGroup(g)" />
        </UFieldGroup>
      </div>
    </div>

    <div v-if="filtered.length" class="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-3">
      <div v-for="g in filtered" :key="g.id" class="border border-default rounded-lg overflow-hidden flex flex-col group">
        <div class="aspect-square bg-elevated flex items-center justify-center">
          <img v-if="g.image" :src="g.image" referrerpolicy="no-referrer" alt="" class="w-full h-full object-contain">
          <UIcon v-else name="i-lucide-gift" class="size-10 text-muted" />
        </div>
        <div class="p-3 flex flex-col gap-1.5 flex-1">
          <div class="font-medium leading-tight">{{ g.title }}</div>
          <div v-if="g.gameName" class="flex items-center gap-1.5 text-xs">
            <img :src="boxArt(null, g.gameId ?? '')" alt="" class="w-3.5 h-4.5 rounded-sm object-cover">
            <span>{{ g.gameName }}</span>
          </div>
          <div class="text-xs text-muted">
            <span v-if="groupName(g.groupId)">{{ groupName(g.groupId) }} · </span>{{ usage(g.id) }} giveaway(s)
          </div>
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
    <p v-else class="text-sm text-muted">
      {{ gifts.length ? 'Aucun cadeau ne correspond.' : 'Catalogue vide. Ajoute ton premier cadeau : il pourra ensuite être rattaché aux giveaways.' }}
    </p>

    <GiftFormModal v-model:open="formOpen" :gift="editing" />

    <UModal :open="toRemove !== null" title="Supprimer le cadeau ?" @update:open="v => { if (!v) toRemove = null }">
      <template #body>
        <p>{{ toRemove?.title }} sera retiré du catalogue et des giveaways auxquels il est rattaché.</p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="toRemove = null" />
          <UButton label="Supprimer" color="error" @click="confirmRemove" />
        </div>
      </template>
    </UModal>
  </div>
</template>
