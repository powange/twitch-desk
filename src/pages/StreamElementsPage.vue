<script setup lang="ts">
import RefreshButton from '../components/RefreshButton.vue'
import ChannelCell from '../components/ChannelCell.vue'
import { computed, reactive, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { useChannelList } from '../composables/useChannelList'
import { useSnapshot } from '../composables/useSnapshot'
import { useOpenExternal } from '../utils/external'
import { useWatchStream } from '../utils/viewer'
import { formatNumber } from '../utils/format'
import type { ChannelView, GiveawayView } from '../types'
import { FAVORITE_ROW, detectionText, giftHits, hasFavoriteGift, useGiftCatalog } from '../composables/useGiftCatalog'
import GiftBadge from '../components/GiftBadge.vue'
import AccountPicker from '../components/AccountPicker.vue'
import { useSelectedAccount } from '../composables/useSelectedAccount'
import { stringList, usePersisted } from '../composables/usePersisted'

const openExternal = useOpenExternal()
const watchStream = useWatchStream()
const { accounts, account, channels, newLogin, adding, addChannel, setOptions, optionsOf, toRemove, removing, confirmRemove } = useChannelList('se')
const { accountId } = useSelectedAccount()

// Filtre par streamer (choix multiple), retenu sur ce poste ; « ?channels=a,b » le préremplit
// pour cette visite (lien depuis les alertes) sans remplacer le filtre retenu
const route = useRoute()
const savedFilter = usePersisted('se.streamers', [] as string[], { sanitize: stringList })
const streamerFilter = ref<string[]>([...savedFilter.value])
// Filtre venu du lien, pas encore modifié à la main : pas retenu
let fromUrl: string | null = null
// « ?account=id » (badge d'un compte sur la page Alertes) : ce compte affiché
watch(() => route.query.account, (q) => {
  const id = Number(Array.isArray(q) ? q[0] : q)
  if (Number.isFinite(id) && id > 0 && accounts.value.some(a => a.id === id)) accountId.value = id
}, { immediate: true })
watch(() => route.query.channels, (q) => {
  const value = Array.isArray(q) ? q.join(',') : (q ?? '')
  if (value) {
    streamerFilter.value = value.toLowerCase().split(',').map(s => s.trim()).filter(Boolean)
    fromUrl = streamerFilter.value.join(',')
  }
}, { immediate: true })
watch(streamerFilter, (v) => {
  if (fromUrl !== null && v.join(',') === fromUrl) return
  fromUrl = null
  savedFilter.value = [...v]
}, { deep: true })
// Streamer retenu qui n'est plus dans la liste : retiré du filtre (une fois le compte chargé)
let pruned = false
watch(account, (a) => {
  if (!a || pruned) return
  pruned = true
  if (fromUrl !== null) return
  const logins = new Set(channels.value.map(c => c.login))
  const kept = streamerFilter.value.filter(l => logins.has(l))
  if (kept.length !== streamerFilter.value.length) streamerFilter.value = kept
}, { immediate: true })
const streamerItems = computed(() => channels.value.map(c => c.login).sort())
const shown = computed(() => streamerFilter.value.length ? channels.value.filter(c => streamerFilter.value.includes(c.login)) : channels.value)

// Giveaway StreamElements connu de l'application pour la chaîne : celui en cours (même lien),
// sinon le dernier détecté depuis 24 h — pour son état (clos) et son gagnant
const { snapshot } = useSnapshot()
const RECENT_MS = 24 * 3600 * 1000
const seGiveaways = computed(() => {
  const map = new Map<string, GiveawayView[]>()
  for (const g of snapshot.value?.giveaways.giveaways ?? []) {
    if (g.kind !== 'streamelements') continue
    if (!map.has(g.channel)) map.set(g.channel, [])
    map.get(g.channel)!.push(g)
  }
  return map
})
function findKnown(channel: ChannelView, now: number): GiveawayView | null {
  const list = seGiveaways.value.get(channel.login) ?? []
  const active = channel.seGiveaway
  if (active) return list.find(g => g.url?.includes(active.id)) ?? null
  let best: GiveawayView | null = null
  for (const g of list) {
    if (g.status === 'closed' && now - new Date(g.detectedAt).getTime() >= RECENT_MS) continue
    if (!best || g.detectedAt > best.detectedAt) best = g
  }
  return best
}
// Table chaîne → giveaway connu, calculée une fois par mise à jour (pas à chaque cellule)
const known = computed(() => {
  const now = Date.now()
  return new Map(channels.value.map(c => [c.login, findKnown(c, now)] as const))
})
function knownGiveaway(channel: ChannelView): GiveawayView | null {
  return known.value.get(channel.login) ?? null
}
// Cadeaux : ceux rattachés au giveaway connu, puis ceux repérés dans le titre et la description
// du giveaway StreamElements en cours (calculés une fois par mise à jour)
const { byId: giftById, gifts: catalogGifts } = useGiftCatalog()
const giftsByLogin = computed(() => new Map(channels.value.map((c) => {
  // Texte habituel de la chaîne (si elle est surveillée pour les giveaways), plus le giveaway
  // StreamElements relevé par les points (chaîne suivie seulement ici)
  const text = [detectionText(snapshot.value, c.login), c.seGiveaway?.title, c.seGiveaway?.description].filter(Boolean).join('\n')
  return [c.login, giftHits(giftById.value, catalogGifts.value, known.value.get(c.login)?.giftIds ?? [], text)] as const
})))
const channelGifts = (c: ChannelView) => giftsByLogin.value.get(c.login) ?? []
// État du giveaway : en cours (StreamElements le donne actif), en pause (live terminé), clos, ou rien de connu
function giveawayState(channel: ChannelView): { label: string, color: 'primary' | 'neutral' | 'success' | 'warning' } | null {
  if (channel.seGiveaway) return { label: 'En cours', color: 'primary' }
  const g = knownGiveaway(channel)
  if (!g) return null
  if (g.winner) return { label: 'Tiré', color: 'success' }
  if (g.status === 'paused') return { label: 'En pause', color: 'warning' }
  return g.open ? { label: 'Ouvert', color: 'primary' } : { label: 'Clos', color: 'neutral' }
}

// Pourquoi une chaîne ne rapporte pas (ou rien : elle rapporte)
function status(channel: ChannelView): { label: string, color: 'success' | 'warning' | 'neutral' | 'error' } {
  if (!channel.chatPresence) return { label: 'Chat désactivé', color: 'neutral' }
  if (channel.seEnabled === false) return { label: 'Pas de StreamElements', color: 'error' }
  if (!channel.inChat) return { label: 'Hors du chat', color: 'warning' }
  if (!channel.live) return { label: 'Hors ligne', color: 'neutral' }
  return { label: 'En live', color: 'success' }
}

// Plafond de l'achat automatique, enregistré après la saisie (pas à chaque chiffre)
const autoBuyMaxTimers = new Map<number, ReturnType<typeof setTimeout>>()
function setAutoBuyMax(channel: ChannelView, value: number | null) {
  const max = Math.max(0, Math.round(value ?? 0))
  clearTimeout(autoBuyMaxTimers.get(channel.id))
  autoBuyMaxTimers.set(channel.id, setTimeout(async () => {
    autoBuyMaxTimers.delete(channel.id)
    try {
      await invoke('set_se_auto_buy_max', { id: channel.id, max })
    } catch (e) {
      toast.add({ title: 'Plafond non enregistré', description: String(e), color: 'error' })
    }
  }, 600))
}
async function setAutoBuy(channel: ChannelView, enabled: boolean) {
  try {
    await invoke('set_se_auto_buy', { id: channel.id, enabled })
    if (enabled) toast.add({ title: `Achat automatique activé sur ${channel.login}`, description: 'Tous les tickets que le solde permet, en une fois, jusqu\'au maximum par personne ou au plafond choisi (0 : sans limite).', color: 'success', icon: 'i-lucide-ticket' })
  } catch (e) {
    toast.add({ title: 'Réglage non enregistré', description: String(e), color: 'error' })
  }
}

function leaderboardUrl(login: string) {
  return `https://streamelements.com/${login}/leaderboard`
}

const toast = useToast()

// --- Achat de tickets (compte StreamElements connecté) ---
// Quantité saisie par chaîne
const quantities = reactive<Record<number, number>>({})
const toBuy = ref<{ channel: ChannelView, tickets: number } | null>(null)
const buying = ref(false)

function quantity(channel: ChannelView): number {
  return quantities[channel.id] ?? 1
}

function askBuy(channel: ChannelView) {
  const max = affordable(channel)
  const tickets = Math.max(1, Math.min(quantity(channel), max ?? quantity(channel)))
  toBuy.value = { channel, tickets }
}

async function confirmBuy() {
  const pending = toBuy.value
  if (!pending || !account.value || !pending.channel.seGiveaway) return
  buying.value = true
  try {
    const total = await invoke<number>('buy_se_tickets', {
      accountId: account.value.id,
      channelId: pending.channel.id,
      giveawayId: pending.channel.seGiveaway.id,
      tickets: pending.tickets
    })
    toast.add({ title: `${pending.tickets} ticket(s) acheté(s) sur ${pending.channel.login}`, description: `Tu as maintenant ${total} ticket(s).`, color: 'success', icon: 'i-lucide-ticket' })
    toBuy.value = null
  } catch (e) {
    toast.add({ title: 'Achat refusé', description: String(e), color: 'error' })
  } finally {
    buying.value = false
  }
}

// Relevé du solde : Auto (rythme de la chaîne) ou fréquence imposée
function refreshItems(channel: ChannelView) {
  const auto = channel.seRefresh === null ? channel.sePollMinutes : null
  return [
    { label: auto ? `Auto (${auto} min)` : 'Auto', value: 'auto' },
    ...[2, 5, 10, 15].map(m => ({ label: `${m} min`, value: String(m) }))
  ]
}

async function setRefresh(channel: ChannelView, value: string) {
  try {
    await invoke('set_se_refresh', { id: channel.id, minutes: value === 'auto' ? null : Number(value) })
  } catch (e) {
    toast.add({ title: 'Réglage non enregistré', description: String(e), color: 'error' })
  }
}

// Chance de gagner : nos tickets sur le total des tickets du giveaway (null : inconnue)
function chance(channel: ChannelView): string | null {
  const g = channel.seGiveaway
  const mine = channel.seMyTickets
  if (!g || mine === null || mine <= 0 || g.totalTickets <= 0) return null
  // Total relu moins souvent que nos tickets : jamais plus de 100 %
  const pct = Math.min(100, mine / Math.max(g.totalTickets, mine) * 100)
  if (pct < 0.01) return '< 0,01 %'
  const digits = pct >= 10 ? 0 : pct >= 1 ? 1 : 2
  return `${pct.toLocaleString('fr-FR', { minimumFractionDigits: digits, maximumFractionDigits: digits })} %`
}

function giveawayUrl(login: string, id: string) {
  return `https://streamelements.com/${login}/giveaway/${id}`
}

const BONUS_LABELS: Record<string, string> = {
  follow: 'follow',
  subscriber: 'abonnement',
  tip: 'don',
  cheer: 'bits',
  raid: 'raid',
  host: 'host'
}

// Comment on gagne les points sur cette chaîne, d'après son réglage StreamElements
function earnRule(channel: ChannelView): string | null {
  const l = channel.seLoyalty
  if (!l) return null
  if (!l.enabled) return 'Fidélité désactivée par la chaîne'
  return `${formatNumber(l.amount)} / ${l.interval} min`
}

function earnDetails(channel: ChannelView): string | null {
  const l = channel.seLoyalty
  if (!l?.enabled) return null
  const parts: string[] = []
  if (l.subscriberMultiplier > 1) parts.push(`abonnés ×${l.subscriberMultiplier}`)
  for (const [key, value] of l.bonuses) parts.push(`${BONUS_LABELS[key] ?? key} +${formatNumber(value)}`)
  return parts.length ? parts.join(' · ') : null
}

// Tickets encore achetables avec le solde, dans la limite par personne
function affordable(channel: ChannelView): number | null {
  const g = channel.seGiveaway
  if (!g || g.cost <= 0 || channel.sePoints === null) return null
  let n = Math.floor(channel.sePoints / g.cost)
  if (g.maxTickets !== null) n = Math.min(n, Math.max(0, g.maxTickets - (channel.seMyTickets ?? 0)))
  return n
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <div class="flex items-center gap-3">
      <h1 class="text-xl font-semibold flex-1">StreamElements</h1>
      <AccountPicker />
    </div>

    <UAlert
      v-if="accounts.length === 0"
      title="Connecte d'abord un compte Twitch"
      :actions="[{ label: 'Comptes', to: '/accounts' }]"
      color="neutral"
      variant="subtle"
    />

    <template v-else-if="account">
      <UAlert
        icon="i-lucide-info"
        color="info"
        variant="subtle"
        title="StreamElements donne ses points aux spectateurs présents dans le chat pendant le live, au rythme choisi par chaque streamer."
        description="Le compte reste connecté au chat de ces chaînes, sans jamais écrire, sans limite de chaînes. Le rythme de chaque chaîne est dans « Comment gagner » ; en Auto, le solde est relu à ce rythme (au plus souvent toutes les 2 min)."
      />

      <form class="flex gap-2" @submit.prevent="addChannel">
        <UInput id="new-se-channel" v-model="newLogin" placeholder="Nom ou adresse de la chaîne (ex. lunarsea)" class="flex-1" icon="i-lucide-plus" />
        <UButton type="submit" label="Ajouter" :loading="adding" />
        <!-- Statut live, soldes, règles de gain et giveaways relus tout de suite -->
        <RefreshButton :action="() => account && invoke('refresh_channels', { accountId: account.id, list: 'se' })" />
      </form>

      <div class="flex items-center gap-2 flex-wrap">
        <USelectMenu
          v-model="streamerFilter"
          :items="streamerItems"
          multiple
          icon="i-lucide-filter"
          placeholder="Tous les streamers"
          :search-input="{ placeholder: 'Rechercher un streamer' }"
          class="w-72"
          aria-label="Filtrer par streamer"
        />
        <UButton v-if="streamerFilter.length" label="Tous les streamers" icon="i-lucide-x" size="sm" color="neutral" variant="ghost" @click="streamerFilter = []" />
        <span v-if="streamerFilter.length" class="text-sm text-muted">{{ shown.length }} chaîne(s) sur {{ channels.length }}</span>
      </div>

      <div class="border border-default rounded-lg overflow-x-auto">
        <table class="w-full text-sm">
          <thead class="bg-elevated/60 text-xs uppercase text-muted">
            <tr>
              <th class="px-1.5 py-2"><span class="sr-only">Regarder</span></th>
              <th class="text-left px-1.5 py-2">Chaîne</th>
              <th class="text-left px-1.5 py-2">État</th>
              <th class="text-right px-1.5 py-2 whitespace-nowrap">Solde</th>
              <th class="text-right px-1.5 py-2 whitespace-nowrap">Aujourd'hui</th>
              <th class="text-left px-1.5 py-2 whitespace-nowrap">Comment gagner</th>
              <th class="text-left px-1.5 py-2 whitespace-nowrap">Commande</th>
              <th class="text-right px-1.5 py-2 whitespace-nowrap">Mes tickets</th>
              <th class="text-right px-1.5 py-2">Tickets</th>
              <th class="text-right px-1.5 py-2">Participants</th>
              <th class="text-left px-1.5 py-2">Giveaway</th>
              <th class="text-left px-1.5 py-2 whitespace-nowrap">Titre + description</th>
              <th class="text-left px-1.5 py-2">Cadeaux</th>
              <th class="text-left px-1.5 py-2">Gagnant</th>
              <th class="text-left px-1.5 py-2">Acheter</th>
              <th class="text-center px-1.5 py-2 whitespace-nowrap">
                <UTooltip text="Un ticket acheté dès que le solde le permet, jusqu'au maximum par personne du giveaway">
                  <span>Achat auto</span>
                </UTooltip>
              </th>
              <th class="text-center px-1.5 py-2">Chat</th>
              <th class="px-1.5 py-2" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="channel in shown" :key="channel.id" class="border-t border-default" :class="hasFavoriteGift(channelGifts(channel)) ? FAVORITE_ROW : channel.seGiveaway ? 'bg-primary/5' : ''">
              <td class="px-1.5 py-2">
                <UButton v-if="channel.live" icon="i-lucide-tv" size="xs" color="neutral" variant="ghost" :aria-label="`Regarder ${channel.login}`" title="Regarder (fenêtre multi-stream)" @click="watchStream(channel.login)" />
              </td>
              <td class="px-1.5 py-2 min-w-0">
                <ChannelCell :login="channel.login" :avatar="channel.profileImage" :live="channel.live" />
              </td>
              <td class="px-1.5 py-2">
                <div class="flex flex-col items-start gap-1">
                  <UBadge :color="status(channel).color" variant="subtle" :label="status(channel).label" size="sm" class="whitespace-nowrap" />
                  <UTooltip v-if="channel.seGiveaway" :text="`${channel.seGiveaway.title || 'Giveaway StreamElements'} : ouvrir la page du giveaway`">
                    <button type="button" class="cursor-pointer" @click="openExternal(giveawayUrl(channel.sePage || channel.login, channel.seGiveaway.id))">
                      <UBadge icon="i-lucide-gift" label="Giveaway en cours" color="primary" variant="subtle" size="sm" class="whitespace-nowrap hover:bg-primary/20" />
                    </button>
                  </UTooltip>
                </div>
              </td>
              <td class="px-1.5 py-2 text-right whitespace-nowrap">
                <div>{{ channel.seEnabled === false ? '—' : formatNumber(channel.sePoints) }}</div>
                <button v-if="channel.seRank" class="text-xs text-muted hover:underline" title="Rang au classement de la chaîne" @click="openExternal(leaderboardUrl(channel.sePage || channel.login))">
                  {{ formatNumber(channel.seRank) }}<sup>e</sup>
                </button>
              </td>
              <td class="px-1.5 py-2 text-right whitespace-nowrap" :class="channel.seEnabled !== false && channel.seEarnedToday > 0 ? 'text-success' : 'text-muted'">
                <template v-if="channel.seEnabled !== false">{{ channel.seEarnedToday > 0 ? '+' : '' }}{{ formatNumber(channel.seEarnedToday) }}</template>
                <template v-else>—</template>
              </td>
              <td class="px-1.5 py-2">
                <div v-if="earnRule(channel)" class="flex items-center gap-1 whitespace-nowrap">
                  {{ earnRule(channel) }}
                  <UTooltip v-if="earnDetails(channel)" :text="earnDetails(channel) ?? ''">
                    <UIcon name="i-lucide-info" class="size-3.5 text-muted" />
                  </UTooltip>
                </div>
                <span v-else class="text-muted">—</span>
                <USelect
                  v-if="channel.seEnabled !== false"
                  :id="`se-refresh-${channel.id}`"
                  :model-value="channel.seRefresh === null ? 'auto' : String(channel.seRefresh)"
                  :items="refreshItems(channel)"
                  size="xs"
                  class="w-36 mt-1"
                  icon="i-lucide-refresh-cw"
                  :aria-label="`Relevé du solde sur ${channel.login}`"
                  @update:model-value="v => setRefresh(channel, String(v))"
                />
              </td>

              <!-- Giveaway en cours : commande et prix du ticket -->
              <td class="px-1.5 py-2 whitespace-nowrap">
                <template v-if="channel.seGiveaway">
                  <div class="flex items-center gap-1.5">
                    <span class="font-mono">{{ channel.seGiveaway.command ?? '—' }}</span>
                    <UButton icon="i-lucide-external-link" size="xs" variant="ghost" color="neutral" aria-label="Voir le giveaway" @click="openExternal(giveawayUrl(channel.sePage || channel.login, channel.seGiveaway.id))" />
                  </div>
                  <div class="text-xs text-muted">
                    {{ formatNumber(channel.seGiveaway.cost) }} pts<template v-if="channel.seGiveaway.freeTicket"> · 1 gratuit</template>
                  </div>
                  <UBadge v-if="channel.seGiveaway.subscriberOnly" label="Abonnés uniquement" color="warning" variant="subtle" size="sm" />
                </template>
                <span v-else class="text-muted">—</span>
              </td>
              <td class="px-1.5 py-2 text-right whitespace-nowrap">
                <template v-if="channel.seGiveaway">
                  <b>{{ channel.seMyTickets ?? '?' }}</b>
                  <span class="text-muted"> / {{ channel.seGiveaway.maxTickets ? formatNumber(channel.seGiveaway.maxTickets) : '∞' }}</span>
                  <UTooltip v-if="chance(channel) !== null" text="Chance de gagner : tes tickets sur le total des tickets achetés par tous les participants">
                    <div class="text-xs text-success">{{ chance(channel) }} de chance</div>
                  </UTooltip>
                </template>
                <span v-else class="text-muted">—</span>
              </td>
              <td class="px-1.5 py-2 text-right">
                {{ channel.seGiveaway ? formatNumber(channel.seGiveaway.totalTickets) : '—' }}
              </td>
              <td class="px-1.5 py-2 text-right">
                {{ channel.seGiveaway ? formatNumber(channel.seGiveaway.totalUsers) : '—' }}
              </td>
              <td class="px-1.5 py-2">
                <UBadge v-if="giveawayState(channel)" :label="giveawayState(channel)!.label" :color="giveawayState(channel)!.color" variant="subtle" size="sm" class="whitespace-nowrap" />
                <span v-else class="text-muted">—</span>
              </td>
              <!-- Giveaway StreamElements en cours : titre en gras, puis la description, en entier -->
              <td class="px-1.5 py-2 min-w-[260px] max-w-[420px]">
                <template v-if="channel.seGiveaway?.title?.trim() || channel.seGiveaway?.description?.trim()">
                  <p v-if="channel.seGiveaway.title?.trim()" class="text-xs font-semibold whitespace-pre-line break-words">{{ channel.seGiveaway.title.trim() }}</p>
                  <p v-if="channel.seGiveaway.description?.trim()" class="text-xs whitespace-pre-line break-words text-muted">{{ channel.seGiveaway.description.trim() }}</p>
                </template>
                <span v-else class="text-muted">—</span>
              </td>
              <td class="px-1.5 py-2">
                <div class="flex flex-wrap items-center gap-1 min-w-24 max-w-64">
                  <GiftBadge v-for="x in channelGifts(channel)" :key="x.gift.id" :gift="x.gift" :attached="x.attached" lazy />
                  <span v-if="channelGifts(channel).length === 0" class="text-muted">—</span>
                </div>
              </td>
              <td class="px-1.5 py-2 whitespace-nowrap">
                <div v-if="knownGiveaway(channel)?.winners.length" class="flex flex-col gap-0.5">
                  <span
                    v-for="w in knownGiveaway(channel)!.winners"
                    :key="w"
                    class="inline-flex items-center gap-1"
                    :class="account && w.toLowerCase() === account.login.toLowerCase() ? 'text-success font-medium' : ''"
                  >
                    <UIcon name="i-lucide-trophy" class="size-3.5" />{{ w }}
                    <span v-if="knownGiveaway(channel)!.winnerChances[w.toLowerCase()]" class="text-xs text-muted">({{ knownGiveaway(channel)!.winnerChances[w.toLowerCase()] }})</span>
                  </span>
                </div>
                <span v-else class="text-muted">—</span>
              </td>
              <td class="px-1.5 py-2">
                <template v-if="channel.seGiveaway">
                  <div v-if="account.seAccount === 'connected'" class="flex flex-col gap-1 w-[5.5rem]">
                    <UInputNumber
                      :id="`buy-${channel.id}`"
                      :model-value="quantity(channel)"
                      :min="1"
                      :max="Math.max(1, affordable(channel) ?? 1)"
                      size="xs"
                      class="w-[5.5rem]"
                      :aria-label="`Tickets à acheter sur ${channel.login}`"
                      @update:model-value="v => quantities[channel.id] = v ?? 1"
                    />
                    <UTooltip :text="affordable(channel) ? `${formatNumber(quantity(channel) * channel.seGiveaway.cost)} points` : (channel.seGiveaway.maxTickets && (channel.seMyTickets ?? 0) >= channel.seGiveaway.maxTickets ? 'Maximum atteint' : 'Pas assez de points')">
                      <UButton icon="i-lucide-ticket" label="Acheter" size="xs" block :disabled="!affordable(channel)" @click="askBuy(channel)" />
                    </UTooltip>
                  </div>
                  <RouterLink v-else to="/accounts" class="text-xs text-primary hover:underline">Connecter StreamElements</RouterLink>
                </template>
                <span v-else class="text-muted">—</span>
              </td>
              <!-- Achat automatique : réglage de la chaîne, gardé d'un giveaway à l'autre -->
              <td class="px-1.5 py-2">
                <div class="flex flex-col items-center gap-1">
                  <USwitch
                    v-if="account.seAccount === 'connected'"
                    :model-value="channel.seAutoBuy"
                    :aria-label="`Achat automatique des tickets sur ${channel.login}`"
                    @update:model-value="v => setAutoBuy(channel, v)"
                  />
                  <span v-else class="text-muted">—</span>
                  <!-- Plafond : tickets voulus au plus sur un giveaway (0 : sans limite) -->
                  <UInputNumber
                    v-if="account.seAccount === 'connected' && channel.seAutoBuy"
                    :model-value="channel.seAutoBuyMax"
                    :min="0"
                    :max="100000"
                    size="xs"
                    class="w-24"
                    :aria-label="`Tickets au plus sur ${channel.login} (0 : sans limite)`"
                    :title="channel.seAutoBuyMax ? `${channel.seAutoBuyMax} tickets au plus par giveaway (et jamais plus que le maximum par personne)` : 'Sans limite : jusqu\'au maximum par personne, ou tout le solde si le giveaway n\'en a pas. 0 : sans limite'"
                    @update:model-value="v => setAutoBuyMax(channel, v)"
                  />
                </div>
              </td>
              <td class="px-1.5 py-2">
                <div class="flex justify-center">
                  <USwitch :model-value="optionsOf(channel).chatPresence" :aria-label="`Présence dans le chat de ${channel.login}`" @update:model-value="v => setOptions(channel, { chatPresence: v })" />
                </div>
              </td>
              <td class="px-1.5 py-2 text-right">
                <UButton icon="i-lucide-trash-2" size="xs" variant="ghost" color="error" aria-label="Retirer de la liste" @click="toRemove = channel" />
              </td>
            </tr>
            <tr v-if="shown.length === 0 && channels.length">
              <td colspan="19" class="px-3 py-8 text-center text-muted">Aucune chaîne ne correspond au filtre.</td>
            </tr>
            <tr v-if="channels.length === 0">
              <td colspan="19" class="px-3 py-8 text-center text-muted">Aucune chaîne. Ajoute la première avec le champ ci-dessus.</td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>

    <UModal :open="toBuy !== null" title="Acheter des tickets ?" @update:open="v => { if (!v) toBuy = null }">
      <template #body>
        <p v-if="toBuy && toBuy.channel.seGiveaway">
          {{ toBuy.tickets }} ticket(s) pour le giveaway de {{ toBuy.channel.login }},
          soit <b>{{ formatNumber(toBuy.tickets * toBuy.channel.seGiveaway.cost) }} points</b> StreamElements
          sur les {{ formatNumber(toBuy.channel.sePoints) }} de {{ account?.login }}.
        </p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="toBuy = null" />
          <UButton label="Acheter" icon="i-lucide-ticket" :loading="buying" @click="confirmBuy" />
        </div>
      </template>
    </UModal>

    <UModal :open="toRemove !== null" title="Retirer la chaîne ?" @update:open="v => { if (!v) toRemove = null }">
      <template #body>
        <p>
          {{ account?.login }} quittera le chat de {{ toRemove?.login }} et n'y gagnera plus de points StreamElements.
          <template v-if="toRemove?.inPoints">Elle reste dans la page Points de chaîne.</template>
        </p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="toRemove = null" />
          <UButton label="Retirer" color="error" :loading="removing" @click="confirmRemove" />
        </div>
      </template>
    </UModal>
  </div>
</template>
