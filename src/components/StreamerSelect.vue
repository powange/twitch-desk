<script setup lang="ts">
import { computed, ref } from 'vue'

// Choix de streamers avec recherche : un bout de nom suffit (« mav » trouve lf_maverick), et ce
// bout peut lui-même servir de filtre (« Contient « mav » ») ; un ou plusieurs selon `multiple`
// `avatars` : photo de profil par streamer (minuscules), affichée à gauche du nom
const props = defineProps<{ items: string[], multiple?: boolean, placeholder?: string, allLabel?: string, avatars?: Record<string, string | null> }>()
const model = defineModel<string | string[]>({ required: true })

// « Tous » : valeur à part (une valeur vide est refusée par la liste, qui cesse alors de
// s'afficher)
const ALL = '__all__'
const value = computed<string | string[]>({
  get: () => (Array.isArray(model.value) ? model.value : model.value || ALL),
  set: (v) => { model.value = Array.isArray(v) ? v : v === ALL ? '' : v }
})
const search = ref('')
const term = computed(() => search.value.trim().toLowerCase())
const selected = computed(() => (Array.isArray(model.value) ? model.value : model.value ? [model.value] : []))
const options = computed(() => {
  const known = new Set(props.items.map(i => i.toLowerCase()))
  const match = (v: string) => !term.value || v.toLowerCase().includes(term.value)
  // Bouts de nom déjà choisis : gardés dans la liste pour rester visibles
  const keywords = selected.value
    .filter(v => v && !known.has(v.toLowerCase()) && match(v))
    .map(v => ({ label: `Contient « ${v} »`, value: v }))
  // Photo de profil (élément de liste de Nuxt UI), sinon une icône
  const names = props.items.filter(match).map((n) => {
    const src = props.avatars?.[n.toLowerCase()]
    return src ? { label: n, value: n, avatar: { src, alt: '' } } : { label: n, value: n, icon: 'i-lucide-circle-user' }
  })
  const all = !props.multiple && !term.value ? [{ label: props.allLabel ?? 'Tous les streamers', value: ALL }] : []
  return [...all, ...keywords, ...names]
})

// Bout de nom tapé, choisi comme filtre
function addKeyword(text: string) {
  const v = text.trim()
  if (!v) return
  if (props.multiple) {
    const list = Array.isArray(model.value) ? model.value : []
    if (!list.some(x => x.toLowerCase() === v.toLowerCase())) model.value = [...list, v]
  } else {
    model.value = v
  }
  search.value = ''
}
</script>

<template>
  <USelectMenu
    v-model="value"
    v-model:search-term="search"
    :items="options"
    value-key="value"
    :multiple="multiple"
    ignore-filter
    icon="i-lucide-tv"
    :placeholder="placeholder ?? 'Tous les streamers'"
    :search-input="{ placeholder: 'Nom ou bout de nom (ex. mav)' }"
    :create-item="{ position: 'top', when: 'always' }"
    @create="addKeyword"
  >
    <template #create-item-label="{ item }">Contient « {{ item }} »</template>
  </USelectMenu>
</template>
