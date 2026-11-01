<script setup lang="ts">
import { computed } from 'vue'
import { useSnapshot } from '../composables/useSnapshot'
import { ACCOUNT_STATUS, EVENT_KIND, formatNumber, formatTime } from '../utils/format'

const { accounts, events, channels } = useSnapshot()

const earnedToday = computed(() => channels.value.reduce((sum, c) => sum + c.channel.earnedToday, 0))
const seEarnedToday = computed(() => channels.value.reduce((sum, c) => sum + c.channel.seEarnedToday, 0))
const watching = computed(() => channels.value.filter(c => c.channel.state === 'watching'))
const live = computed(() => channels.value.filter(c => c.channel.live).length)
const attention = computed(() => accounts.value.filter(a => a.status === 'expired' || a.status === 'error').length)
const bonusesToday = computed(() => {
  const today = new Date().toDateString()
  return events.value.filter(e => e.kind === 'bonus' && new Date(e.at).toDateString() === today).length
})
</script>

<template>
  <div class="p-6 flex flex-col gap-6">
    <h1 class="text-xl font-semibold">Tableau de bord</h1>

    <div class="grid grid-cols-4 gap-3">
      <UCard>
        <div class="text-2xl font-semibold">+{{ formatNumber(earnedToday) }}</div>
        <div class="text-sm text-muted">points de chaîne gagnés aujourd'hui · {{ bonusesToday }} bonus</div>
      </UCard>
      <UCard>
        <div class="text-2xl font-semibold">+{{ formatNumber(seEarnedToday) }}</div>
        <div class="text-sm text-muted">points StreamElements gagnés aujourd'hui</div>
      </UCard>
      <UCard>
        <div class="text-2xl font-semibold">{{ watching.length }} / {{ live }}</div>
        <div class="text-sm text-muted">chaînes regardées / en live</div>
      </UCard>
      <UCard>
        <div class="text-2xl font-semibold" :class="attention ? 'text-error' : ''">{{ attention }}</div>
        <div class="text-sm text-muted">compte(s) à vérifier</div>
      </UCard>
    </div>

    <UAlert
      v-if="accounts.length === 0"
      icon="i-lucide-user-plus"
      title="Aucun compte connecté"
      description="Connecte un compte Twitch, puis ajoute les chaînes sur lesquelles récolter des points."
      :actions="[{ label: 'Connecter un compte', to: '/accounts' }]"
      color="primary"
      variant="subtle"
    />

    <div class="grid grid-cols-[1.3fr_1fr] gap-4 items-start">
      <UCard :ui="{ body: 'p-0 sm:p-0' }">
        <template #header>
          <span class="font-medium">Activité par compte</span>
        </template>
        <div v-for="account in accounts" :key="account.id" class="px-4 py-3 border-b border-default last:border-0 flex items-center gap-3">
          <UAvatar :text="account.login.slice(0, 2).toUpperCase()" size="sm" />
          <div class="flex-1 min-w-0">
            <div class="font-medium">{{ account.login }}</div>
            <div class="text-sm text-muted truncate">
              <template v-if="account.statusText">{{ account.statusText }}</template>
              <template v-else-if="account.channels.some(c => c.state === 'watching')">
                Points sur {{ account.channels.filter(c => c.state === 'watching').map(c => c.login).join(' et ') }}
              </template>
              <template v-else>Aucune chaîne suivie en live</template>
            </div>
          </div>
          <UBadge :color="ACCOUNT_STATUS[account.status].color" variant="subtle" :label="ACCOUNT_STATUS[account.status].label" />
        </div>
        <div v-if="accounts.length === 0" class="px-4 py-6 text-sm text-muted">Rien pour l'instant.</div>
      </UCard>

      <UCard :ui="{ body: 'p-0 sm:p-0' }">
        <template #header>
          <span class="font-medium">Fil d'événements</span>
        </template>
        <div class="max-h-[420px] overflow-y-auto">
          <div v-for="(event, i) in events.slice(0, 80)" :key="i" class="px-4 py-2 border-b border-default last:border-0 flex items-start gap-2.5 text-sm">
            <span class="text-xs text-muted w-10 shrink-0 pt-0.5">{{ formatTime(event.at) }}</span>
            <UIcon :name="EVENT_KIND[event.kind].icon" :class="EVENT_KIND[event.kind].color" class="size-4 shrink-0 mt-0.5" />
            <span class="min-w-0"><b v-if="accounts.length > 1">{{ event.account }} · </b>{{ event.message }}</span>
          </div>
          <div v-if="events.length === 0" class="px-4 py-6 text-sm text-muted">Les bonus, gains et passages en live apparaîtront ici.</div>
        </div>
      </UCard>
    </div>
  </div>
</template>
