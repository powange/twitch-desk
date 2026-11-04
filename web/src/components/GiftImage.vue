<script setup lang="ts">
import { ref, watch } from 'vue'
import { giftImage } from '../api'

// Image d'un lot : adresse du serveur (lue avec le jeton de l'appareil, gardée en cache) ou
// adresse en ligne
const props = defineProps<{ src: string }>()
const shown = ref<string | null>(null)

watch(() => props.src, async (src) => {
  if (!src.startsWith('/api/')) {
    shown.value = src
    return
  }
  shown.value = null
  const url = await giftImage(src)
  if (props.src === src) shown.value = url
}, { immediate: true })
</script>

<template>
  <img v-if="shown" :src="shown" alt="" class="size-12 rounded-md object-cover bg-muted shrink-0" loading="lazy">
  <div v-else class="size-12 rounded-md bg-muted shrink-0" />
</template>
