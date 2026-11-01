<script setup lang="ts">
import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { boxArt } from '../utils/format'

// Champ de catégorie Twitch avec autocomplétion (jaquette et nom), après une courte pause
// dans la frappe. Entrée sans choisir dans la liste : le texte tapé (nom exact ou adresse)
interface CategoryHit { id: string, name: string, slug: string, boxArt: string | null }

const text = defineModel<string>({ required: true })
defineProps<{ id: string, placeholder: string, loading?: boolean }>()
const emit = defineEmits<{ submit: [input: string] }>()

const hits = ref<CategoryHit[]>([])
const open = ref(false)
const highlighted = ref(-1)
let timer: ReturnType<typeof setTimeout> | null = null
let seq = 0
// Texte mis par un choix dans la liste : pas de nouvelle recherche
let picked: string | null = null

// Recherche en attente ou en cours abandonnée (sa réponse n'ouvrira pas la liste)
function invalidate() {
  if (timer) clearTimeout(timer)
  timer = null
  seq++
}

watch(text, (value) => {
  invalidate()
  const q = value.trim()
  const fromPick = picked !== null && value === picked
  picked = null
  // Adresse collée, champ vidé ou choix fait : pas de recherche
  if (fromPick || q.length < 2 || q.includes('twitch.tv')) {
    hits.value = []
    open.value = false
    return
  }
  timer = setTimeout(async () => {
    const mine = ++seq
    try {
      const found = await invoke<CategoryHit[]>('search_categories', { query: q })
      if (mine !== seq) return
      hits.value = found
      highlighted.value = -1
      open.value = found.length > 0
    } catch {
      hits.value = []
    }
  }, 300)
})

function pick(hit: CategoryHit) {
  invalidate()
  open.value = false
  hits.value = []
  picked = hit.name
  text.value = hit.name
  emit('submit', `https://www.twitch.tv/directory/category/${hit.slug}`)
}

function onKey(e: KeyboardEvent) {
  if (e.key === 'ArrowDown' && hits.value.length) {
    e.preventDefault()
    open.value = true
    highlighted.value = (highlighted.value + 1) % hits.value.length
  } else if (e.key === 'ArrowUp' && hits.value.length) {
    e.preventDefault()
    highlighted.value = (highlighted.value - 1 + hits.value.length) % hits.value.length
  } else if (e.key === 'Enter') {
    e.preventDefault()
    const hit = open.value ? hits.value[highlighted.value] : undefined
    if (hit) pick(hit)
    else if (text.value.trim()) {
      invalidate()
      open.value = false
      emit('submit', text.value)
    }
  } else if (e.key === 'Escape' && open.value) {
    e.stopPropagation()
    open.value = false
  }
}

function onBlur() {
  // Laisser le clic sur une ligne aboutir
  setTimeout(() => { open.value = false }, 150)
}
</script>

<template>
  <div class="relative flex-1">
    <UInput
      :id="id"
      v-model="text"
      :placeholder="placeholder"
      icon="i-lucide-search"
      :loading="loading"
      autocomplete="off"
      class="w-full"
      role="combobox"
      :aria-expanded="open"
      :aria-controls="`${id}-list`"
      @keydown="onKey"
      @focus="open = hits.length > 0"
      @blur="onBlur"
    />
    <ul
      v-if="open && hits.length"
      :id="`${id}-list`"
      role="listbox"
      class="absolute z-50 left-0 right-0 mt-1 max-h-80 overflow-y-auto rounded-md border border-default bg-default shadow-lg py-1"
    >
      <li
        v-for="(hit, i) in hits"
        :key="hit.id"
        role="option"
        :aria-selected="i === highlighted"
        class="flex items-center gap-3 px-2 py-1.5 cursor-pointer text-sm"
        :class="i === highlighted ? 'bg-elevated' : ''"
        @mouseenter="highlighted = i"
        @mousedown.prevent="pick(hit)"
      >
        <img :src="boxArt(hit.boxArt, hit.id)" alt="" class="w-8 h-11 rounded object-cover bg-elevated shrink-0">
        <span class="font-medium">{{ hit.name }}</span>
      </li>
    </ul>
  </div>
</template>
