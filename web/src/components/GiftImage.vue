<script setup lang="ts">
import { ref, watch } from 'vue'
import { giftImage } from '../api'

// Image d'un lot : adresse du serveur (lue avec le jeton de l'appareil, gardée en cache) ou
// adresse en ligne. Icône de cadeau quand le lot n'a pas d'image ou qu'elle ne se charge pas
const props = defineProps<{ src: string | null }>()
const shown = ref<string | null>(null)
const failed = ref(false)

watch(() => props.src, async (src) => {
  failed.value = false
  shown.value = null
  if (!src) {
    failed.value = true
    return
  }
  if (!src.startsWith('/api/')) {
    shown.value = src
    return
  }
  const url = await giftImage(src)
  if (props.src !== src) return
  shown.value = url
  failed.value = !url
}, { immediate: true })
</script>

<template>
  <img v-if="shown && !failed" :src="shown" alt="" class="size-12 rounded-md object-cover bg-muted shrink-0" loading="lazy" @error="failed = true">
  <div v-else class="size-12 rounded-md bg-muted shrink-0 flex items-center justify-center">
    <UIcon v-if="failed" name="i-lucide-gift" class="size-5 text-muted" />
  </div>
</template>
