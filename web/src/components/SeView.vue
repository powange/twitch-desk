<script setup lang="ts">
import { computed, ref } from 'vue'
import { ApiError, buyTickets } from '../api'
import type { InstanceState, PhoneAction, SeChannel } from '../types'
import { formatNumber } from '../format'
import AccountPicker from './AccountPicker.vue'

// Page StreamElements : chaînes du compte choisi, giveaway en cours, achat de tickets (fait par
// le PC s'il l'autorise)
const props = defineProps<{ state: InstanceState | null, token: string, pending: PhoneAction[] }>()
const account = defineModel<string>('account', { required: true })
const emit = defineEmits<{ requested: [PhoneAction] }>()
const toast = useToast()

const accounts = computed(() => props.state?.streamelements ?? [])
const current = computed(() => accounts.value.find(a => a.login === account.value) ?? accounts.value[0] ?? null)
const totalToday = computed(() => current.value?.channels.reduce((n, c) => n + c.earnedToday, 0) ?? 0)
// Giveaways en cours d'abord, puis chaînes en live
const channels = computed(() => [...(current.value?.channels ?? [])].sort((a, b) =>
  Number(!!b.giveaway) - Number(!!a.giveaway) || Number(!!b.live) - Number(!!a.live)))

// Achat : nombre de tickets possibles (maximum du giveaway, points disponibles)
const buying = ref<SeChannel | null>(null)
const tickets = ref(1)
const sending = ref(false)
function room(c: SeChannel) {
  const g = c.giveaway
  if (!g) return 0
  const byMax = g.maxTickets ? Math.max(0, g.maxTickets - (c.myTickets ?? 0)) : 100_000
  const byPoints = g.cost > 0 && c.points !== null ? Math.floor(c.points / g.cost) : 100_000
  return Math.min(byMax, byPoints)
}
const pendingFor = (c: SeChannel) => props.pending.some(p => p.status === 'pending' && p.payload.channel === c.login && p.payload.account === current.value?.login)
function askBuy(c: SeChannel) {
  buying.value = c
  // Un ticket par défaut : jamais une grosse dépense par erreur
  tickets.value = 1
}
async function confirmBuy() {
  const c = buying.value
  const acc = current.value
  if (!c?.giveaway || !acc) return
  sending.value = true
  try {
    const action = await buyTickets<PhoneAction>(props.token, { account: acc.login, channel: c.login, giveaway: c.giveaway.id, tickets: tickets.value })
    emit('requested', action)
    toast.add({ title: 'Demande envoyée au PC', description: `${tickets.value} ticket(s) sur ${c.login}`, color: 'neutral', icon: 'i-lucide-send' })
    buying.value = null
  } catch (e) {
    const message = e instanceof ApiError && e.status === 404 ? 'Mets à jour le serveur (version 0.8 ou plus) pour acheter depuis le téléphone.' : String(e instanceof Error ? e.message : e)
    toast.add({ title: 'Achat impossible', description: message, color: 'error' })
  } finally {
    sending.value = false
  }
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <p v-if="!state?.streamelements" class="text-sm text-muted">Mets à jour l'application du PC (version 0.41 ou plus) pour voir ses chaînes StreamElements ici.</p>
    <template v-else-if="current">
      <AccountPicker v-model="account" :accounts="accounts.map(a => a.login)" />
      <div class="flex items-baseline justify-between gap-2">
        <h1 class="text-sm font-medium text-muted uppercase truncate min-w-0">StreamElements · {{ current.login }}</h1>
        <span class="text-sm tabular-nums whitespace-nowrap shrink-0">+{{ formatNumber(totalToday) }} aujourd'hui</span>
      </div>
      <p v-if="!channels.length" class="text-sm text-muted py-6 text-center">Aucune chaîne dans la page StreamElements de ce compte.</p>
      <ul class="flex flex-col gap-2">
        <li v-for="c in channels" :key="c.login" class="rounded-xl border border-default bg-elevated/40 p-3 flex flex-col gap-2" :class="c.giveaway ? 'border-primary/50' : ''">
          <div class="flex items-center gap-3">
            <img v-if="c.avatar" :src="c.avatar" alt="" class="size-10 rounded-full shrink-0" loading="lazy">
            <div v-else class="size-10 rounded-full bg-accented shrink-0" />
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-1.5 flex-wrap">
                <span class="font-semibold break-all">{{ c.login }}</span>
                <UBadge v-if="c.live" label="En live" color="error" variant="solid" size="sm" />
                <UBadge v-if="c.inChat" label="Dans le chat" color="success" variant="subtle" size="sm" />
                <UBadge v-if="c.enabled === false" label="Sans StreamElements" color="neutral" variant="subtle" size="sm" />
              </div>
              <div v-if="c.live" class="text-xs text-muted truncate">{{ c.live.category ?? 'Sans catégorie' }} · <span class="text-error">{{ formatNumber(c.live.viewers) }} spectateurs</span></div>
            </div>
            <div class="text-right shrink-0 tabular-nums">
              <div class="font-semibold">{{ formatNumber(c.points) }}</div>
              <div class="text-xs text-success">+{{ formatNumber(c.earnedToday) }}</div>
            </div>
          </div>
          <!-- Giveaway StreamElements en cours -->
          <div v-if="c.giveaway" class="rounded-lg bg-primary/5 p-3 flex flex-col gap-1.5 text-sm">
            <div class="font-semibold whitespace-pre-line break-words">{{ c.giveaway.title || 'Giveaway StreamElements' }}</div>
            <p v-if="c.giveaway.description" class="text-muted whitespace-pre-line break-words">{{ c.giveaway.description }}</p>
            <div class="flex flex-wrap gap-x-4 gap-y-1 tabular-nums">
              <span class="inline-flex items-center gap-1"><UIcon name="i-lucide-ticket" class="size-3.5 text-muted" />{{ formatNumber(c.giveaway.cost) }} pts le ticket</span>
              <span v-if="c.giveaway.command" class="font-mono">{{ c.giveaway.command }}</span>
              <span>Mes tickets : <b>{{ formatNumber(c.myTickets) }}</b><template v-if="c.giveaway.maxTickets"> / {{ formatNumber(c.giveaway.maxTickets) }}</template></span>
              <span class="text-muted">{{ formatNumber(c.giveaway.totalTickets) }} tickets · {{ formatNumber(c.giveaway.totalUsers) }} participants</span>
            </div>
            <div class="flex items-center gap-2 pt-1">
              <UButton
                v-if="state.actionsAllowed && current.connected"
                label="Acheter des tickets"
                icon="i-lucide-ticket-plus"
                size="sm"
                :loading="pendingFor(c)"
                :disabled="room(c) < 1 || pendingFor(c)"
                @click="askBuy(c)"
              />
              <span v-if="pendingFor(c)" class="text-xs text-muted">Achat en cours sur le PC…</span>
              <span v-else-if="state.actionsAllowed && current.connected && room(c) < 1" class="text-xs text-muted">Maximum atteint ou points insuffisants</span>
              <span v-else-if="!current.connected" class="text-xs text-muted">Compte StreamElements non connecté sur le PC</span>
              <span v-else-if="!state.actionsAllowed" class="text-xs text-muted">Achats depuis le téléphone désactivés sur le PC</span>
            </div>
          </div>
        </li>
      </ul>
    </template>
    <p v-else class="text-sm text-muted py-6 text-center">Aucun compte sur le PC.</p>

    <UModal :open="buying !== null" title="Acheter des tickets" @update:open="v => { if (!v) buying = null }">
      <template #body>
        <div v-if="buying?.giveaway" class="flex flex-col gap-3 text-sm">
          <p>{{ buying.giveaway.title || 'Giveaway StreamElements' }} sur <b>{{ buying.login }}</b>, avec <b>{{ current?.login }}</b>.</p>
          <UFormField label="Nombre de tickets" :help="`Jusqu'à ${formatNumber(room(buying))} (points disponibles : ${formatNumber(buying.points)})`">
            <UInputNumber id="se-buy-tickets" v-model="tickets" :min="1" :max="Math.max(1, room(buying))" class="w-full" />
          </UFormField>
          <p class="tabular-nums">Coût : <b>{{ formatNumber(tickets * buying.giveaway.cost) }}</b> points StreamElements.</p>
          <p class="text-muted">Le PC fait l'achat : il doit être ouvert. Le résultat s'affiche ici.</p>
        </div>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="buying = null" />
          <UButton label="Acheter" icon="i-lucide-ticket-plus" :loading="sending" :disabled="!buying || room(buying) < 1" @click="confirmBuy" />
        </div>
      </template>
    </UModal>
  </div>
</template>
