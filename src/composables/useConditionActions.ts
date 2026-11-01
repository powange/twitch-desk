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
}

interface FollowState { accountId: number, login: string, follows: boolean | null, followedAt: string | null }
interface GreetingState { accountId: number, login: string, greeted: boolean }

// État et actions des conditions (follow, bonjour, Discord), relus à l'ouverture de la page
// pour les chaînes de `targets` qui les demandent
export function useConditionActions(targets: () => ConditionTarget[]) {
  const toast = useToast()
  const openExternal = useOpenExternal()

  // --- Follow requis : nos comptes suivent-ils la chaîne ? ---
  const follows = reactive<Record<string, FollowState[] | 'loading'>>({})
  const following = ref<string | null>(null)
  async function loadFollows(channel: string) {
    follows[channel] = 'loading'
    try {
      follows[channel] = await invoke<FollowState[]>('follow_status', { channel })
    } catch {
      delete follows[channel]
    }
  }
  watch(() => targets().filter(t => t.requireFollow === true).map(t => t.channel), (channels) => {
    for (const c of new Set(channels)) if (!(c in follows)) void loadFollows(c)
  }, { immediate: true })
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
  async function followChannel(channel: string) {
    const ids = notFollowing(channel).map(f => f.accountId)
    if (!ids.length || following.value) return
    following.value = channel
    try {
      follows[channel] = await invoke<FollowState[]>('follow_channel', { channel, accountIds: ids })
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
  const greetings = reactive<Record<string, GreetingState[]>>({})
  const greeting = ref<string | null>(null)
  async function loadGreetings(channel: string) {
    try {
      greetings[channel] = await invoke<GreetingState[]>('greeting_status', { channel })
    } catch {
      delete greetings[channel]
    }
  }
  watch(() => targets().filter(t => t.requireGreeting === true).map(t => t.channel), (channels) => {
    for (const c of new Set(channels)) if (!(c in greetings)) void loadGreetings(c)
  }, { immediate: true })
  const greetedByAll = (channel: string) => !!greetings[channel]?.length && greetings[channel]!.every(s => s.greeted)
  function greetingTooltip(channel: string) {
    const list = greetings[channel]
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
    following, notFollowing, tooRecent, followOk, followTooltip, followChannel,
    openingDiscord, openDiscord,
    greeting, greetedByAll, greetingTooltip, sayHello
  })
}
export type ConditionActions = ReturnType<typeof useConditionActions>
