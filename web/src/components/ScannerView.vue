<script setup lang="ts">
import { computed } from 'vue'
import type { InstanceState } from '../types'
import { formatNumber, timeAgo } from '../format'
import { GIVEAWAY_KIND, streamUrl } from '../giveaways'

// Page Scanner : streams repérés par le scan automatique du PC, en lecture seule
const props = defineProps<{ state: InstanceState | null }>()
const scanner = computed(() => props.state?.scanner ?? null)
</script>

<template>
  <div class="flex flex-col gap-3">
    <p v-if="!scanner" class="text-sm text-muted">Mets à jour l'application du PC (version 0.44 ou plus) pour voir ici les streams repérés par le scanner.</p>
    <template v-else>
      <div class="flex items-baseline justify-between gap-2">
        <h1 class="text-sm font-medium text-muted uppercase">Streams repérés · {{ scanner.matches.length }}</h1>
        <span v-if="scanner.scannedAt" class="text-xs text-muted whitespace-nowrap">Scan {{ timeAgo(scanner.scannedAt) }}</span>
      </div>
      <div v-if="!scanner.matches.length" class="flex flex-col items-center gap-2 py-12 text-center">
        <UIcon name="i-lucide-radar" class="size-10 text-muted" />
        <p class="font-medium">Aucun stream repéré</p>
        <p class="text-sm text-muted">Ils apparaissent ici quand le scanner du PC en trouve un.</p>
      </div>
      <ul class="flex flex-col gap-2">
        <li v-for="m in scanner.matches" :key="m.login" class="rounded-xl border border-default bg-elevated/40 p-3 flex flex-col gap-2">
          <div class="flex items-center gap-3">
            <img v-if="m.avatar" :src="m.avatar" alt="" class="size-10 rounded-full shrink-0" loading="lazy">
            <div v-else class="size-10 rounded-full bg-accented shrink-0" />
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-1.5 flex-wrap">
                <span class="font-semibold break-all">{{ m.displayName || m.login }}</span>
                <UBadge v-if="m.monitored" label="Surveillée" color="success" variant="subtle" size="sm" />
                <UBadge v-if="m.giveawayKind" :label="GIVEAWAY_KIND[m.giveawayKind]?.label ?? m.giveawayKind" :color="GIVEAWAY_KIND[m.giveawayKind]?.color ?? 'neutral'" variant="subtle" size="sm" />
              </div>
              <div class="text-xs text-muted truncate">
                {{ m.game }} · <span class="text-error">{{ formatNumber(m.viewers) }} spectateurs</span><template v-if="m.startedAt"> · en live {{ timeAgo(m.startedAt).replace('il y a ', 'depuis ') }}</template>
              </div>
            </div>
            <UButton icon="i-lucide-external-link" size="sm" color="neutral" variant="soft" :to="streamUrl(m.login)" target="_blank" :aria-label="`Ouvrir ${m.login} sur Twitch`" />
          </div>
          <p class="text-sm break-words">{{ m.title }}</p>
          <div class="flex flex-wrap items-center gap-1.5">
            <UBadge v-for="k in m.keywords" :key="k" :label="k" color="primary" variant="outline" size="sm" />
            <span v-if="m.giveawayCommand" class="font-mono text-xs">{{ m.giveawayCommand }}</span>
          </div>
        </li>
      </ul>
    </template>
  </div>
</template>
