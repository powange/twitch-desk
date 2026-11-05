<script setup lang="ts">
import { computed } from 'vue'
import type { DropCampaign, DropProgress, InstanceState } from '../types'
import AccountPicker from './AccountPicker.vue'

// Page Drops : chaîne regardée et campagnes en cours du compte choisi, en lecture seule
const props = defineProps<{ state: InstanceState | null }>()
const account = defineModel<string>('account', { required: true })

const accounts = computed(() => props.state?.drops ?? [])
const current = computed(() => accounts.value.find(a => a.login === account.value) ?? accounts.value[0] ?? null)

function percent(d: DropProgress) {
  if (d.claimed) return 100
  return d.requiredMinutes ? Math.min(100, Math.round(d.currentMinutes / d.requiredMinutes * 100)) : 0
}

function formatMinutes(m: number) {
  if (m < 60) return `${m} min`
  const h = Math.floor(m / 60)
  return m % 60 ? `${h} h ${m % 60}` : `${h} h`
}

// Même logique que la page Drops du PC
function dropState(c: DropCampaign, d: DropProgress): { label: string, color: 'success' | 'warning' | 'neutral' | 'info' | 'error' } {
  if (d.claimed) return { label: 'Obtenu', color: 'success' }
  if (d.claimable ?? (d.requiredMinutes > 0 && d.currentMinutes >= d.requiredMinutes)) {
    return c.accountLinked ? { label: 'À réclamer', color: 'info' } : { label: 'Compte du jeu à lier', color: 'error' }
  }
  if (d.currentMinutes > 0) return { label: 'En cours', color: 'warning' }
  return { label: 'À faire', color: 'neutral' }
}

const obtained = (c: DropCampaign) => c.drops.filter(d => d.claimed).length
const endDate = (iso: string) => new Date(iso).toLocaleDateString('fr-FR', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
</script>

<template>
  <div class="flex flex-col gap-3">
    <p v-if="!state?.drops" class="text-sm text-muted">Mets à jour l'application du PC (version 0.42 ou plus) pour voir ses drops ici.</p>
    <template v-else-if="current">
      <AccountPicker v-model="account" :accounts="accounts.map(a => a.login)" />
      <h1 class="text-sm font-medium text-muted uppercase truncate">Drops · {{ current.login }}</h1>
      <UAlert v-if="!current.enabled" color="warning" variant="subtle" icon="i-lucide-pause" description="Drops désactivés pour ce compte sur le PC." />
      <div v-if="current.target" class="rounded-xl border border-success/40 bg-success/10 p-3 flex items-start gap-2 text-sm">
        <UIcon name="i-lucide-eye" class="size-4 text-success shrink-0 mt-0.5" />
        <div class="min-w-0">
          <div><b>{{ current.target.game }}</b> : visionnage de <b class="break-all">{{ current.target.login }}</b></div>
          <div v-if="current.target.title" class="text-xs text-muted break-words">{{ current.target.title }}</div>
        </div>
      </div>
      <p v-if="!current.campaigns.length" class="text-sm text-muted py-6 text-center">Aucune campagne en cours pour ce compte.</p>
      <ul class="flex flex-col gap-3">
        <li v-for="c in current.campaigns" :key="c.id" class="rounded-xl border bg-elevated/40 overflow-hidden" :class="c.watched ? 'border-success/60' : 'border-default'">
          <div class="flex items-center gap-3 p-3 border-b border-default">
            <img :src="c.boxArt" alt="" class="w-10 h-14 rounded object-cover shrink-0 bg-accented" loading="lazy">
            <div class="min-w-0 flex-1">
              <div class="font-semibold break-words">{{ c.game }}</div>
              <div class="text-xs text-muted break-words">{{ c.name }}</div>
              <div class="text-xs text-muted">Jusqu'au {{ endDate(c.endsAt) }} · {{ obtained(c) }}/{{ c.drops.length }} obtenu(s)</div>
            </div>
            <div class="flex flex-col items-end gap-1 shrink-0">
              <UBadge v-if="c.watched" label="Regardée" color="success" variant="solid" size="sm" icon="i-lucide-eye" />
              <UBadge v-if="!c.accountLinked" label="Compte non lié" color="error" variant="subtle" size="sm" icon="i-lucide-unlink" />
            </div>
          </div>
          <ul class="flex flex-col">
            <li v-for="d in c.drops" :key="d.id" class="flex items-center gap-3 px-3 py-2.5 border-t border-default first:border-t-0">
              <img v-if="d.image" :src="d.image" alt="" class="size-9 rounded object-cover shrink-0" loading="lazy">
              <div v-else class="size-9 rounded bg-accented shrink-0" />
              <div class="min-w-0 flex-1 flex flex-col gap-1">
                <div class="flex items-center justify-between gap-2">
                  <span class="text-sm break-words min-w-0">{{ d.name }}</span>
                  <UBadge :label="dropState(c, d).label" :color="dropState(c, d).color" variant="subtle" size="sm" class="shrink-0 whitespace-nowrap" />
                </div>
                <UProgress :model-value="percent(d)" size="sm" :color="d.claimed ? 'success' : 'primary'" />
                <div class="text-xs text-muted tabular-nums">{{ formatMinutes(d.currentMinutes) }} / {{ formatMinutes(d.requiredMinutes) }} · {{ percent(d) }} %</div>
              </div>
            </li>
          </ul>
        </li>
      </ul>
    </template>
    <p v-else class="text-sm text-muted py-6 text-center">Aucun compte sur le PC.</p>
  </div>
</template>
