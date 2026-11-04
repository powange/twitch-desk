<script setup lang="ts">
import { computed, ref } from 'vue'

// Choix de streamers avec recherche : un bout de nom suffit (« mav » trouve lf_maverick), et ce
// bout peut lui-même servir de filtre (« Contient « mav » ») ; un ou plusieurs selon `multiple`
const props = defineProps<{ items: string[], multiple?: boolean, placeholder?: string, allLabel?: string }>()
const model = defineModel<string | string[]>({ required: true })

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
  const names = props.items.filter(match).map(n => ({ label: n, value: n }))
  const all = !props.multiple && !term.value ? [{ label: props.allLabel ?? 'Tous les streamers', value: '' }] : []
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
    v-model="model"
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
