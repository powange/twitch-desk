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
  status: 'open' | 'paused'
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
}

// État publié par l'application du PC
export interface InstanceState {
  accounts: string[]
  giveaways: Record<string, { participated: string[], handled: boolean }>
  live: Record<string, ChannelLive>
  // Pages Points de chaîne et StreamElements (application 0.41 et plus)
  points?: { login: string, enabled: boolean, channels: PointsChannel[] }[]
  streamelements?: { login: string, connected: boolean, channels: SeChannel[] }[]
  // Achats depuis le téléphone autorisés sur le PC
  actionsAllowed?: boolean
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
}

// Demande envoyée au PC (achat de tickets StreamElements)
export interface PhoneAction {
  id: string
  kind: string
  // Achat de tickets ; vide pour une mise à jour (« update_check », « update_install »)
  payload: { account?: string, channel?: string, giveaway?: string, tickets?: number }
  status: 'pending' | 'done' | 'failed'
  message: string | null
  createdAt: string
  updatedAt: string
}

export interface Overview {
  instance: { id: string, name: string }
  device: { id: string, name: string }
  state: InstanceState | null
  stateAt: string | null
  giveaways: Giveaway[]
  seq: number
}
