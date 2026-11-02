<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { useChannelList } from '../composables/useChannelList'
import { useSnapshot } from '../composables/useSnapshot'
import { useOpenExternal } from '../utils/external'
import { formatNumber } from '../utils/format'
import type { ChannelView, GiveawayView } from '../types'
import ChannelFlag from '../components/ChannelFlag.vue'
import AccountPicker from '../components/AccountPicker.vue'
import { stringList, usePersisted } from '../composables/usePersisted'

const openExternal = useOpenExternal()
const { accounts, account, channels, newLogin, adding, addChannel, setOptions, optionsOf, toRemove, removing, confirmRemove } = useChannelList('se')

// Filtre par streamer (choix multiple), retenu sur ce poste ; « ?channels=a,b » le préremplit
// pour cette visite (lien depuis les alertes) sans remplacer le filtre retenu
const route = useRoute()
const savedFilter = usePersisted('se.streamers', [] as string[], { sanitize: stringList })
const streamerFilter = ref<string[]>([...savedFilter.value])
// Filtre venu du lien, pas encore modifié à la main : pas retenu
let fromUrl: string | null = null
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
function knownGiveaway(channel: ChannelView): GiveawayView | null {
  const list = seGiveaways.value.get(channel.login) ?? []
  const active = channel.seGiveaway
  if (active) return list.find(g => g.url?.includes(active.id)) ?? null
  const recent = list.filter(g => g.status !== 'closed' || Date.now() - new Date(g.detectedAt).getTime() < RECENT_MS)
  return recent.sort((a, b) => b.detectedAt.localeCompare(a.detectedAt))[0] ?? null
}
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
              <th class="text-left px-1.5 py-2">Gagnant</th>
              <th class="text-left px-1.5 py-2">Acheter</th>
              <th class="text-center px-1.5 py-2">Chat</th>
              <th class="px-1.5 py-2" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="channel in shown" :key="channel.id" class="border-t border-default" :class="channel.seGiveaway ? 'bg-primary/5' : ''">
              <td class="px-1.5 py-2 min-w-0">
                <div class="flex items-center gap-1.5">
                  <ChannelFlag :login="channel.login" />
                  <button class="font-medium hover:underline" @click="openExternal(`https://www.twitch.tv/${channel.login}`)">{{ channel.login }}</button>
                </div>
                <div v-if="channel.live" class="flex items-center gap-1.5 text-xs text-muted min-w-0">
                  <span class="truncate max-w-[165px]" :title="channel.live.gameName ?? 'Sans catégorie'">{{ channel.live.gameName ?? 'Sans catégorie' }}</span>
                  <span class="inline-flex items-center gap-0.5 whitespace-nowrap text-error" :title="`${formatNumber(channel.live.viewers)} spectateurs`">
                    <UIcon name="i-lucide-eye" class="size-3.5" />{{ formatNumber(channel.live.viewers) }}
                  </span>
                </div>
              </td>
              <td class="px-1.5 py-2">
                <div class="flex flex-col items-start gap-1">
                  <UBadge :color="status(channel).color" variant="subtle" :label="status(channel).label" size="sm" class="whitespace-nowrap" />
                  <UTooltip v-if="channel.seGiveaway" :text="`${channel.seGiveaway.title || 'Giveaway StreamElements'} : ouvrir la page du giveaway`">
                    <button type="button" class="cursor-pointer" @click="openExternal(giveawayUrl(channel.login, channel.seGiveaway.id))">
                      <UBadge icon="i-lucide-gift" label="Giveaway en cours" color="primary" variant="subtle" size="sm" class="whitespace-nowrap hover:bg-primary/20" />
                    </button>
                  </UTooltip>
                </div>
              </td>
              <td class="px-1.5 py-2 text-right whitespace-nowrap">
                <div>{{ channel.seEnabled === false ? '—' : formatNumber(channel.sePoints) }}</div>
                <button v-if="channel.seRank" class="text-xs text-muted hover:underline" title="Rang au classement de la chaîne" @click="openExternal(leaderboardUrl(channel.login))">
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
                    <UButton icon="i-lucide-external-link" size="xs" variant="ghost" color="neutral" aria-label="Voir le giveaway" @click="openExternal(giveawayUrl(channel.login, channel.seGiveaway.id))" />
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
              <td class="px-1.5 py-2 whitespace-nowrap">
                <span v-if="knownGiveaway(channel)?.winner" class="inline-flex items-center gap-1" :class="knownGiveaway(channel)?.winnerOurs ? 'text-success font-medium' : ''">
                  <UIcon name="i-lucide-trophy" class="size-3.5" />{{ knownGiveaway(channel)!.winner }}
                </span>
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
              <td colspan="14" class="px-3 py-8 text-center text-muted">Aucune chaîne ne correspond au filtre.</td>
            </tr>
            <tr v-if="channels.length === 0">
              <td colspan="14" class="px-3 py-8 text-center text-muted">Aucune chaîne. Ajoute la première avec le champ ci-dessus.</td>
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
