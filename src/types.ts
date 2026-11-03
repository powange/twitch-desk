// Miroir des structures envoyées par le cœur Rust (src-tauri/src/app.rs, db.rs)

export type AccountStatus = 'starting' | 'running' | 'expired' | 'error'
export type ChannelState = 'watching' | 'waiting' | 'offline' | 'disabled' | 'balanceReached' | 'unknown'
export type EventKind = 'bonus' | 'gain' | 'live' | 'offline' | 'raid' | 'drop' | 'giveaway' | 'info' | 'error'

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
  // Listes où figure la chaîne : page Points de chaîne, page StreamElements
  inPoints: boolean
  inSe: boolean
  // Prioritaire pour les points : giveaway par points de chaîne ouvert
  priority: boolean
  chatPresence: boolean
  inChat: boolean
  seRank: number | null
  seLoyalty: SeLoyalty | null
  seGiveaway: SeGiveaway | null
  // Nos tickets dans le giveaway en cours, lus dans les réponses du bot (null : inconnu)
  seMyTickets: number | null
  // Relevé du solde StreamElements : choisi (null : automatique) et appliqué, en minutes
  seRefresh: number | null
  // Solde de points de chaîne à atteindre (null : pas de limite)
  maxBalance: number | null
  sePollMinutes: number
  // null : pas encore vérifié ; false : la chaîne n'utilise pas StreamElements
  seEnabled: boolean | null
  sePoints: number | null
  seEarnedToday: number
  // Photo de profil (connue après la première relecture du statut live)
  profileImage: string | null
  // Pseudo de sa page StreamElements (son pseudo Twitch, sauf fiche du streamer)
  sePage: string
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
  giveaways: GiveawaysView
  // Langue de diffusion des chaînes (login → code ISO 639-1)
  languages: Record<string, string>
  scanner: ScannerView
  // Chaînes surveillées en live : giveaway StreamElements actif, description, message épinglé
  insights: Record<string, ChannelInsight>
  sync: SyncView
}

export interface SyncView {
  enabled: boolean
  url: string
  status: 'off' | 'syncing' | 'ok' | 'error'
  lastSync: string | null
  error: string | null
  // Collections proposées par le serveur
  serverCollections: string[]
  // Éléments par donnée, ici et sur le serveur (null : serveur trop ancien)
  localCounts: Record<string, number>
  serverCounts: Record<string, number> | null
  // Collections synchronisées (celles du serveur non décochées)
  collections: string[]
  sent: number
  received: number
  // Augmente à chaque modification reçue
  revision: number
  // Flux temps réel du serveur (0.5 et plus) : modifications des autres reçues aussitôt
  live: 'off' | 'connecting' | 'connected' | 'reconnecting' | 'unavailable' | 'refused'
}

export interface ChannelInsight {
  description: string | null
  pinnedMessage: string | null
  streamelements: { title: string, description: string, command: string | null, cost: number } | null
}

export interface ScanMatch {
  login: string
  displayName: string
  title: string
  viewers: number
  startedAt: string | null
  language: string | null
  profileImage: string | null
  gameId: string
  gameName: string
  keywords: string[]
  monitored: boolean
  ignored: boolean
  description: string | null
  pinnedMessage: string | null
  // Type du giveaway en cours s'il est connu (giveaway ouvert de la chaîne, ou StreamElements actif)
  giveawayKind: GiveawayKind | null
  giveawayCommand: string | null
  // Giveaway StreamElements actif : titre et description
  giveawayText: string | null
}

export interface ScannerView {
  matches: ScanMatch[]
  scannedAt: string | null
  scannedStreams: number
  scanning: boolean
  error: string | null
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
  // Mise à jour trouvée installée sans confirmation, vérifiée toutes les updateCheckMinutes (1 à 360)
  autoInstall: boolean
  updateCheckMinutes: number
  notifications: boolean
  autoParticipate: boolean
  autoDelayMin: number
  autoDelayMax: number
  // Comptes qui participent automatiquement (vide : tous)
  autoAccounts: number[]
  scannerKeywords: string[]
  // Mots-clés de la liste décochés
  scannerKeywordsOff: string[]
  // Catégories décochées (identifiants Twitch des jeux)
  scannerGamesOff: string[]
  // Langues retenues par le scanner (vide : toutes)
  scannerLanguages: string[]
  scannerMinViewers: number
  scannerAuto: boolean
  soundAlerts: boolean
  // Volume des alertes, de 0 à 100
  soundVolume: number
  // Alertes avec un son : giveaway, win, mention, reminder, scanner
  soundEvents: string[]
  // Son de chaque alerte : son prédéfini ou « custom » (absent : son par défaut)
  soundChoices: Record<string, string>
  // Nom du fichier choisi pour chaque alerte
  soundFiles: Record<string, string>
  // Détection d'une commande de giveaway
  detectWindow: number
  detectPercent: number
  detectMaxPerUser: number
  detectCooldownSecs: number
  // Compter aussi les mots-clés tapés seuls, sans « ! »
  detectKeywords: boolean
  // Page multi-stream : intégrée à l'application, ou page GitHub (secours)
  // Fenêtre « Streams » : bloqueur de publicités Twitch dans les lecteurs
  viewerAdblock: boolean
  // Processeur, mémoire et trafic de l'application affichés en bas à gauche
  showUsage: boolean
  // Phrases pour dire bonjour dans le chat, par langue (code ISO 639-1)
  greetings: Record<string, string[]>
  // Langue des phrases quand celle de la chaîne est inconnue ou sans phrases
  greetingDefaultLang: string
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
  startsAt: string
  endsAt: string
  rewards: DropReward[]
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
  // campaignId : campagne pour laquelle la chaîne est regardée (null : pas encore connue)
  target: { gameName: string, login: string, viewers: number, title: string, campaignId: string | null } | null
  inventoryAt: string | null
  error: string | null
}

