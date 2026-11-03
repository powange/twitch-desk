import type { GiveawayDetails, GiveawayKind } from '../types'

type BadgeColor = 'primary' | 'secondary' | 'success' | 'info' | 'warning' | 'error' | 'neutral'

// Libellé et couleur de chaque type (repris du site Giveaway Twitch)
export const GIVEAWAY_KIND: Record<GiveawayKind, { label: string, color: BadgeColor }> = {
  command: { label: 'Commande', color: 'primary' },
  ticket: { label: 'Ticket', color: 'info' },
  streamelements: { label: 'StreamElements', color: 'warning' },
  channelpoints: { label: 'Points de chaîne', color: 'secondary' }
}

export const GIVEAWAY_KIND_ITEMS = (Object.keys(GIVEAWAY_KIND) as GiveawayKind[]).map(value => ({ label: GIVEAWAY_KIND[value].label, value }))

export interface ParticipationCondition {
  key: string
  icon: string
  // Phrase complète : infobulle et nom accessible
  label: string
  colorClass: string
  // Variante « non requis » sans picto barré (Discord) : trait diagonal ajouté
  strike: boolean
  suffix?: string
}

// Conditions affichées en pictos (port de app/utils/participationConditions.ts du site).
// Tri-état : true = requis, false = non requis, null = on ne sait pas (rien d'affiché).
export function participationConditions(d: GiveawayDetails, kind: GiveawayKind): ParticipationCondition[] {
  const out: ParticipationCondition[] = []
  if (kind === 'ticket' && d.ticketUnique !== null) {
    out.push(d.ticketUnique
      ? { key: 'ticket', icon: 'i-lucide-ticket-check', label: 'Commande à usage unique : 1 ticket par personne', colorClass: 'text-amber-600 dark:text-amber-400', strike: false }
      : {
          key: 'ticket',
          icon: 'i-lucide-ticket-plus',
          label: d.ticketMax ? `Multi-tickets : ${d.ticketMax} au plus par personne` : 'Commande multi-tickets : plusieurs tickets possibles',
          colorClass: 'text-sky-600 dark:text-sky-400',
          strike: false,
          suffix: d.ticketMax ? `×${d.ticketMax}` : undefined
        })
  }
  // Dans l'ordre des colonnes des tableaux : présence, Discord, follow, bonjour, temps dans le chat
  if (d.requirePresence !== null) {
    out.push(d.requirePresence
      ? { key: 'presence', icon: 'i-lucide-user-check', label: 'Présence requise au moment du tirage', colorClass: 'text-emerald-600 dark:text-emerald-400', strike: false }
      : { key: 'presence', icon: 'i-lucide-user-x', label: 'Présence non requise au tirage', colorClass: 'text-muted', strike: false })
  }
  if (d.requireDiscord !== null) {
    out.push({
      key: 'discord',
      icon: 'i-simple-icons-discord',
      label: d.requireDiscord ? 'Être sur le Discord du streamer' : 'Discord non requis',
      colorClass: d.requireDiscord ? 'text-indigo-600 dark:text-indigo-400' : 'text-muted',
      strike: !d.requireDiscord
    })
  }
  if (d.requireFollow !== null) {
    out.push(d.requireFollow
      ? {
          key: 'follow',
          icon: 'i-lucide-heart',
          label: d.followMinDays ? `Follow requis depuis au moins ${d.followMinDays} jour(s)` : 'Follow requis pour participer',
          colorClass: 'text-pink-600 dark:text-pink-400',
          strike: false,
          suffix: d.followMinDays ? `${d.followMinDays} j` : undefined
        }
      : { key: 'follow', icon: 'i-lucide-heart-off', label: 'Follow non requis', colorClass: 'text-muted', strike: false })
  }
  if (d.requireGreeting !== null) {
    out.push(d.requireGreeting
      ? { key: 'greeting', icon: 'i-lucide-hand', label: 'Avoir dit bonjour dans le chat', colorClass: 'text-amber-600 dark:text-amber-400', strike: false }
      : { key: 'greeting', icon: 'i-lucide-hand', label: 'Bonjour non requis', colorClass: 'text-muted', strike: true })
  }
  if (d.minChatMinutes !== null) {
    out.push(d.minChatMinutes > 0
      ? { key: 'chat', icon: 'i-lucide-timer', label: `Présent dans le chat depuis au moins ${d.minChatMinutes} min`, colorClass: 'text-sky-600 dark:text-sky-400', strike: false, suffix: `${d.minChatMinutes} min` }
      : { key: 'chat', icon: 'i-lucide-timer-off', label: 'Pas de temps de présence minimum', colorClass: 'text-muted', strike: false })
  }
  return out
}

// Heure du tirage : « 22:30 » aujourd'hui, sinon jour et heure
export function formatDrawAt(iso: string | null): string {
  if (!iso) return '—'
  const d = new Date(iso)
  const today = new Date()
  const time = d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' })
  return d.toDateString() === today.toDateString() ? time : `${d.toLocaleDateString('fr-FR', { weekday: 'short', day: 'numeric', month: 'short' })} ${time}`
}
