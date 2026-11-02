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
  followMinDays?: number | null
  requirePresence?: boolean | null
  requireDiscord?: boolean | null
  requireGreeting?: boolean | null
  minChatMinutes?: number | null
  gifts: Gift[]
}

export interface ChannelLive {
  live: boolean
  viewers?: number
  category?: string | null
  title?: string | null
}

// État publié par l'application du PC
export interface InstanceState {
  accounts: string[]
  giveaways: Record<string, { participated: string[], handled: boolean }>
  live: Record<string, ChannelLive>
}

export interface Overview {
  instance: { id: string, name: string }
  device: { id: string, name: string }
  state: InstanceState | null
  stateAt: string | null
  giveaways: Giveaway[]
  seq: number
}
