// Accès au serveur de synchronisation avec le jeton de cet appareil (lecture seule)

const API = '/api/v1'
const STORAGE_KEY = 'twitch-desk:device'

export interface Device {
  token: string
  deviceId: string
  instanceName: string
}

export class ApiError extends Error {
  constructor(message: string, public status: number) {
    super(message)
  }
}

export function loadDevice(): Device | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    const d = raw ? JSON.parse(raw) as Device : null
    return d && typeof d.token === 'string' ? d : null
  } catch {
    return null
  }
}

export function saveDevice(d: Device | null) {
  try {
    if (d) localStorage.setItem(STORAGE_KEY, JSON.stringify(d))
    else localStorage.removeItem(STORAGE_KEY)
  } catch { /* stockage indisponible (navigation privée) */ }
}

async function errorOf(res: Response): Promise<ApiError> {
  let message = ''
  try {
    message = ((await res.json()) as { error?: string }).error ?? ''
  } catch { /* corps illisible */ }
  if (!message) {
    message = res.status === 404
      ? 'Serveur introuvable ou trop ancien (version 0.6 ou plus nécessaire)'
      : `Erreur du serveur (${res.status})`
  }
  return new ApiError(message, res.status)
}

async function call<T>(path: string, init: RequestInit = {}): Promise<T> {
  let res: Response
  try {
    res = await fetch(`${API}${path}`, { ...init, cache: 'no-store' })
  } catch {
    throw new ApiError('Serveur injoignable : vérifie ta connexion', 0)
  }
  if (!res.ok) throw await errorOf(res)
  return await res.json() as T
}

export function authHeader(token: string) {
  return { Authorization: `Device ${token}` }
}

// Code à 6 chiffres affiché par l'application du PC → jeton de cet appareil
export async function claim(code: string, deviceName: string): Promise<Device> {
  const r = await call<{ token: string, deviceId: string, instance: { name: string } }>('/pair/claim', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ code, deviceName })
  })
  return { token: r.token, deviceId: r.deviceId, instanceName: r.instance.name }
}

export function overview<T>(token: string): Promise<T> {
  return call<T>('/app/overview', { headers: authHeader(token) })
}

// État publié par le PC seul (léger, serveur 0.8 et plus)
export function state<T>(token: string): Promise<{ state: T | null, stateAt: string | null }> {
  return call('/app/state', { headers: authHeader(token) })
}

// Demande d'achat de tickets StreamElements, exécutée par le PC (serveur 0.8 et plus)
function requestAction<T>(token: string, body: Record<string, unknown>): Promise<T> {
  return call<T>('/app/actions', {
    method: 'POST',
    headers: { ...authHeader(token), 'Content-Type': 'application/json' },
    body: JSON.stringify(body)
  })
}

export function buyTickets<T>(token: string, body: { account: string, channel: string, giveaway: string, tickets: number }): Promise<T> {
  return requestAction<T>(token, { kind: 'se_buy', ...body })
}

// Mise à jour de l'application du PC : vérifier, ou installer puis redémarrer (serveur 0.8.2 et plus)
export function updatePc<T>(token: string, kind: 'update_check' | 'update_install'): Promise<T> {
  return requestAction<T>(token, { kind })
}

// Giveaway (serveur 0.8.10 et plus, exécuté par le PC) : masquer, journal du chat, conditions
export function hideGiveaway<T>(token: string, item: string, hidden: boolean): Promise<T> {
  return requestAction<T>(token, { kind: 'giveaway_hide', item, hidden })
}
export function giveawayChatLog<T>(token: string, item: string): Promise<T> {
  return requestAction<T>(token, { kind: 'giveaway_chat_log', item })
}
export function giveawayConditions<T>(token: string, item: string, conditions: Record<string, unknown>): Promise<T> {
  return requestAction<T>(token, { kind: 'giveaway_conditions', item, conditions })
}
// Demande quelconque au PC (vérifiée par le serveur selon son type)
export function pcAction<T>(token: string, body: Record<string, unknown>): Promise<T> {
  return requestAction<T>(token, body)
}

// Ordre des chaînes de la page Points de chaîne d'un compte (pseudos)
export function pointsOrder<T>(token: string, account: string, order: string[]): Promise<T> {
  return requestAction<T>(token, { kind: 'points_order', account, order })
}
// Présence dans le chat ou achat automatique d'une chaîne StreamElements
export function seOption<T>(token: string, account: string, channel: string, option: 'chat' | 'autoBuy', enabled: boolean): Promise<T> {
  return requestAction<T>(token, { kind: 'se_option', account, channel, option, enabled })
}
// Chaîne du scanner ajoutée aux chaînes surveillées du PC
export function monitorChannel<T>(token: string, channel: string): Promise<T> {
  return requestAction<T>(token, { kind: 'channel_monitor', channel })
}

export function actions<T>(token: string): Promise<T[]> {
  return call<T[]>('/app/actions', { headers: authHeader(token) })
}

export type LiveStatus = 'connecting' | 'live' | 'reconnecting'

