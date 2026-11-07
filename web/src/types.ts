// Données lues sur le serveur de synchronisation (GET /api/v1/app/overview)

export type GiveawayKind = 'command' | 'ticket' | 'streamelements' | 'channelpoints'

export interface Gift {
  id: string
  title: string | null
  image: string | null
  gameName: string | null
  group: string | null
}

export interface Giveaway {
  id: string
  channel: string
  kind: GiveawayKind
  // Clos : seulement ceux des dernières 48 h, avec leur heure de clôture
  status: 'open' | 'paused' | 'closed'
  closedAt?: string | null
  command?: string | null
  url?: string | null
  detectedAt: string
  winner?: string | null
  winnerChance?: string | null
  drawAt?: string | null
  drawnAt?: string | null
  notes?: string | null
  ticketUnique?: boolean | null
  ticketMax?: number | null
  rewardName?: string | null
  rewardCost?: number | null
  requireFollow?: boolean | null
  // Versions du PC avant la 0.48.2 : seulement en jours
  followMinDays?: number | null
  followMinSecs?: number | null
  requirePresence?: boolean | null
  requireDiscord?: boolean | null
  requireGreeting?: boolean | null
  minChatMinutes?: number | null
  // Gain offrable à quelqu'un d'autre
  canGift?: boolean | null
  gifts: Gift[]
}

export interface ChannelLive {
  live: boolean
  viewers?: number
  category?: string | null
  title?: string | null
}

// Live d'une chaîne des pages Points et StreamElements (spectateurs arrondis)
export interface ChannelLiveInfo {
  viewers: number
  category: string | null
  title: string | null
}

export type PointsChannelState = 'watching' | 'waiting' | 'offline' | 'disabled' | 'balanceReached' | 'unknown'

export interface PointsChannel {
  login: string
  avatar: string | null
  state: PointsChannelState
  balance: number | null
  earnedToday: number
  maxBalance: number | null
  priority: boolean
  live: ChannelLiveInfo | null
  // Récolte et bonus auto (application 0.48.56 et plus)
  harvest?: boolean
  autoClaim?: boolean
}

export interface SeGiveawayInfo {
  id: string
  title: string
  description: string
  command: string | null
  cost: number
  maxTickets: number | null
  totalTickets: number
  totalUsers: number
}

export interface SeChannel {
  login: string
  avatar: string | null
  // null : pas encore vérifié ; false : la chaîne n'utilise pas StreamElements
  enabled: boolean | null
  points: number | null
  earnedToday: number
  inChat: boolean
  live: ChannelLiveInfo | null
  myTickets: number | null
  giveaway: SeGiveawayInfo | null
  // Réglages de la ligne (application 0.48.55 et plus)
  chatPresence?: boolean
  autoBuy?: boolean
}

// État publié par l'application du PC
export interface InstanceState {
  accounts: string[]
  // hidden : masqué sur le PC (application 0.48.54 et plus)
  giveaways: Record<string, { participated: string[], handled: boolean, hidden?: boolean }>
  live: Record<string, ChannelLive>
  // Pages Points de chaîne et StreamElements (application 0.41 et plus)
  points?: { login: string, enabled: boolean, channels: PointsChannel[] }[]
  streamelements?: { login: string, connected: boolean, channels: SeChannel[] }[]
  // Achats depuis le téléphone autorisés sur le PC
  actionsAllowed?: boolean
  // Langue de diffusion des chaînes affichées, login → code (application 0.48.55 et plus)
  languages?: Record<string, string>
  // Photo de profil des chaînes des giveaways, mentions et drops (application 0.48.55 et plus)
  avatars?: Record<string, string>
  // Application 0.48.56 et plus : en pause, réponses au gain en attente (annulables), Discord des
  // streamers des giveaways en cours
  paused?: boolean
  winReplies?: { id: string, login: string, channel: string, text: string, sendAt: string }[]
  discord?: Record<string, string>
  // Page Drops (application 0.42 et plus)
  drops?: DropsAccount[]
  // Mentions de nos pseudos, les plus récentes d'abord (application 0.45 et plus)
  mentions?: Mention[]
  // Page Scanner (application 0.44 et plus)
  scanner?: { scannedAt: string | null, matches: ScanMatch[] }
  // Version de l'application du PC et dernière vérification des mises à jour (0.43 et plus)
  pc?: { version: string, update: { available: string | null, checkedAt: string } | null }
}

// Un de nos pseudos cité dans un chat surveillé
export interface Mention {
  id: number
  at: string
  channel: string
  // Notre compte mentionné
  account: string
  author: string
  message: string
}

// Stream repéré par le scanner du PC
export interface ScanMatch {
  login: string
  displayName: string
  avatar: string | null
  title: string
  game: string
  viewers: number
  startedAt: string | null
  keywords: string[]
  // Déjà dans les chaînes surveillées
  monitored: boolean
  giveawayKind: GiveawayKind | null
  giveawayCommand: string | null
}

export interface DropProgress {
  id: string
  name: string
  image: string | null
  requiredMinutes: number
  currentMinutes: number
  claimed: boolean
  // Réclamable selon le cœur du PC (absent : PC plus ancien)
  claimable?: boolean
}

export interface DropCampaign {
  id: string
  name: string
  game: string
  boxArt: string
  endsAt: string
  accountLinked: boolean
  // Campagne de la chaîne regardée en ce moment
  watched: boolean
  drops: DropProgress[]
}

export interface DropsAccount {
  login: string
  enabled: boolean
  target: { game: string, login: string, title: string } | null
  campaigns: DropCampaign[]
  // Campagnes ignorées, à rétablir (application 0.48.56 et plus)
  ignored?: { id: string, name: string, game: string }[]
}

// Demande envoyée au PC (achat de tickets StreamElements)
export interface PhoneAction {
  id: string
  kind: string
  // Achat de tickets ; vide pour une mise à jour (« update_check », « update_install »)
  payload: { account?: string, channel?: string, giveaway?: string, tickets?: number, item?: string, hidden?: boolean, conditions?: Record<string, unknown> }
  status: 'pending' | 'done' | 'failed'
  message: string | null
  // Résultat détaillé (journal du chat), serveur 0.8.10 et plus
  data?: { channel?: string, entries?: ChatLogEntry[] }
  createdAt: string
  updatedAt: string
}

// Message du journal du chat d'un giveaway, lu par le PC
export interface ChatLogEntry {
  at: string
  author: string
  kind: string
  command: string | null
  message: string
}

// Conditions d'un giveaway modifiables depuis le téléphone (null : on ne sait pas)
export interface GiveawayConditions {
  requirePresence: boolean | null
  requireDiscord: boolean | null
  requireFollow: boolean | null
  requireGreeting: boolean | null
  minChatMinutes: number | null
  canGift: boolean | null
}

export interface Overview {
  instance: { id: string, name: string }
  device: { id: string, name: string }
  state: InstanceState | null
  stateAt: string | null
  giveaways: Giveaway[]
  seq: number
}