// --- Giveaways ---

export interface GiveawayEvidence {
  count: number
  window: number
  users: string[]
}

export type GiveawayKind = 'command' | 'ticket' | 'streamelements' | 'channelpoints'

// Fiche d'un giveaway au-delà de sa détection ; conditions : null = on ne sait pas
export interface GiveawayDetails {
  ticketUnique: boolean | null
  ticketMax: number | null
  rewardName: string | null
  rewardCost: number | null
  drawAt: string | null
  requireFollow: boolean | null
  requirePresence: boolean | null
  requireDiscord: boolean | null
  notes: string | null
  // Avoir dit bonjour dans le chat
  requireGreeting: boolean | null
  // Présence minimum dans le chat en minutes (0 : non requis, null : on ne sait pas)
  minChatMinutes: number | null
  // Follow depuis au moins tant de jours (avec le follow requis)
  followMinDays: number | null
}

export interface GiveawayInput extends GiveawayDetails {
  channel: string
  kind: GiveawayKind
  command: string | null
  url: string | null
  giftIds: number[]
}

export interface GiveawayView extends GiveawayDetails {
  id: number
  channel: string
  kind: GiveawayKind
  manual: boolean
  giftIds: number[]
  // Lot d'un giveaway gagné
  prizeReceived: boolean
  prizeNote: string | null
  command: string | null
  url: string | null
  detectedAt: string
  // Statut : en pause = live terminé, tirage pas encore fait (reprend au prochain live)
  status: 'open' | 'paused' | 'closed'
  open: boolean
  handled: boolean
  // Premier gagnant (voir winners)
  winner: string | null
  winnerChance: string | null
  drawnAt: string | null
  // Tous les gagnants, dans l'ordre du tirage, et leur chance (pseudo en minuscules → « 20.92% »)
  winners: string[]
  winnerChances: Record<string, string>
  evidence: GiveawayEvidence | null
  // Comptes inscrits ; tickets : giveaway StreamElements (relevés sur StreamElements)
  entriesView: { accountId: number, login: string, at: string, auto: boolean, tickets: number | null }[]
  winnerOurs: string | null
  // Réponse écrite automatiquement si un de nos comptes gagne (présence requise), délai en s
  winReply: string | null
  winReplyDelay: number | null
  // Pseudo de sa page StreamElements s'il diffère de son pseudo Twitch
  seName: string | null
}

export interface MentionView {
  id: number
  at: string
  channel: string
  account: string
  author: string
  message: string
}

export interface GiveawayChannelView {
  id: number
  login: string
  // Qui lit ce chat : « anonyme », le pseudo d'un compte, ou null (pas encore rejoint)
  reader: string | null
  liveChecked: boolean
  // En live (null : hors ligne ou pas encore vérifié)
  live: StreamInfo | null
  inCategory: boolean
  // Photo de profil (connue après la première relecture du statut live)
  profileImage: string | null
}

export interface GiveawayGame {
  id: number
  gameId: string
  name: string
  slug: string
  boxArt: string | null
}

export interface GiveawaysView {
  channels: GiveawayChannelView[]
  games: GiveawayGame[]
  giveaways: GiveawayView[]
  mentions: MentionView[]
  // Réponses des bots aux commandes d'information (!giveaway…), par chaîne
  botReplies: Record<string, BotReply[]>
}

export interface BotReply {
  channel: string
  command: string
  bot: string
  message: string
  askedBy: string
  updatedAt: string
  seenAt: string
}

export interface ParticipationResult {
  accountId: number
  login: string
  ok: boolean
  message: string
}

// --- Catalogue de cadeaux ---

export interface Gift {
  id: number
  title: string
  // Data URL de l'image, réduite à l'ajout
  image: string | null
  groupId: number | null
  // Mots qui, dans le titre d'un live, suggèrent ce cadeau
  keywords: string[]
  // Catégorie Twitch (jeu) du cadeau
  gameId: string | null
  gameName: string | null
  // Cadeau recherché (propre à ce poste, jamais synchronisé)
  favorite: boolean
}

export interface GiftGroup {
  id: number
  name: string
  // Catégorie du groupe (null : sans catégorie)
  gameId: string | null
  gameName: string | null
}

export interface TwitchGame {
  id: string
  name: string
  slug: string | null
  boxArt: string | null
}

export interface GiftCatalog {
  gifts: Gift[]
  groups: GiftGroup[]
  // Nombre de giveaways par cadeau
  usage: Record<string, number>
}
