<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { InstanceState, PhoneAction, PointsChannel, PointsChannelState } from '../types'
import { ApiError, pointsOrder } from '../api'
import { sendAction, usePhone } from '../phone'
import { formatNumber } from '../format'
import AccountPicker from './AccountPicker.vue'
import ChannelFlag from './ChannelFlag.vue'

// Page Points de chaîne : chaînes du compte choisi ; leur ordre (celui des chaînes regardées)
// se change en faisant glisser la poignée, puis est enregistré par le PC
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

// --- Réglages d'une chaîne (récolte, bonus auto, solde maximum), appliqués par le PC ---
const optionPending = (login: string, option: string) =>
  phone.actions.value.some(a => a.status === 'pending' && a.kind === 'points_option' && a.payload.channel === login && (a.payload as Record<string, unknown>).option === option)
function setOption(login: string, option: 'harvest' | 'bonus', enabled: boolean) {
  if (!current.value) return
  void sendAction(phone, toast, { kind: 'points_option', account: current.value.login, channel: login, option, enabled })
}
const maxOpen = ref(false)
const maxChannel = ref('')
const maxValue = ref<number | null>(null)
function editMax(c: PointsChannel) {
  maxChannel.value = c.login
  maxValue.value = c.maxBalance
  maxOpen.value = true
}
function saveMax() {
  if (!current.value) return
  void sendAction(phone, toast, { kind: 'points_max', account: current.value.login, channel: maxChannel.value, max: maxValue.value ?? 0 })
  maxOpen.value = false
}

// --- Ordre : glisser la poignée (souris ou doigt) ---
const phone = usePhone()
const toast = useToast()
// Ordre affiché pendant le glisser, puis jusqu'à ce que le PC publie le nouveau (null : celui du PC)
const order = ref<string[] | null>(null)
const dragging = ref<string | null>(null)
const list = ref<HTMLElement | null>(null)
const channels = computed<PointsChannel[]>(() => {
  const all = current.value?.channels ?? []
  if (!order.value) return all
  const byLogin = new Map(all.map(c => [c.login, c]))
  const kept = order.value.map(l => byLogin.get(l)).filter((c): c is PointsChannel => !!c)
  return [...kept, ...all.filter(c => !order.value!.includes(c.login))]
})
// Ordre publié par le PC égal à celui demandé (ou compte changé) : on revient au sien
watch(() => [current.value?.login, current.value?.channels.map(c => c.login).join()], ([, published]) => {
  if (!dragging.value && order.value && (!published || order.value.join() === published)) order.value = null
})
watch(() => current.value?.login, () => { order.value = null })
let timer: ReturnType<typeof setTimeout> | null = null

function onGrab(e: PointerEvent, login: string) {
  e.preventDefault()
  dragging.value = login
  order.value = channels.value.map(c => c.login)
  ;(e.target as HTMLElement).setPointerCapture(e.pointerId)
}
function onMove(e: PointerEvent) {
  if (!dragging.value || !order.value || !list.value) return
  // Ligne sous le doigt : la chaîne tirée prend sa place
  const rows = [...list.value.querySelectorAll<HTMLElement>('[data-login]')]
  const over = rows.find(r => {
    const b = r.getBoundingClientRect()
    return e.clientY >= b.top && e.clientY <= b.bottom
  })
  const target = over?.dataset.login
  if (!target || target === dragging.value) return
  const from = order.value.indexOf(dragging.value)
  const to = order.value.indexOf(target)
  const next = [...order.value]
  next.splice(from, 1)
  next.splice(to, 0, dragging.value)
  order.value = next
}
async function onDrop() {
  if (!dragging.value) return
  dragging.value = null
  const acc = current.value
  const token = phone.token.value
  if (!acc || !token || !order.value) return
  if (order.value.join() === acc.channels.map(c => c.login).join()) {
    order.value = null
    return
  }
  try {
    phone.requested(await pointsOrder<PhoneAction>(token, acc.login, order.value))
    // Sans nouvelle publication du PC : retour à son ordre
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => { order.value = null }, 20_000)
  } catch (e) {
    order.value = null
    toast.add({ title: 'Ordre non envoyé', description: e instanceof ApiError && e.status === 400 ? 'Mets à jour le serveur de synchronisation (version 0.8.10 ou plus).' : String(e), color: 'error' })
  }
}
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
      <p v-if="current.channels.length > 1" class="text-xs text-muted -mt-1">Fais glisser la poignée pour changer l'ordre : les premières chaînes en live sont regardées.</p>
      <ul ref="list" class="flex flex-col gap-2">
        <li
          v-for="c in channels"
          :key="c.login"
          :data-login="c.login"
          class="rounded-xl border bg-elevated/40 p-3 flex flex-col gap-2 transition-colors"
          :class="dragging === c.login ? 'border-primary bg-primary/10' : 'border-default'"
        >
          <div class="flex items-center gap-3">
          <button
            type="button"
            class="touch-none cursor-grab active:cursor-grabbing text-muted -ml-1 p-1 shrink-0"
            :aria-label="`Déplacer ${c.login}`"
            @pointerdown="onGrab($event, c.login)"
            @pointermove="onMove"
            @pointerup="onDrop"
            @pointercancel="onDrop"
          >
            <UIcon name="i-lucide-grip-vertical" class="size-5 block" />
          </button>
          <img v-if="c.avatar" :src="c.avatar" alt="" class="size-10 rounded-full shrink-0" loading="lazy">
          <div v-else class="size-10 rounded-full bg-accented shrink-0" />
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-1.5 flex-wrap">
              <ChannelFlag :login="c.login" :langs="state?.languages" />
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
          </div>
          <!-- Réglages de la ligne, appliqués par le PC -->
          <div v-if="c.harvest !== undefined" class="flex flex-wrap items-center gap-x-5 gap-y-2 pl-9">
            <USwitch :model-value="c.harvest" label="Récolte" size="sm" :disabled="optionPending(c.login, 'harvest')" @update:model-value="(v: boolean) => setOption(c.login, 'harvest', v)" />
            <USwitch :model-value="!!c.autoClaim" label="Bonus auto" size="sm" :disabled="optionPending(c.login, 'bonus')" @update:model-value="(v: boolean) => setOption(c.login, 'bonus', v)" />
            <UButton :label="c.maxBalance ? `Max ${formatNumber(c.maxBalance)}` : 'Solde max'" icon="i-lucide-target" size="xs" color="neutral" variant="soft" @click="editMax(c)" />
          </div>
        </li>
      </ul>
    </template>
    <p v-else class="text-sm text-muted py-6 text-center">Aucun compte sur le PC.</p>
    <UModal v-model:open="maxOpen" :title="`Solde maximum · ${maxChannel}`" description="Une fois ce solde atteint, la chaîne n'est plus regardée et laisse sa place. Vide : pas de limite.">
      <template #body>
        <UInputNumber v-model="maxValue" :min="1" :max="2000000000" placeholder="Pas de limite" class="w-full" />
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="maxOpen = false" />
          <UButton label="Enregistrer" @click="saveMax" />
        </div>
      </template>
    </UModal>
  </div>
</template>
