<script setup lang="ts">
import { ref } from 'vue'

// Bouton « Actualiser » d'un tableau : lance la relecture, puis reste en chargement un court
// instant (les nouvelles données arrivent du cœur juste après)
const props = withDefaults(defineProps<{ action: () => unknown, label?: string }>(), { label: 'Actualiser' })
const toast = useToast()
const busy = ref(false)

async function run() {
  if (busy.value) return
  busy.value = true
  try {
    await Promise.all([props.action(), new Promise(r => setTimeout(r, 1200))])
  } catch (e) {
    toast.add({ title: 'Actualisation impossible', description: String(e), color: 'error' })
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <UButton :label="label" icon="i-lucide-refresh-cw" size="xs" color="neutral" variant="ghost" :loading="busy" @click="run" />
</template>
