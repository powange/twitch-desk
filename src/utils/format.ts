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
export const CHANNEL_STATE: Record<ChannelState, { label: string, color: Color }> = {
  watching: { label: 'En cours', color: 'success' },
  waiting: { label: 'En attente (live, hors quota)', color: 'warning' },
  offline: { label: 'Hors ligne', color: 'neutral' },
  disabled: { label: 'Désactivée', color: 'neutral' },
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
  info: { icon: 'i-lucide-info', color: 'text-info' },
  error: { icon: 'i-lucide-triangle-alert', color: 'text-error' }
}
