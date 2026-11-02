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
