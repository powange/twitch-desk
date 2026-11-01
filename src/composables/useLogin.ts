import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useSnapshot } from './useSnapshot'
import type { AccountView, DeviceCode, LoginEvent, SeLoginEvent } from '../types'

// Connexion d'un compte Twitch (code à saisir sur twitch.tv/activate) et de StreamElements.
// État gardé hors de la page Comptes : le code, le compte à rebours et les messages de
// réussite ou d'expiration survivent à un changement de page.
const code = ref<DeviceCode | null>(null)
const starting = ref(false)
const remaining = ref(0)
// Comptes dont la fenêtre de connexion StreamElements est ouverte
const seConnecting = ref<Set<number>>(new Set())
let countdown: ReturnType<typeof setInterval> | null = null
let toast: ReturnType<typeof useToast> | null = null
let installed = false

function stopCountdown() {
  if (countdown) clearInterval(countdown)
  countdown = null
}

function seDone(accountId: number) {
  const next = new Set(seConnecting.value)
  next.delete(accountId)
  seConnecting.value = next
}

function seLoginToast(ev: SeLoginEvent) {
  const { accounts } = useSnapshot()
  const login = accounts.value.find(a => a.id === ev.accountId)?.login ?? ''
  if (ev.status === 'done') toast?.add({ title: `StreamElements connecté pour ${login}`, color: 'success', icon: 'i-lucide-check' })
  if (ev.status === 'wrongAccount') toast?.add({ title: 'Mauvais compte', description: `Tu t'es connecté à StreamElements avec ${ev.username}, pas avec ${login}. Recommence avec le bon compte Twitch.`, color: 'warning' })
  if (ev.status === 'error') toast?.add({ title: 'Connexion StreamElements échouée', description: ev.message, color: 'error' })
}

// Écoute des événements de connexion, une seule fois pour toute l'application (App.vue)
export function installLogin() {
  toast ??= useToast()
  if (installed) return
  installed = true
  void listen<SeLoginEvent>('se-login', (e) => {
    seDone(e.payload.accountId)
    seLoginToast(e.payload)
  })
  void listen<LoginEvent>('login', (e) => {
    code.value = null
    stopCountdown()
    const ev = e.payload
    if (ev.status === 'done') toast?.add({ title: `${ev.login} connecté`, color: 'success', icon: 'i-lucide-check' })
    if (ev.status === 'expired') toast?.add({ title: 'Le code a expiré', description: 'Relance la connexion.', color: 'warning' })
    if (ev.status === 'denied') toast?.add({ title: 'Connexion refusée sur Twitch', color: 'warning' })
    if (ev.status === 'error') toast?.add({ title: 'Connexion échouée', description: ev.message, color: 'error' })
  })
}

async function connect() {
  starting.value = true
  try {
    code.value = await invoke<DeviceCode>('start_login')
    remaining.value = code.value.expires_in
    stopCountdown()
    countdown = setInterval(() => {
      remaining.value = Math.max(0, remaining.value - 1)
    }, 1000)
  } catch (e) {
    toast?.add({ title: 'Connexion impossible', description: String(e), color: 'error' })
  } finally {
    starting.value = false
  }
}

// Annulation : le code n'est retiré que si le cœur l'a bien abandonné
async function cancel() {
  await invoke('cancel_login')
  code.value = null
  stopCountdown()
}

async function connectSe(account: AccountView) {
  try {
    await invoke('connect_streamelements', { accountId: account.id })
    seConnecting.value = new Set(seConnecting.value).add(account.id)
  } catch (e) {
    toast?.add({ title: 'Connexion StreamElements impossible', description: String(e), color: 'error' })
  }
}

export function useLogin() {
  toast ??= useToast()
  return { code, starting, remaining, seConnecting, connect, cancel, connectSe, seLoginToast }
}
