<script setup lang="ts">
import { computed } from 'vue'
import { CHANGELOG, type ChangeKind } from '../changelog'

// Version installée : mise en avant dans la liste
const props = defineProps<{ current: string | null }>()

const KINDS: Record<ChangeKind, { label: string, color: 'success' | 'info' | 'warning', icon: string }> = {
  new: { label: 'Nouveauté', color: 'success', icon: 'i-lucide-sparkles' },
  improved: { label: 'Amélioration', color: 'info', icon: 'i-lucide-wand-sparkles' },
  fixed: { label: 'Correction', color: 'warning', icon: 'i-lucide-wrench' }
}
// Dans chaque version : nouveautés, puis améliorations, puis corrections
const ORDER: ChangeKind[] = ['new', 'improved', 'fixed']
const releases = computed(() => CHANGELOG.map(r => ({
  ...r,
  changes: [...r.changes].sort((a, b) => ORDER.indexOf(a.kind) - ORDER.indexOf(b.kind))
})))
const formatDate = (d: string) => new Date(`${d}T12:00:00`).toLocaleDateString('fr-FR', { day: 'numeric', month: 'long', year: 'numeric' })
</script>

<template>
  <div class="flex flex-col gap-4">
    <article v-for="r in releases" :key="r.version" class="border border-default rounded-lg px-4 py-3" :class="r.version === props.current ? 'border-primary/60 bg-primary/5' : ''">
      <header class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h3 class="font-semibold">Version {{ r.version }}</h3>
        <span class="text-sm text-muted">{{ formatDate(r.date) }}</span>
        <UBadge v-if="r.version === props.current" label="Installée" color="primary" variant="subtle" size="sm" />
      </header>
      <p v-if="r.title" class="text-sm font-medium mt-1">{{ r.title }}</p>
      <ul class="mt-2 flex flex-col gap-1.5">
        <li v-for="(c, i) in r.changes" :key="i" class="flex items-start gap-2 text-sm">
          <UBadge :label="KINDS[c.kind].label" :color="KINDS[c.kind].color" :icon="KINDS[c.kind].icon" variant="subtle" size="sm" class="shrink-0 w-32 justify-center mt-px" />
          <span>{{ c.text }}</span>
        </li>
      </ul>
    </article>
  </div>
</template>
