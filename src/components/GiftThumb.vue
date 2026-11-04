<script setup lang="ts">
import { ref, watch } from 'vue'

// Image d'un cadeau, ou icône de cadeau quand il n'en a pas ou que l'adresse ne répond plus.
// La taille vient de la classe donnée par le parent ; `icon` règle celle de l'icône
const props = withDefaults(defineProps<{ src?: string | null, icon?: string, lazy?: boolean }>(), { src: null, icon: 'size-4', lazy: false })
const broken = ref(false)
watch(() => props.src, () => { broken.value = false })
</script>

<template>
  <img v-if="src && !broken" :src="src" referrerpolicy="no-referrer" alt="" class="rounded object-contain bg-elevated" :loading="lazy ? 'lazy' : undefined" @error="broken = true">
  <span v-else class="rounded bg-elevated flex items-center justify-center shrink-0"><UIcon name="i-lucide-gift" class="text-muted" :class="icon" /></span>
</template>
