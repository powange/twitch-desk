<script setup lang="ts">
import { ref } from 'vue'

// Commandes du chat que la détection de giveaway ignore sur la chaîne d'un streamer
const commands = defineModel<string[]>({ required: true })
defineProps<{ id: string }>()

const input = ref('')
function add() {
  const list = [...commands.value]
  for (const raw of input.value.split(/[\s,]+/)) {
    const c = raw.trim().toLowerCase()
    if (c && !list.includes(c)) list.push(c)
  }
  commands.value = list
  input.value = ''
}
function remove(c: string) {
  commands.value = commands.value.filter(x => x !== c)
}
</script>

<template>
  <div class="flex items-center gap-1.5 flex-wrap">
    <UBadge v-for="c in commands" :key="c" color="neutral" variant="subtle" class="font-mono">
      {{ c }}
      <button type="button" class="ml-1 hover:text-error" :aria-label="`Ne plus ignorer ${c}`" @click="remove(c)">×</button>
    </UBadge>
    <UInput
      :id="id"
      v-model="input"
      placeholder="!commande"
      size="sm"
      class="w-36 font-mono"
      @keydown.enter.prevent="add"
      @blur="add"
    />
  </div>
</template>
