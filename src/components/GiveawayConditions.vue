<script setup lang="ts">
import { computed } from 'vue'
import { participationConditions } from '../utils/giveaways'
import type { GiveawayDetails, GiveawayKind } from '../types'

const props = defineProps<{ details: GiveawayDetails, kind: GiveawayKind }>()
const conditions = computed(() => participationConditions(props.details, props.kind))
</script>

<template>
  <div class="flex items-center gap-1.5">
    <UTooltip v-for="c in conditions" :key="c.key" :text="c.label">
      <span class="relative inline-flex items-center gap-0.5" :class="c.colorClass" role="img" :aria-label="c.label">
        <UIcon :name="c.icon" class="size-4" />
        <span v-if="c.suffix" class="text-xs font-medium">{{ c.suffix }}</span>
        <span v-if="c.strike" class="absolute inset-0 flex items-center justify-center pointer-events-none">
          <span class="block w-5 h-px bg-current rotate-45" />
        </span>
      </span>
    </UTooltip>
    <span v-if="conditions.length === 0" class="text-muted">—</span>
  </div>
</template>
