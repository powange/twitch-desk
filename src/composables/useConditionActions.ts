import { reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useOpenExternal } from '../utils/external'

// Conditions d'un giveaway (ou conditions habituelles d'un streamer) ; null : on ne sait pas
export interface ConditionTarget {
  channel: string
  requirePresence: boolean | null
  requireDiscord: boolean | null
  requireFollow: boolean | null
  followMinDays: number | null
  requireGreeting: boolean | null
  minChatMinutes: number | null
  // Réponse automatique au gain (giveaways avec présence requise), et son délai en secondes
  winReply?: string | null
  winReplyDelay?: number | null
}

interface FollowState { accountId: number, login: string, follows: boolean | null, followedAt: string | null }
interface GreetingState { accountId: number, login: string, greeted: boolean }

// États relus au plus toutes les 10 minutes, partagés par les pages (une réponse en erreur est
// retentée après ce délai, sans relire en boucle)
const TTL = 10 * 60_000
type Cached<T> = T | 'loading' | 'failed'
const follows = reactive<Record<string, Cached<FollowState[]>>>({})
const greetings = reactive<Record<string, Cached<GreetingState[]>>>({})
// Heure du dernier chargement (réussi ou non) de chaque chaîne
const followsAt = new Map<string, number>()
const greetingsAt = new Map<string, number>()

function needsLoad(cache: Record<string, Cached<unknown>>, at: Map<string, number>, channel: string) {
  const v = cache[channel]
  if (v === undefined) return true
  if (v === 'loading') return false
  return Date.now() - (at.get(channel) ?? 0) > TTL
}

async function loadFollows(channel: string) {
  follows[channel] = 'loading'
  try {
    follows[channel] = await invoke<FollowState[]>('follow_status', { channel })
  } catch {
    follows[channel] = 'failed'
  } finally {
    followsAt.set(channel, Date.now())
  }
}

async function loadGreetings(channel: string) {
  greetings[channel] = 'loading'
  try {
    greetings[channel] = await invoke<GreetingState[]>('greeting_status', { channel })
  } catch {
    greetings[channel] = 'failed'
  } finally {
    greetingsAt.set(channel, Date.now())
  }
}

