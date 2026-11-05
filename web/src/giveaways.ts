import type { Giveaway, GiveawayKind } from './types'

type BadgeColor = 'primary' | 'secondary' | 'success' | 'info' | 'warning' | 'error' | 'neutral'

// Repris de src/utils/giveaways.ts de l'application
export const GIVEAWAY_KIND: Record<GiveawayKind, { label: string, color: BadgeColor }> = {
  command: { label: 'Commande', color: 'primary' },
  ticket: { label: 'Ticket', color: 'info' },
  streamelements: { label: 'StreamElements', color: 'warning' },
  channelpoints: { label: 'Points de chaîne', color: 'secondary' }
}

export interface Condition {
  key: string
  icon: string
  // Libellé court, affiché à côté du picto (pas d'infobulle au doigt)
  label: string
  required: boolean
  colorClass: string
}

// Conditions connues (non renseignée : rien d'affiché)
// Ancienneté du follow requise : « 7j », « 2h 30min » ; null : pas de minimum
function followMin(g: Giveaway): string | null {
  const secs = g.followMinSecs ?? (g.followMinDays ? g.followMinDays * 86_400 : null)
  if (!secs) return null
  const parts: string[] = []
  let rest = secs
  for (const [unit, size] of [['j', 86_400], ['h', 3_600], ['min', 60], ['s', 1]] as const) {
    const n = Math.floor(rest / size)
    if (n) parts.push(`${n}${unit}`)
    rest -= n * size
  }
  return parts.join(' ')
}

export function conditions(g: Giveaway): Condition[] {
  const out: Condition[] = []
  if (g.kind === 'ticket' && g.ticketUnique != null) {
    out.push(g.ticketUnique
      ? { key: 'ticket', icon: 'i-lucide-ticket-check', label: '1 ticket par personne', required: true, colorClass: 'text-amber-600 dark:text-amber-400' }
      : { key: 'ticket', icon: 'i-lucide-ticket-plus', label: g.ticketMax ? `Jusqu'à ${g.ticketMax} tickets` : 'Multi-tickets', required: true, colorClass: 'text-sky-600 dark:text-sky-400' })
  }
  if (g.requirePresence != null) {
    out.push(g.requirePresence
      ? { key: 'presence', icon: 'i-lucide-user-check', label: 'Présence au tirage', required: true, colorClass: 'text-emerald-600 dark:text-emerald-400' }
      : { key: 'presence', icon: 'i-lucide-user-x', label: 'Présence non requise', required: false, colorClass: 'text-muted' })
  }
  if (g.requireDiscord != null) {
    out.push(g.requireDiscord
      ? { key: 'discord', icon: 'i-simple-icons-discord', label: 'Discord', required: true, colorClass: 'text-indigo-600 dark:text-indigo-400' }
      : { key: 'discord', icon: 'i-simple-icons-discord', label: 'Discord non requis', required: false, colorClass: 'text-muted' })
  }
  if (g.requireFollow != null) {
    out.push(g.requireFollow
      ? { key: 'follow', icon: 'i-lucide-heart', label: followMin(g) ? `Follow depuis ${followMin(g)}` : 'Follow', required: true, colorClass: 'text-pink-600 dark:text-pink-400' }
      : { key: 'follow', icon: 'i-lucide-heart-off', label: 'Follow non requis', required: false, colorClass: 'text-muted' })
  }
  if (g.requireGreeting != null) {
    out.push(g.requireGreeting
      ? { key: 'greeting', icon: 'i-lucide-hand', label: 'Dire bonjour', required: true, colorClass: 'text-amber-600 dark:text-amber-400' }
      : { key: 'greeting', icon: 'i-lucide-hand', label: 'Bonjour non requis', required: false, colorClass: 'text-muted' })
  }
  if (g.minChatMinutes != null) {
    out.push(g.minChatMinutes > 0
      ? { key: 'chat', icon: 'i-lucide-timer', label: `${g.minChatMinutes} min dans le chat`, required: true, colorClass: 'text-sky-600 dark:text-sky-400' }
      : { key: 'chat', icon: 'i-lucide-timer-off', label: 'Pas de temps de chat', required: false, colorClass: 'text-muted' })
  }
  if (g.canGift != null) {
    out.push(g.canGift
      ? { key: 'gift', icon: 'i-lucide-hand-heart', label: 'Gain offrable', required: true, colorClass: 'text-rose-600 dark:text-rose-400' }
      : { key: 'gift', icon: 'i-lucide-hand-heart', label: 'Gain non offrable', required: false, colorClass: 'text-muted' })
  }
  return out
}

// Liens Twitch : sur téléphone, l'app Twitch s'ouvre à leur place quand elle est installée
export const streamUrl = (login: string) => `https://www.twitch.tv/${encodeURIComponent(login)}`
export const chatUrl = (login: string) => `https://www.twitch.tv/popout/${encodeURIComponent(login)}/chat`
export const playerUrl = (login: string, parent = location.hostname) =>
  `https://player.twitch.tv/?channel=${encodeURIComponent(login)}&parent=${encodeURIComponent(parent)}&muted=true&autoplay=true`

// Copie dans le presse-papiers (repli pour les navigateurs sans API Clipboard)
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    const area = document.createElement('textarea')
    area.value = text
    area.setAttribute('readonly', '')
    area.style.position = 'fixed'
    area.style.opacity = '0'
    document.body.appendChild(area)
    area.select()
    let ok = false
    try {
      ok = document.execCommand('copy')
    } catch { /* copie refusée */ }
    area.remove()
    return ok
  }
}
