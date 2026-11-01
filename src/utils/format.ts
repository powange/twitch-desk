import { reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { AccountStatus, ChannelState, EventKind } from '../types'

const numberFormat = new Intl.NumberFormat('fr-FR')

export function formatNumber(n: number | null | undefined): string {
  return n === null || n === undefined ? '—' : numberFormat.format(n)
}

export function formatTime(iso: string): string {
  return new Date(iso).toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' })
}

type Color = 'primary' | 'success' | 'warning' | 'error' | 'info' | 'neutral'

// Chaque état a une couleur ET un mot : on ne lit jamais l'état à la couleur seule
export const CHANNEL_STATE: Record<ChannelState, { label: string, color: Color, hint?: string }> = {
  watching: { label: 'En cours', color: 'success', hint: 'Le temps de visionnage est compté' },
  waiting: { label: 'En attente', color: 'warning', hint: 'En live, mais 2 chaînes plus haut dans la liste sont déjà regardées' },
  offline: { label: 'Hors ligne', color: 'neutral' },
  disabled: { label: 'Désactivée', color: 'neutral' },
  balanceReached: { label: 'Solde atteint', color: 'info', hint: 'Solde maximum atteint : la place revient à la chaîne suivante' },
  unknown: { label: 'Vérification…', color: 'neutral' }
}

export const ACCOUNT_STATUS: Record<AccountStatus, { label: string, color: Color }> = {
  starting: { label: 'Connexion…', color: 'neutral' },
  running: { label: 'Connecté', color: 'success' },
  expired: { label: 'Reconnexion nécessaire', color: 'error' },
  error: { label: 'Erreur', color: 'warning' }
}

export const EVENT_KIND: Record<EventKind, { icon: string, color: string }> = {
  bonus: { icon: 'i-lucide-gift', color: 'text-primary' },
  gain: { icon: 'i-lucide-coins', color: 'text-success' },
  live: { icon: 'i-lucide-radio', color: 'text-error' },
  offline: { icon: 'i-lucide-moon', color: 'text-muted' },
  raid: { icon: 'i-lucide-swords', color: 'text-warning' },
  drop: { icon: 'i-lucide-package-open', color: 'text-primary' },
  giveaway: { icon: 'i-lucide-gift', color: 'text-primary' },
  info: { icon: 'i-lucide-info', color: 'text-info' },
  error: { icon: 'i-lucide-triangle-alert', color: 'text-error' }
}

// « il y a 3 min », « il y a 2 h », sinon la date
export function timeAgo(iso: string, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - new Date(iso).getTime()) / 1000))
  if (s < 60) return 'à l\'instant'
  if (s < 3600) return `il y a ${Math.floor(s / 60)} min`
  if (s < 86400) return `il y a ${Math.floor(s / 3600)} h`
  return new Date(iso).toLocaleDateString('fr-FR', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}

// Jaquette d'une catégorie Twitch, en double résolution. Adresse inconnue : construite
// à partir de l'identifiant du jeu (ttv-boxart/<id>-<l>x<h>.jpg)
// Jaquettes connues par identifiant de jeu. L'adresse ne se déduit pas de l'identifiant (les
// jeux récents sont en « <id>_IGDB-… ») : une adresse inconnue est demandée à Twitch, par lots
const boxArts = reactive(new Map<string, string>())
const pendingIds = new Set<string>()
const requestedIds = new Set<string>()
let boxArtTimer: ReturnType<typeof setTimeout> | null = null

function requestBoxArt(gameId: string) {
  if (!gameId || requestedIds.has(gameId)) return
  requestedIds.add(gameId)
  pendingIds.add(gameId)
  boxArtTimer ??= setTimeout(() => {
    boxArtTimer = null
    const ids = [...pendingIds]
    pendingIds.clear()
    for (let i = 0; i < ids.length; i += 50) {
      invoke<Record<string, string>>('game_box_arts', { ids: ids.slice(i, i + 50) })
        .then((found) => { for (const [id, url] of Object.entries(found)) boxArts.set(id, url) })
        .catch(() => { /* Jaquette de secours */ })
    }
  }, 100)
}

export function boxArt(url: string | null, gameId: string): string {
  const known = url ?? boxArts.get(gameId)
  if (!known) {
    requestBoxArt(gameId)
    return `https://static-cdn.jtvnw.net/ttv-boxart/${gameId}-80x112.jpg`
  }
  return known.replace(/-\d+x\d+\./, '-80x112.').replace('{width}', '80').replace('{height}', '112')
}

export function channelUrl(login: string): string {
  return `https://www.twitch.tv/${login}`
}
