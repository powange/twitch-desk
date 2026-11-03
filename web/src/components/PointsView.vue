<script setup lang="ts">
import { computed } from 'vue'
import type { InstanceState, PointsChannelState } from '../types'
import { formatNumber } from '../format'
import AccountPicker from './AccountPicker.vue'

// Page Points de chaîne : chaînes du compte choisi, en lecture seule
const props = defineProps<{ state: InstanceState | null }>()
const account = defineModel<string>('account', { required: true })

const accounts = computed(() => props.state?.points ?? [])
const current = computed(() => accounts.value.find(a => a.login === account.value) ?? accounts.value[0] ?? null)

const STATES: Record<PointsChannelState, { label: string, color: 'success' | 'primary' | 'neutral' | 'warning' }> = {
  watching: { label: 'Regardée', color: 'success' },
  waiting: { label: 'En attente', color: 'primary' },
  offline: { label: 'Hors ligne', color: 'neutral' },
  disabled: { label: 'Récolte coupée', color: 'neutral' },
  balanceReached: { label: 'Solde atteint', color: 'warning' },
  unknown: { label: '…', color: 'neutral' }
}
const totalToday = computed(() => current.value?.channels.reduce((n, c) => n + c.earnedToday, 0) ?? 0)
</script>

<template>
  <div class="flex flex-col gap-3">
    <p v-if="!state?.points" class="text-sm text-muted">Mets à jour l'application du PC (version 0.41 ou plus) pour voir ses points de chaîne ici.</p>
    <template v-else-if="current">
      <AccountPicker v-model="account" :accounts="accounts.map(a => a.login)" />
      <div class="flex items-baseline justify-between gap-2">
        <h1 class="text-sm font-medium text-muted uppercase truncate min-w-0">Points de chaîne · {{ current.login }}</h1>
        <span class="text-sm tabular-nums whitespace-nowrap shrink-0">+{{ formatNumber(totalToday) }} aujourd'hui</span>
      </div>
      <UAlert v-if="!current.enabled" color="warning" variant="subtle" icon="i-lucide-pause" description="Points de chaîne coupés pour ce compte sur le PC." />
      <p v-if="!current.channels.length" class="text-sm text-muted py-6 text-center">Aucune chaîne dans la page Points de chaîne de ce compte.</p>
      <ul class="flex flex-col gap-2">
        <li v-for="c in current.channels" :key="c.login" class="rounded-xl border border-default bg-elevated/40 p-3 flex items-center gap-3">
          <img v-if="c.avatar" :src="c.avatar" alt="" class="size-10 rounded-full shrink-0" loading="lazy">
          <div v-else class="size-10 rounded-full bg-accented shrink-0" />
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-1.5 flex-wrap">
              <span class="font-semibold break-all">{{ c.login }}</span>
              <UBadge :label="STATES[c.state]?.label ?? c.state" :color="STATES[c.state]?.color ?? 'neutral'" variant="subtle" size="sm" />
              <UBadge v-if="c.priority" label="Prioritaire" color="primary" variant="subtle" size="sm" icon="i-lucide-gift" />
            </div>
            <div v-if="c.live" class="text-xs text-muted truncate">
              {{ c.live.category ?? 'Sans catégorie' }} · <span class="text-error">{{ formatNumber(c.live.viewers) }} spectateurs</span>
            </div>
          </div>
          <div class="text-right shrink-0 tabular-nums">
            <div class="font-semibold">{{ formatNumber(c.balance) }}</div>
            <div class="text-xs text-success">+{{ formatNumber(c.earnedToday) }}</div>
            <div v-if="c.maxBalance" class="text-xs text-muted">max {{ formatNumber(c.maxBalance) }}</div>
          </div>
        </li>
      </ul>
    </template>
    <p v-else class="text-sm text-muted py-6 text-center">Aucun compte sur le PC.</p>
  </div>
</template>
