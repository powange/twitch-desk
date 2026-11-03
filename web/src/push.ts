// Notifications push de ce navigateur : abonnement (Web Push) enregistré sur le serveur avec
// les alertes choisies. Le serveur les envoie même web app fermée.
import { ref } from 'vue'
import { ApiError, deletePush, getPush, savePush, type PushInfo } from './api'

export const ALERTS: { key: string, label: string, description: string }[] = [
  { key: 'giveaway', label: 'Giveaway détecté', description: 'Nouveau giveaway, ou commande trouvée' },
  { key: 'win', label: 'Gain', description: 'Un de tes comptes a gagné' },
  { key: 'reminder', label: 'Rappel de tirage', description: 'Tirage imminent avec présence requise' },
  { key: 'mention', label: 'Mention', description: 'Un de tes pseudos est mentionné dans un chat' },
  { key: 'scanner', label: 'Scanner', description: 'Nouveau stream repéré par le scanner' },
  { key: 'drop', label: 'Drop obtenu', description: 'Un drop a été réclamé pour un de tes comptes' }
]
const DEFAULT_EVENTS = ['giveaway', 'win', 'reminder', 'mention', 'drop']

// « unsupported » : navigateur sans Web Push ; « install » : iPhone ou iPad, la page doit
// d'abord être ajoutée à l'écran d'accueil ; « denied » : notifications refusées dans le navigateur
export type PushSupport = 'ok' | 'unsupported' | 'install' | 'denied'

const isIos = /iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)
const standalone = window.matchMedia('(display-mode: standalone)').matches || (navigator as { standalone?: boolean }).standalone === true

export function support(): PushSupport {
  const api = 'serviceWorker' in navigator && 'PushManager' in window && 'Notification' in window
  if (!api) return isIos && !standalone ? 'install' : 'unsupported'
  if (Notification.permission === 'denied') return 'denied'
  return 'ok'
}

function base64UrlToBytes(text: string): Uint8Array<ArrayBuffer> {
  const b64 = (text + '='.repeat((4 - text.length % 4) % 4)).replace(/-/g, '+').replace(/_/g, '/')
  const raw = atob(b64)
  const bytes = new Uint8Array(raw.length)
  for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i)
  return bytes
}

function sameKey(sub: PushSubscription, publicKey: string): boolean {
  const key = sub.options.applicationServerKey
  if (!key) return false
  const a = new Uint8Array(key)
  const b = base64UrlToBytes(publicKey)
  return a.length === b.length && a.every((v, i) => v === b[i])
}

async function registration(): Promise<ServiceWorkerRegistration> {
  const timeout = new Promise<never>((_, reject) => setTimeout(() => reject(new Error('Service worker indisponible : recharge la page')), 10_000))
  return await Promise.race([navigator.serviceWorker.ready, timeout])
}

async function currentSubscription(): Promise<PushSubscription | null> {
  if (support() === 'unsupported' || support() === 'install') return null
  return await (await registration()).pushManager.getSubscription()
}

// État partagé : informations du serveur, notifications actives sur ce navigateur
export const info = ref<PushInfo | null>(null)
export const active = ref(false)
// Serveur sans notifications (avant la 0.7)
export const serverTooOld = ref(false)

// Relit l'abonnement ; abonnement du navigateur présent mais inconnu du serveur (serveur
// réinstallé, abonnement renouvelé) : réenregistré avec les alertes choisies
export async function refresh(token: string) {
  try {
    info.value = await getPush(token)
    serverTooOld.value = false
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) serverTooOld.value = true
    return
  }
  const sub = await currentSubscription().catch(() => null)
  if (!sub || Notification.permission !== 'granted' || !sameKey(sub, info.value.publicKey)) {
    active.value = false
    return
  }
  if (info.value.endpoint !== sub.endpoint) {
    const events = info.value.events.length ? info.value.events : DEFAULT_EVENTS
    info.value = await savePush(token, sub.toJSON(), events).catch(() => info.value)
  }
  active.value = info.value?.endpoint === sub.endpoint
}

// Active les notifications (demande d'autorisation : à appeler depuis un clic)
export async function enable(token: string, events?: string[]) {
  const permission = await Notification.requestPermission()
  if (permission !== 'granted') {
    throw new Error(permission === 'denied'
      ? 'Notifications refusées : autorise-les dans les réglages du navigateur pour ce site'
      : 'Autorisation non donnée')
  }
  const server = info.value ?? await getPush(token)
  const reg = await registration()
  let sub = await reg.pushManager.getSubscription()
  // Abonnement fait avec une autre clé (autre serveur) : refait
  if (sub && !sameKey(sub, server.publicKey)) {
    await sub.unsubscribe().catch(() => {})
    sub = null
  }
  try {
    sub ??= await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: base64UrlToBytes(server.publicKey) })
  } catch {
    throw new Error('Le navigateur a refusé l\'abonnement aux notifications (navigation privée, ou notifications bloquées pour ce site)')
  }
  const chosen = events ?? (server.events.length ? server.events : DEFAULT_EVENTS)
  info.value = await savePush(token, sub.toJSON(), chosen)
  active.value = true
}

// Alertes choisies changées (notifications déjà actives)
export async function choose(token: string, events: string[]) {
  const sub = await currentSubscription()
  if (!sub) throw new Error('Notifications inactives sur ce téléphone')
  info.value = await savePush(token, sub.toJSON(), events)
}

// Plus de notifications sur ce téléphone ; `token` absent : appareil déjà oublié par le serveur
export async function disable(token: string | null) {
  const sub = await currentSubscription().catch(() => null)
  await sub?.unsubscribe().catch(() => {})
  active.value = false
  if (token) {
    await deletePush(token)
    if (info.value) info.value = { ...info.value, endpoint: null }
  }
}