interface StreamHandlers {
  // Événement reçu (« seq » : documents modifiés ; « state » : état du PC publié)
  onEvent: (type: string) => void
  onStatus: (status: LiveStatus) => void
  // Jeton refusé (appareil révoqué)
  onUnauthorized: () => void
}

// Rien reçu pendant ce délai (le serveur envoie un commentaire toutes les 20 s) : reconnexion
const IDLE_MS = 60_000
const RETRY_MIN = 2_000
const RETRY_MAX = 60_000

// Flux temps réel lu avec fetch et non EventSource : EventSource ne peut pas envoyer
// l'en-tête Authorization, et un jeton dans l'adresse finirait dans les journaux du proxy.
// Rend une fonction qui ferme le flux ; `restart` le rouvre tout de suite (retour au premier plan)
export function openEvents(token: string, h: StreamHandlers): { close: () => void, restart: () => void } {
  let stopped = false
  let controller: AbortController | null = null
  let wake: (() => void) | null = null
  let retry = RETRY_MIN

  const sleep = (ms: number) => new Promise<void>((resolve) => {
    const t = setTimeout(resolve, ms)
    wake = () => { clearTimeout(t); resolve() }
  })

  async function once(): Promise<boolean> {
    controller = new AbortController()
    let idle: ReturnType<typeof setTimeout> | null = null
    const ctrl = controller
    const resetIdle = () => {
      if (idle) clearTimeout(idle)
      idle = setTimeout(() => ctrl.abort(), IDLE_MS)
    }
    try {
      resetIdle()
      const res = await fetch(`${API}/events`, {
        headers: { ...authHeader(token), Accept: 'text/event-stream' },
        signal: ctrl.signal,
        cache: 'no-store'
      })
      if (res.status === 401) {
        stopped = true
        h.onUnauthorized()
        return false
      }
      if (!res.ok || !res.body) return false
      h.onStatus('live')
      retry = RETRY_MIN
      const reader = res.body.pipeThrough(new TextDecoderStream()).getReader()
      let buffer = ''
      for (;;) {
        const { value, done } = await reader.read()
        if (done) break
        resetIdle()
        buffer += value
        let end: number
        // Événements séparés par une ligne vide
        while ((end = buffer.search(/\r?\n\r?\n/)) >= 0) {
          const block = buffer.slice(0, end)
          buffer = buffer.slice(end).replace(/^\r?\n\r?\n/, '')
          let type = 'message'
          let hasData = false
          for (const line of block.split(/\r?\n/)) {
            if (line.startsWith(':')) continue
            if (line.startsWith('event:')) type = line.slice(6).trim()
            else if (line.startsWith('data:')) hasData = true
          }
          if (hasData) h.onEvent(type)
        }
      }
      return true
    } catch {
      return false
    } finally {
      if (idle) clearTimeout(idle)
    }
  }

  void (async () => {
    h.onStatus('connecting')
    while (!stopped) {
      await once()
      if (stopped) break
      h.onStatus('reconnecting')
      await sleep(retry)
      retry = Math.min(retry * 2, RETRY_MAX)
    }
  })()

  return {
    close() {
      stopped = true
      controller?.abort()
      wake?.()
    },
    restart() {
      retry = RETRY_MIN
      controller?.abort()
      wake?.()
    }
  }
}

// --- Notifications push (serveur 0.7 et plus) ---

export interface PushInfo {
  // Clé publique VAPID du serveur (applicationServerKey)
  publicKey: string
  // Abonnement enregistré pour cet appareil (null : aucun)
  endpoint: string | null
  // Alertes choisies
  events: string[]
  // Alertes possibles
  alerts: string[]
}

export function getPush(token: string): Promise<PushInfo> {
  return call<PushInfo>('/app/push', { headers: authHeader(token) })
}

export function savePush(token: string, subscription: PushSubscriptionJSON, events: string[]): Promise<PushInfo> {
  return call<PushInfo>('/app/push', {
    method: 'PUT',
    headers: { ...authHeader(token), 'Content-Type': 'application/json' },
    body: JSON.stringify({ endpoint: subscription.endpoint, keys: subscription.keys, events })
  })
}

export async function deletePush(token: string): Promise<void> {
  let res: Response
  try {
    res = await fetch(`${API}/app/push`, { method: 'DELETE', headers: authHeader(token), cache: 'no-store' })
  } catch {
    throw new ApiError('Serveur injoignable : vérifie ta connexion', 0)
  }
  if (!res.ok) throw await errorOf(res)
}

// Images des lots (serveur 0.8.6 et plus) : lues une fois par adresse (qui change avec l'image),
// avec le cache du navigateur entre deux ouvertures
const images = new Map<string, Promise<string | null>>()
export function giftImage(path: string): Promise<string | null> {
  let found = images.get(path)
  if (!found) {
    const d = loadDevice()
    found = d
      ? fetch(path, { headers: authHeader(d.token) })
          .then(r => r.ok ? r.blob() : null)
          .then(b => b ? URL.createObjectURL(b) : null)
          .catch(() => null)
      : Promise.resolve(null)
    found.then((u) => { if (!u) images.delete(path) })
    images.set(path, found)
  }
  return found
}
