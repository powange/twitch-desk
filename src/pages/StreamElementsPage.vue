<script setup lang="ts">
import { useChannelList } from '../composables/useChannelList'
import { useOpenExternal } from '../utils/external'
import { formatNumber } from '../utils/format'
import type { ChannelView } from '../types'

const openExternal = useOpenExternal()
const { accounts, accountId, account, accountItems, channels, newLogin, adding, addChannel, setOptions, toRemove, confirmRemove } = useChannelList('se')

// Pourquoi une chaîne ne rapporte pas (ou rien : elle rapporte)
function status(channel: ChannelView): { label: string, color: 'success' | 'warning' | 'neutral' | 'error' } {
  if (!channel.chatPresence) return { label: 'Chat désactivé', color: 'neutral' }
  if (channel.seEnabled === false) return { label: 'Pas de StreamElements', color: 'error' }
  if (!channel.inChat) return { label: 'Connexion au chat…', color: 'neutral' }
  if (!channel.live) return { label: 'Dans le chat · hors ligne', color: 'neutral' }
  return { label: 'Dans le chat · en live', color: 'success' }
}

function leaderboardUrl(login: string) {
  return `https://streamelements.com/${login}/leaderboard`
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
      <USelect v-if="accounts.length > 1" v-model="accountId" :items="accountItems" class="w-48" icon="i-lucide-user" />
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
        title="StreamElements donne ses points toutes les 10 minutes aux spectateurs présents dans le chat pendant le live."
        description="Le compte reste connecté au chat de ces chaînes, sans jamais écrire. Pas de limite de chaînes. Le solde est relu toutes les 10 minutes."
      />

      <form class="flex gap-2" @submit.prevent="addChannel">
        <UInput id="new-se-channel" v-model="newLogin" placeholder="Nom ou adresse de la chaîne (ex. lunarsea)" class="flex-1" icon="i-lucide-plus" />
        <UButton type="submit" label="Ajouter" :loading="adding" />
      </form>

      <div class="border border-default rounded-lg overflow-x-auto">
        <table class="w-full text-sm">
          <thead class="bg-elevated/60 text-xs uppercase text-muted">
            <tr>
              <th class="text-left px-3 py-2">Chaîne</th>
              <th class="text-left px-3 py-2">État</th>
              <th class="text-right px-3 py-2">Solde</th>
              <th class="text-right px-3 py-2">Aujourd'hui</th>
              <th class="text-left px-3 py-2">Comment gagner</th>
              <th class="text-center px-3 py-2">Chat</th>
              <th class="px-3 py-2" />
            </tr>
          </thead>
          <tbody>
            <template v-for="channel in channels" :key="channel.id">
            <tr class="border-t border-default">
              <td class="px-3 py-2 min-w-0">
                <button class="font-medium hover:underline" @click="openExternal(`https://www.twitch.tv/${channel.login}`)">{{ channel.login }}</button>
                <div v-if="channel.live" class="text-xs text-muted truncate max-w-[320px]">
                  {{ channel.live.gameName ?? 'Sans catégorie' }} · {{ formatNumber(channel.live.viewers) }} spectateurs
                </div>
              </td>
              <td class="px-3 py-2">
                <UBadge :color="status(channel).color" variant="subtle" :label="status(channel).label" class="whitespace-nowrap" />
              </td>
              <td class="px-3 py-2 text-right whitespace-nowrap">
                <div>{{ channel.seEnabled === false ? '—' : formatNumber(channel.sePoints) }}</div>
                <button v-if="channel.seRank" class="text-xs text-muted hover:underline" @click="openExternal(leaderboardUrl(channel.login))">
                  {{ formatNumber(channel.seRank) }}<sup>e</sup> au classement
                </button>
              </td>
              <td class="px-3 py-2 text-right" :class="channel.seEarnedToday > 0 ? 'text-success' : 'text-muted'">
                {{ channel.seEarnedToday > 0 ? '+' : '' }}{{ formatNumber(channel.seEarnedToday) }}
              </td>
              <td class="px-3 py-2">
                <template v-if="earnRule(channel)">
                  <div class="whitespace-nowrap">{{ earnRule(channel) }}</div>
                  <div v-if="earnDetails(channel)" class="text-xs text-muted min-w-[190px]">{{ earnDetails(channel) }}</div>
                </template>
                <span v-else class="text-muted">—</span>
              </td>
              <td class="px-3 py-2 text-center">
                <div class="flex justify-center">
                  <USwitch :model-value="channel.chatPresence" :aria-label="`Présence dans le chat de ${channel.login}`" @update:model-value="v => setOptions(channel, { chatPresence: v })" />
                </div>
              </td>
              <td class="px-3 py-2 text-right">
                <UButton icon="i-lucide-trash-2" size="xs" variant="ghost" color="error" aria-label="Retirer de la liste" @click="toRemove = channel" />
              </td>
            </tr>
            <tr v-if="channel.seGiveaway" class="bg-primary/5">
              <td colspan="7" class="px-3 pb-3 pt-1">
                <div class="flex items-center gap-x-5 gap-y-1 flex-wrap text-sm">
                  <span class="inline-flex items-center gap-1.5 font-medium text-primary">
                    <UIcon name="i-lucide-gift" class="size-4" />
                    Giveaway en cours<template v-if="channel.seGiveaway.title"> : {{ channel.seGiveaway.title }}</template>
                  </span>
                  <span v-if="channel.seGiveaway.command"><span class="font-mono bg-elevated px-1.5 py-0.5 rounded">{{ channel.seGiveaway.command }}</span></span>
                  <span>{{ formatNumber(channel.seGiveaway.cost) }} points le ticket<template v-if="channel.seGiveaway.freeTicket"> · 1 gratuit</template></span>
                  <span>
                    Mes tickets :
                    <b>{{ channel.seMyTickets ?? (channel.seGiveaway.botResponses ? '?' : 'inconnu') }}</b>
                    <template v-if="channel.seGiveaway.maxTickets"> / {{ formatNumber(channel.seGiveaway.maxTickets) }} max</template>
                    <template v-else> (pas de maximum)</template>
                  </span>
                  <span v-if="affordable(channel) !== null" class="text-muted">encore {{ formatNumber(affordable(channel)) }} achetable(s)</span>
                  <span class="text-muted">{{ formatNumber(channel.seGiveaway.totalTickets) }} tickets · {{ formatNumber(channel.seGiveaway.totalUsers) }} participants</span>
                  <UBadge v-if="channel.seGiveaway.subscriberOnly" label="Abonnés uniquement" color="warning" variant="subtle" />
                  <UButton label="Voir" size="xs" variant="link" icon="i-lucide-external-link" @click="openExternal(giveawayUrl(channel.login, channel.seGiveaway.id))" />
                </div>
              </td>
            </tr>
            </template>
            <tr v-if="channels.length === 0">
              <td colspan="7" class="px-3 py-8 text-center text-muted">Aucune chaîne. Ajoute la première avec le champ ci-dessus.</td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>

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
          <UButton label="Retirer" color="error" @click="confirmRemove" />
        </div>
      </template>
    </UModal>
  </div>
</template>
