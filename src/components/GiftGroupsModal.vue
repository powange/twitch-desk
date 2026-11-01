<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useGiftCatalog, useKnownCategories } from '../composables/useGiftCatalog'
import { boxArt } from '../utils/format'
import type { GiftGroup } from '../types'

// Groupes de cadeaux : par catégorie Twitch, ou sans catégorie (communs à tous les jeux).
// gameId / gameName : catégorie proposée pour un nouveau groupe
const props = defineProps<{ gameId?: string | null, gameName?: string | null }>()
const open = defineModel<boolean>('open', { required: true })
const emit = defineEmits<{ created: [id: number] }>()

const { gifts, groups, refresh } = useGiftCatalog()
const knownCategories = useKnownCategories()
const toast = useToast()

// --- Nouveau groupe ---
const NONE = 'none'
const name = ref('')
const category = ref<string>(NONE)
watch(open, (isOpen) => {
  if (!isOpen) return
  name.value = ''
  category.value = props.gameId ?? NONE
})
const categoryItems = computed(() => {
  const list = [...knownCategories.value]
  if (props.gameId && !list.some(c => c.id === props.gameId)) list.push({ id: props.gameId, name: props.gameName ?? props.gameId })
  return [{ label: 'Sans catégorie (tous les jeux)', value: NONE }, ...list.map(c => ({ label: c.name, value: c.id }))]
})
const adding = ref(false)
async function add() {
  if (!name.value.trim()) return
  adding.value = true
  try {
    const gameId = category.value === NONE ? null : category.value
    const gameName = gameId ? categoryItems.value.find(c => c.value === gameId)?.label ?? null : null
    const id = await invoke<number>('save_gift_group', { id: 0, name: name.value, gameId, gameName })
    await refresh()
    name.value = ''
    emit('created', id)
  } catch (e) {
    toast.add({ title: 'Groupe non ajouté', description: String(e), color: 'error' })
  } finally {
    adding.value = false
  }
}

// --- Liste : par catégorie, groupes sans catégorie à la fin ---
const sections = computed(() => {
  const map = new Map<string, { id: string | null, name: string, groups: GiftGroup[] }>()
  for (const g of groups.value) {
    const key = g.gameId ?? ''
    if (!map.has(key)) map.set(key, { id: g.gameId, name: g.gameName ?? (g.gameId ? `Catégorie ${g.gameId}` : 'Sans catégorie'), groups: [] })
    map.get(key)!.groups.push(g)
  }
  return [...map.values()].sort((a, b) => (a.id === null ? 1 : 0) - (b.id === null ? 1 : 0) || a.name.localeCompare(b.name))
})
const giftCount = (id: number) => gifts.value.filter(g => g.groupId === id).length

// Renommage à la sortie du champ
const names = reactive<Record<number, string>>({})
async function rename(g: GiftGroup) {
  const value = names[g.id]
  if (value === undefined || value.trim() === g.name) return
  try {
    await invoke('save_gift_group', { id: g.id, name: value, gameId: null, gameName: null })
    await refresh()
  } catch (e) {
    toast.add({ title: 'Groupe non renommé', description: String(e), color: 'error' })
  }
  delete names[g.id]
}

async function remove(g: GiftGroup) {
  try {
    await invoke('delete_gift_group', { id: g.id })
    await refresh()
  } catch (e) {
    toast.add({ title: 'Groupe non supprimé', description: String(e), color: 'error' })
  }
}
</script>

<template>
  <UModal v-model:open="open" title="Groupes de cadeaux" description="Un groupe appartient à une catégorie Twitch, ou à aucune : il est alors proposé pour les cadeaux de tous les jeux." :ui="{ content: 'max-w-xl' }">
    <template #body>
      <div class="flex flex-col gap-4">
        <form class="flex gap-2" @submit.prevent="add">
          <UInput id="gift-group-name" v-model="name" placeholder="Nom du groupe (Skins d'armes…)" class="flex-1" autofocus />
          <USelect id="gift-group-category" v-model="category" :items="categoryItems" class="w-56" />
          <UButton type="submit" label="Ajouter" icon="i-lucide-plus" :loading="adding" :disabled="!name.trim()" />
        </form>

        <div v-for="s in sections" :key="s.id ?? ''" class="flex flex-col gap-1">
          <div class="flex items-center gap-2 text-sm font-medium">
            <img v-if="s.id" :src="boxArt(null, s.id)" alt="" class="w-4 h-5 rounded-sm object-cover">
            <UIcon v-else name="i-lucide-layers" class="size-4 text-muted" />
            {{ s.name }}
          </div>
          <ul class="border border-default rounded-lg divide-y divide-default">
            <li v-for="g in s.groups" :key="g.id" class="flex items-center gap-2 px-2 py-1">
              <UInput
                :id="`gift-group-${g.id}`"
                :model-value="names[g.id] ?? g.name"
                variant="ghost"
                size="sm"
                class="flex-1"
                :aria-label="`Nom du groupe ${g.name}`"
                @update:model-value="v => names[g.id] = String(v)"
                @blur="rename(g)"
                @keydown.enter.prevent="rename(g)"
              />
              <span class="text-xs text-muted whitespace-nowrap">{{ giftCount(g.id) }} cadeau(x)</span>
              <UButton icon="i-lucide-trash-2" size="xs" color="error" variant="ghost" :aria-label="`Supprimer le groupe ${g.name}`" title="Supprimer le groupe (ses cadeaux restent, sans groupe)" @click="remove(g)" />
            </li>
          </ul>
        </div>
        <p v-if="groups.length === 0" class="text-sm text-muted text-center py-2">Aucun groupe pour l'instant.</p>
      </div>
    </template>
  </UModal>
</template>