// État et actions des conditions (follow, bonjour, Discord) pour les chaînes de `targets` qui
// les demandent (giveaways ouverts, chaînes surveillées)
export function useConditionActions(targets: () => ConditionTarget[]) {
  const toast = useToast()
  const openExternal = useOpenExternal()

  const followChannels = () => [...new Set(targets().filter(t => t.requireFollow === true).map(t => t.channel))]
  const greetingChannels = () => [...new Set(targets().filter(t => t.requireGreeting === true).map(t => t.channel))]
  // Relu à chaque mise à jour des cibles : seulement les chaînes inconnues ou dont l'état a expiré
  watch(followChannels, (channels) => {
    for (const c of channels) if (needsLoad(follows, followsAt, c)) void loadFollows(c)
  }, { immediate: true })
  watch(greetingChannels, (channels) => {
    for (const c of channels) if (needsLoad(greetings, greetingsAt, c)) void loadGreetings(c)
  }, { immediate: true })
  // Relecture forcée des états des chaînes affichées
  function refresh() {
    for (const c of followChannels()) if (follows[c] !== 'loading') void loadFollows(c)
    for (const c of greetingChannels()) if (greetings[c] !== 'loading') void loadGreetings(c)
  }

  // --- Follow requis : nos comptes suivent-ils la chaîne ? ---
  const following = ref<string | null>(null)
  function followList(channel: string): FollowState[] | null {
    const f = follows[channel]
    return Array.isArray(f) ? f : null
  }
  const notFollowing = (channel: string) => (followList(channel) ?? []).filter(f => f.follows === false)
  // Jours depuis le follow d'un compte
  const followDays = (f: FollowState) => f.followedAt ? Math.floor((Date.now() - new Date(f.followedAt).getTime()) / 86_400_000) : null
  // Comptes qui suivent, mais depuis moins longtemps que demandé
  function tooRecent(t: Pick<ConditionTarget, 'channel' | 'followMinDays'>) {
    const min = t.followMinDays
    if (!min) return []
    return (followList(t.channel) ?? []).filter(f => f.follows && (followDays(f) ?? 0) < min)
  }
  const followOk = (t: Pick<ConditionTarget, 'channel' | 'followMinDays'>) => {
    const list = followList(t.channel)
    return !!list && list.every(f => f.follows) && tooRecent(t).length === 0
  }
  function followTooltip(t: Pick<ConditionTarget, 'channel' | 'followMinDays'>) {
    const list = followList(t.channel)
    const head = t.followMinDays ? `Follow requis depuis au moins ${t.followMinDays} jour(s)` : 'Follow requis'
    if (!list) return follows[t.channel] === 'loading' ? `${head} : vérification…` : `${head} : état du follow inconnu`
    const missing = list.filter(f => f.follows === false).map(f => f.login)
    const unknown = list.filter(f => f.follows === null).map(f => f.login)
    const recent = tooRecent(t).map(f => `${f.login} (${followDays(f)} j)`)
    const parts = [head]
    if (missing.length) parts.push(`pas suivie par ${missing.join(', ')} : clique pour suivre`)
    if (recent.length) parts.push(`follow trop récent : ${recent.join(', ')}`)
    if (!missing.length && !recent.length && !unknown.length) parts.push('suivie par tous tes comptes')
    if (unknown.length) parts.push(`état inconnu pour ${unknown.join(', ')}`)
    return parts.join(' · ')
  }
  // État du cœur en texte, pour les lecteurs d'écran
  function followLabel(t: Pick<ConditionTarget, 'channel' | 'followMinDays'>) {
    const list = followList(t.channel)
    if (!list) return follows[t.channel] === 'loading' ? 'Follow requis : vérification en cours' : 'Follow requis : état inconnu'
    const missing = notFollowing(t.channel).map(f => f.login)
    if (missing.length) return `Follow requis : pas suivie par ${missing.join(', ')}`
    if (tooRecent(t).length) return 'Follow requis : follow trop récent'
    if (followOk(t)) return 'Follow requis : suivie par tous tes comptes'
    return 'Follow requis : état inconnu pour certains comptes'
  }
  async function followChannel(channel: string) {
    const ids = notFollowing(channel).map(f => f.accountId)
    if (!ids.length || following.value) return
    following.value = channel
    try {
      follows[channel] = await invoke<FollowState[]>('follow_channel', { channel, accountIds: ids })
      followsAt.set(channel, Date.now())
      toast.add({ title: `${channel} suivie`, color: 'success', icon: 'i-lucide-check' })
    } catch (e) {
      toast.add({ title: 'Follow impossible', description: String(e), color: 'error' })
      void loadFollows(channel)
    } finally {
      following.value = null
    }
  }

  // --- Discord requis : invitation de la fiche du streamer, sinon trouvée sur sa chaîne Twitch ---
  const openingDiscord = ref<string | null>(null)
  async function openDiscord(channel: string) {
    if (openingDiscord.value) return
    openingDiscord.value = channel
    try {
      const url = await invoke<string | null>('streamer_discord', { login: channel })
      if (url) openExternal(url)
      else toast.add({ title: 'Aucune invitation Discord connue', description: `Ajoute-la sur la fiche de ${channel}.`, color: 'warning' })
    } catch (e) {
      toast.add({ title: 'Invitation Discord introuvable', description: String(e), color: 'error' })
    } finally {
      openingDiscord.value = null
    }
  }

  // --- Avoir dit bonjour : comptes qui l'ont déjà dit sur la chaîne (24 h), bonjour en un clic ---
  const greeting = ref<string | null>(null)
  function greetingList(channel: string): GreetingState[] | null {
    const g = greetings[channel]
    return Array.isArray(g) ? g : null
  }
  const greetedByAll = (channel: string) => {
    const list = greetingList(channel)
    return !!list?.length && list.every(s => s.greeted)
  }
  function greetingTooltip(channel: string) {
    const list = greetingList(channel)
    if (!list) return 'Avoir dit bonjour dans le chat'
    const missing = list.filter(s => !s.greeted).map(s => s.login)
    return missing.length
      ? `Avoir dit bonjour : pas encore dit par ${missing.join(', ')}. Clique pour dire bonjour (phrase au hasard dans la langue de la chaîne)`
      : 'Bonjour déjà dit par tous tes comptes'
  }
  async function sayHello(channel: string) {
    if (greeting.value) return
    greeting.value = channel
    try {
      greetings[channel] = await invoke<GreetingState[]>('say_hello', { channel })
      greetingsAt.set(channel, Date.now())
      toast.add({ title: `Bonjour dit sur ${channel}`, color: 'success', icon: 'i-lucide-hand' })
    } catch (e) {
      toast.add({ title: 'Bonjour non envoyé', description: String(e), color: 'error' })
      void loadGreetings(channel)
    } finally {
      greeting.value = null
    }
  }

  // reactive : les refs sont déballées dans les gabarits (cond.following…)
  return reactive({
    following, notFollowing, tooRecent, followOk, followTooltip, followLabel, followChannel,
    openingDiscord, openDiscord,
    greeting, greetedByAll, greetingTooltip, sayHello,
    refresh
  })
}
export type ConditionActions = ReturnType<typeof useConditionActions>
