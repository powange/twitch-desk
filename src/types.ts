// Miroir des structures envoyées par le cœur Rust (src-tauri/src/app.rs, db.rs)

export type AccountStatus = 'starting' | 'running' | 'expired' | 'error'
export type ChannelState = 'watching' | 'waiting' | 'offline' | 'disabled' | 'unknown'
export type EventKind = 'bonus' | 'gain' | 'live' | 'offline' | 'raid' | 'drop' | 'info' | 'error'

export interface StreamInfo {
  broadcastId: string
  title: string
  gameId: string | null
  gameName: string | null
  viewers: number
}

export interface ChannelView {
  id: number
  login: string
  channelId: string
  enabled: boolean
  autoClaim: boolean
  balance: number | null
  earnedToday: number
  live: StreamInfo | null
  state: ChannelState
  error: string | null
  lastBonusAt: string | null
  // Listes où figure la chaîne : page Points de chaîne, page StreamElements
  inPoints: boolean
  inSe: boolean
  chatPresence: boolean
  inChat: boolean
  seRank: number | null
  seLoyalty: SeLoyalty | null
  seGiveaway: SeGiveaway | null
  // Nos tickets dans le giveaway en cours, lus dans les réponses du bot (null : inconnu)
  seMyTickets: number | null
  // Relevé du solde StreamElements : choisi (null : automatique) et appliqué, en minutes
  seRefresh: number | null
  sePollMinutes: number
  // null : pas encore vérifié ; false : la chaîne n'utilise pas StreamElements
  seEnabled: boolean | null
  sePoints: number | null
  seEarnedToday: number
}

export interface AccountView {
  id: number
  login: string
  userId: string
  pointsEnabled: boolean
  status: AccountStatus
  statusText: string | null
  realtime: boolean
  chatConnected: boolean
  // Compte StreamElements du spectateur : lecture exacte et achat des tickets
  seAccount: 'none' | 'connected' | 'expired'
  drops: DropsView
  channels: ChannelView[]
}

export interface LogEvent {
  at: string
  account: string
  kind: EventKind
  message: string
}

export interface Snapshot {
  paused: boolean
  accounts: AccountView[]
  events: LogEvent[]
  operationsVersion: number
  operationsUpdated: string
}

export interface DeviceCode {
  device_code: string
  user_code: string
  verification_uri: string
  expires_in: number
  interval: number
}

export type LoginEvent
  = | { status: 'done', login: string }
    | { status: 'expired' }
    | { status: 'denied' }
    | { status: 'error', message: string }

export interface Settings {
  closeToTray: boolean
  startMinimized: boolean
  autoUpdate: boolean
}

export type ChannelList = 'points' | 'se'

export type SeLoginEvent
  = | { status: 'done', accountId: number, username: string }
    | { status: 'wrongAccount', accountId: number, username: string }
    | { status: 'cancelled', accountId: number }
    | { status: 'error', accountId: number, message: string }

export interface SeLoyalty {
  enabled: boolean
  name: string
  amount: number
  interval: number
  subscriberMultiplier: number
  bonuses: [string, number][]
}

export interface SeGiveaway {
  id: string
  title: string
  description: string
  command: string | null
  cost: number
  maxTickets: number | null
  freeTicket: boolean
  subscriberOnly: boolean
  totalTickets: number
  totalUsers: number
  botResponses: boolean
}

// --- Drops ---

export type DropGameStatus = 'unknown' | 'watching' | 'waiting' | 'noStreams' | 'done' | 'noProgress' | 'disabled'

export interface DropGameView {
  id: number
  gameId: string
  name: string
  slug: string
  boxArt: string | null
  enabled: boolean
  status: DropGameStatus
  streams: number
}

export interface DropReward {
  name: string
  image: string | null
}

export interface TimedDrop {
  id: string
  name: string
  requiredMinutes: number
  currentMinutes: number
  claimed: boolean
  instanceId: string | null
  startsAt: string
  endsAt: string
  rewards: DropReward[]
  preconditions: string[]
}

export interface DropCampaign {
  id: string
  name: string
  game: { id: string, name: string, slug: string | null, boxArt: string | null }
  status: string
  startsAt: string
  endsAt: string
  accountLinked: boolean
  linkUrl: string | null
  allowedChannels: string[]
  drops: TimedDrop[]
}

export interface DropsView {
  enabled: boolean
  games: DropGameView[]
  campaigns: DropCampaign[]
  target: { gameName: string, login: string, viewers: number, title: string } | null
  inventoryAt: string | null
  error: string | null
}
