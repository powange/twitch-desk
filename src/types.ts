// Miroir des structures envoyées par le cœur Rust (src-tauri/src/app.rs, db.rs)

export type AccountStatus = 'starting' | 'running' | 'expired' | 'error'
export type ChannelState = 'watching' | 'waiting' | 'offline' | 'disabled' | 'unknown'
export type EventKind = 'bonus' | 'gain' | 'live' | 'offline' | 'raid' | 'info' | 'error'

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
}

export interface AccountView {
  id: number
  login: string
  userId: string
  pointsEnabled: boolean
  status: AccountStatus
  statusText: string | null
  realtime: boolean
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
