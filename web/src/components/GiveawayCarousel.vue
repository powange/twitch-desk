<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { Giveaway, InstanceState } from '../types'
import GiveawayCard from './GiveawayCard.vue'

// Giveaways en diapositives : un par écran, glisser à gauche ou à droite pour passer au
// suivant ou au précédent (défilement aimanté), compteur et flèches. `index` : giveaway affiché
// (compteur « 1 / 2 » du titre de la section)
const props = defineProps<{ giveaways: Giveaway[], state: InstanceState | null }>()
const track = ref<HTMLElement | null>(null)
const index = defineModel<number>('index', { default: 0 })

function onScroll() {
  const el = track.value
  if (!el || !el.clientWidth) return
  index.value = Math.min(props.giveaways.length - 1, Math.max(0, Math.round(el.scrollLeft / el.clientWidth)))
}
function go(i: number) {
  const el = track.value
  if (!el) return
  const target = Math.min(props.giveaways.length - 1, Math.max(0, i))
  const smooth = !window.matchMedia('(prefers-reduced-motion: reduce)').matches
  el.scrollTo({ left: target * el.clientWidth, behavior: smooth ? 'smooth' : 'auto' })
}
// Liste raccourcie (giveaway clos) : rester dans les bornes
watch(() => props.giveaways.length, (n) => { if (index.value > n - 1) go(n - 1) })

function onKey(e: KeyboardEvent) {
  if (e.key === 'ArrowRight') { e.preventDefault(); go(index.value + 1) }
  if (e.key === 'ArrowLeft') { e.preventDefault(); go(index.value - 1) }
}
onMounted(() => track.value?.addEventListener('scroll', onScroll, { passive: true }))
onBeforeUnmount(() => track.value?.removeEventListener('scroll', onScroll))
</script>

<template>
  <div class="flex flex-col gap-2">
    <div
      ref="track"
      class="flex overflow-x-auto snap-x snap-mandatory -mx-4 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      tabindex="0"
      role="region"
      aria-roledescription="carrousel"
      :aria-label="`Giveaways, ${index + 1} sur ${giveaways.length}`"
      @keydown="onKey"
    >
      <div v-for="(g, i) in giveaways" :key="g.id" class="w-full shrink-0 snap-center px-4" :aria-hidden="i !== index">
        <GiveawayCard :giveaway="g" :state="state" />
      </div>
    </div>
    <div v-if="giveaways.length > 1" class="flex items-center justify-center gap-3">
      <UButton icon="i-lucide-chevron-left" size="sm" color="neutral" variant="ghost" aria-label="Giveaway précédent" :disabled="index === 0" @click="go(index - 1)" />
      <div v-if="giveaways.length <= 12" class="flex items-center gap-1.5" aria-hidden="true">
        <span
          v-for="(g, i) in giveaways"
          :key="g.id"
          class="size-1.5 rounded-full transition-colors"
          :class="i === index ? 'bg-primary' : 'bg-accented'"
        />
      </div>
      <span class="text-xs text-muted tabular-nums">{{ index + 1 }} / {{ giveaways.length }}</span>
      <UButton icon="i-lucide-chevron-right" size="sm" color="neutral" variant="ghost" aria-label="Giveaway suivant" :disabled="index === giveaways.length - 1" @click="go(index + 1)" />
    </div>
  </div>
</template>
