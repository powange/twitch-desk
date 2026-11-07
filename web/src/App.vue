<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import type { DropdownMenuItem } from '@nuxt/ui'
import { ApiError, actions as listActions, loadDevice, openEvents, overview, saveDevice, state as loadStateOnly, type Device, type LiveStatus } from './api'
import type { InstanceState, Overview, PhoneAction } from './types'
import { timeAgo } from './format'
import PairScreen from './components/PairScreen.vue'
import GiveawayCarousel from './components/GiveawayCarousel.vue'
import PointsView from './components/PointsView.vue'
import SeView from './components/SeView.vue'
import DropsView from './components/DropsView.vue'
import ScannerView from './components/ScannerView.vue'
import MentionList from './components/MentionList.vue'
import NotificationsModal from './components/NotificationsModal.vue'
import UpdateModal from './components/UpdateModal.vue'
import ChatLogModal from './components/ChatLogModal.vue'
import { providePhone, sendAction } from './phone'
import type { ChatLogEntry } from './types'
import { active as pushActive, disable as disablePush, refresh as refreshPush, serverTooOld, support as pushSupport } from './push'

// Lien d'association scanné alors que l'appareil est déjà associé : nouvelle association
const device = ref<Device | null>(/pair=\d{6}/.test(location.hash) ? null : loadDevice())
const notice = ref<string | null>(null)
const data = ref<Overview | null>(null)
const error = ref<string | null>(null)
// Serveur injoignable au lancement : dernière vue gardée sur ce téléphone, avec sa date
const LAST_VIEW = 'twitch-desk:last-view'
const staleAt = ref<string | null>(null)
function keepView(v: Overview) {
  try { localStorage.setItem(LAST_VIEW, JSON.stringify({ at: new Date().toISOString(), data: v })) } catch { /* trop gros ou stockage indisponible */ }
}
function lastView(): { at: string, data: Overview } | null {
  try {
    const raw = localStorage.getItem(LAST_VIEW)
    return raw ? JSON.parse(raw) as { at: string, data: Overview } : null
  } catch {
    return null
  }
}
const loading = ref(false)
const status = ref<LiveStatus>('connecting')
let events: ReturnType<typeof openEvents> | null = null
let refreshTimer: ReturnType<typeof setTimeout> | null = null

// Onglet et comptes affichés, retenus sur ce téléphone
type Tab = 'giveaways' | 'scanner' | 'points' | 'se' | 'drops'
function remembered(key: string, fallback: string) {
  try { return localStorage.getItem(`twitch-desk:${key}`) ?? fallback } catch { return fallback }
}
function remember(key: string, value: string) {
  try { localStorage.setItem(`twitch-desk:${key}`, value) } catch { /* stockage indisponible */ }
}
const TAB_KEYS = ['giveaways', 'scanner', 'points', 'se', 'drops'] as const
const asTab = (t: unknown) => TAB_KEYS.find(k => k === t)
// Ouverte depuis une notification (« ?tab=drops ») : sur l'onglet de l'alerte
const query = new URLSearchParams(location.search)
const asked = asTab(query.get('tab'))
// « ?focus=mentions » : section Mentions de l'accueil montrée une fois les données lues
const focus = ref<string | null>(query.get('focus'))
// « ?target=… » : cible de la notification (chaîne du giveaway, du gain, de la mention ou du
// stream repéré ; compte du drop), ouverte une fois les données lues
const target = ref<string | null>(query.get('target'))
if (asked) history.replaceState(null, '', location.pathname + location.hash)
const tab = ref<Tab>(asked ?? asTab(remembered('tab', 'giveaways')) ?? 'giveaways')
const pointsAccount = ref(remembered('points-account', ''))
const seAccount = ref(remembered('se-account', ''))
const dropsAccount = ref(remembered('drops-account', ''))
watch(tab, t => remember('tab', t))
watch(pointsAccount, a => remember('points-account', a))
watch(seAccount, a => remember('se-account', a))
watch(dropsAccount, a => remember('drops-account', a))
// Notification touchée, web app déjà ouverte : le service worker indique l'onglet
navigator.serviceWorker?.addEventListener('message', (e: MessageEvent) => {
  const msg = e.data as { tab?: unknown, focus?: unknown, target?: unknown } | null
  const t = asTab(msg?.tab)
  if (t) tab.value = t
  if (typeof msg?.focus === 'string') focus.value = msg.focus
  if (typeof msg?.target === 'string') target.value = msg.target
})
watch([focus, () => data.value, tab], async ([f]) => {
  if (!f || !data.value) return
  await nextTick()
  const el = document.getElementById(f)
  if (!el) return
  el.scrollIntoView({ behavior: 'smooth', block: 'start' })
  focus.value = null
})
// `title` : nom complet dans l'en-tête quand le libellé de l'onglet est abrégé
const TABS: { key: Tab, label: string, icon: string, title?: string }[] = [
  { key: 'giveaways', label: 'Giveaways', icon: 'i-lucide-gift' },
  { key: 'scanner', label: 'Scanner', icon: 'i-lucide-radar' },
  { key: 'points', label: 'Points', icon: 'i-lucide-coins' },
  { key: 'se', label: 'SE', icon: 'i-lucide-ticket', title: 'StreamElements' },
  { key: 'drops', label: 'Drops', icon: 'i-lucide-package-open' }
]

// Un seul carrousel : giveaways en cours, masqués sur le PC, ou en pause (live terminé, tirage
// pas encore fait), au choix (gardé sur ce téléphone)
type GiveawayList = 'open' | 'hidden' | 'paused'
const savedList = localStorage.getItem('giveawayList')
const listChoice = ref<GiveawayList>(savedList === 'hidden' || savedList === 'paused' ? savedList : 'open')
watch(listChoice, v => localStorage.setItem('giveawayList', v))
const isHidden = (id: string) => !!data.value?.state?.giveaways[id]?.hidden
const all = computed(() => data.value?.giveaways ?? [])
const hiddenList = computed(() => all.value.filter(g => isHidden(g.id)))
const pausedList = computed(() => all.value.filter(g => g.status === 'paused' && !isHidden(g.id)))
const openList = computed(() => all.value.filter(g => g.status === 'open' && !isHidden(g.id)))
const mode = computed<GiveawayList>(() => listChoice.value)
const listItems = computed(() => [
  { label: `Giveaways en cours (${openList.value.length})`, value: 'open' },
  { label: `Giveaways masqués (${hiddenList.value.length})`, value: 'hidden' },
  { label: `Giveaways en pause (${pausedList.value.length})`, value: 'paused' }
])
const shown = computed(() => mode.value === 'hidden' ? hiddenList.value : mode.value === 'paused' ? pausedList.value : openList.value)
const shownTitle = computed(() => mode.value === 'hidden' ? 'Masqués' : mode.value === 'paused' ? 'En pause' : 'En cours')
// Giveaway affiché ; revenu au premier quand la liste change
const shownIndex = ref(0)
watch(mode, () => { shownIndex.value = 0 })

// Cible d'une notification touchée : son giveaway (dans la bonne liste), son stream repéré, la
// mention de sa chaîne ou le compte du drop ; une fois, dès que les données sont là
const highlight = ref<string | null>(null)
let highlightTimer: ReturnType<typeof setTimeout> | null = null
watch([target, () => data.value, tab], async ([t]) => {
  if (!t || !data.value) return
  const login = t.toLowerCase()
  target.value = null
  if (tab.value === 'drops') {
    const acc = data.value.state?.drops?.find(a => a.login.toLowerCase() === login)
    if (acc) dropsAccount.value = acc.login
    return
  }
  if (tab.value === 'giveaways' && focus.value !== 'mentions') {
    const all = data.value.giveaways.filter(g => g.channel.toLowerCase() === login)
    const g = all.find(x => x.status === 'open' && !isHidden(x.id)) ?? all[0]
    if (!g) return
    listChoice.value = isHidden(g.id) ? 'hidden' : g.status === 'paused' ? 'paused' : 'open'
    await nextTick()
    const i = shown.value.findIndex(x => x.id === g.id)
    if (i >= 0) shownIndex.value = i
    return
  }
  // Scanner, mentions : élément de la chaîne mis en évidence quelques secondes
  highlight.value = login
  if (highlightTimer) clearTimeout(highlightTimer)
  highlightTimer = setTimeout(() => { highlight.value = null }, 6000)
  await nextTick()
  document.getElementById(`${tab.value === 'scanner' ? 'scan' : 'mention'}-${login}`)?.scrollIntoView({ behavior: 'smooth', block: 'center' })
}, { immediate: true })
const instanceName = computed(() => data.value?.instance.name || device.value?.instanceName || 'Twitch Desk')
const connection = computed(() => {
  switch (status.value) {
    case 'live': return { label: 'En direct', color: 'success' as const, icon: 'i-lucide-radio' }
    case 'reconnecting': return { label: 'Reconnexion…', color: 'warning' as const, icon: 'i-lucide-refresh-cw' }
    default: return { label: 'Connexion…', color: 'neutral' as const, icon: 'i-lucide-loader' }
  }
})

function forget(message: string | null) {
  // Plus de notifications pour un appareil oublié
  void disablePush(null)
  events?.close()
  events = null
  saveDevice(null)
  device.value = null
  data.value = null
  staleAt.value = null
  try { localStorage.removeItem(LAST_VIEW) } catch { /* rien */ }
  notice.value = message
}

// Lectures numérotées : une réponse lente arrivée après une plus récente est ignorée (elle
// réafficherait un giveaway déjà fermé)
let loadSeq = 0
let stateSeq = 0
async function load() {
  const d = device.value
  if (!d) return
  const seq = ++loadSeq
  // L'état complet relu remplace aussi un état seul en cours de lecture
  stateSeq++
  loading.value = true
  try {
    const fresh = await overview<Overview>(d.token)
    if (seq !== loadSeq) return
    data.value = fresh
    staleAt.value = null
    keepView(fresh)
    error.value = null
    if (data.value.instance.name && data.value.instance.name !== d.instanceName) {
      d.instanceName = data.value.instance.name
      saveDevice(d)
    }
  } catch (e) {
    if (seq !== loadSeq) return
    if (e instanceof ApiError && e.status === 401) forget('Ce téléphone a été déconnecté depuis le PC. Demande un nouveau code pour le reconnecter.')
    else {
      error.value = e instanceof Error ? e.message : String(e)
      // Rien d'affiché : la dernière vue connue, signalée comme ancienne
      const last = data.value ? null : lastView()
      if (last) {
        data.value = last.data
        staleAt.value = last.at
      }
    }
  } finally {
    if (seq === loadSeq) loading.value = false
  }
}

// Plusieurs annonces rapprochées : une seule relecture
function refreshSoon() {
  if (refreshTimer) clearTimeout(refreshTimer)
  refreshTimer = setTimeout(() => { refreshTimer = null; void load() }, 300)
}

// État du PC seul (léger) : relu à chaque annonce « state » ; serveur plus ancien : tout relire
let stateTimer: ReturnType<typeof setTimeout> | null = null
function refreshStateSoon() {
  if (stateTimer) clearTimeout(stateTimer)
  stateTimer = setTimeout(async () => {
    stateTimer = null
    const d = device.value
    if (!d || !data.value) return refreshSoon()
    const seq = ++stateSeq
    try {
      const r = await loadStateOnly<InstanceState>(d.token)
      if (seq !== stateSeq) return
      if (data.value) {
        data.value.state = r.state
        data.value.stateAt = r.stateAt
      }
    } catch {
      refreshSoon()
    }
  }, 300)
}

// Demandes au PC (achats, mise à jour) : suivies jusqu'au résultat, annoncé par un message
const ACTION_TITLES: Record<string, [string, string]> = {
  se_buy: ['Achat fait par le PC', 'Achat refusé'],
  update_check: ['Mise à jour du PC', 'Vérification impossible'],
  update_install: ['Mise à jour lancée', 'Mise à jour impossible'],
  giveaway_hide: ['Fait par le PC', 'Refusé par le PC'],
  giveaway_conditions: ['Conditions enregistrées', 'Conditions non enregistrées'],
  giveaway_chat_log: ['Journal du chat', 'Journal du chat illisible'],
  channel_monitor: ['Chaîne surveillée', 'Chaîne non ajoutée'],
  points_order: ['Ordre enregistré', 'Ordre non enregistré'],
  giveaway_participate: ['Participation', 'Participation incomplète'],
  giveaway_follow: ['Follow', 'Follow impossible'],
  giveaway_hello: ['Bonjour', 'Bonjour impossible'],
  giveaway_delete: ['Giveaway supprimé', 'Suppression impossible'],
  win_reply_cancel: ['Réponse annulée', 'Annulation impossible'],
  app_pause: ['Application du PC', 'Pause impossible'],
  points_option: ['Réglage enregistré', 'Réglage refusé'],
  points_max: ['Solde maximum', 'Solde maximum refusé'],
  drop_campaign_ignore: ['Drops', 'Campagne non modifiée'],
  drops_refresh: ['Drops', 'Drops non relus'],
  se_option: ['Réglage enregistré', 'Réglage refusé']
}
// Journal du chat rendu par le PC : affiché dans sa fenêtre
const chatLog = ref<{ channel: string, entries: ChatLogEntry[] } | null>(null)
const chatLogOpen = ref(false)
const phoneActions = ref<PhoneAction[]>([])
const pendingActions = computed(() => phoneActions.value.filter(a => a.status === 'pending'))
async function loadActions() {
  const d = device.value
  if (!d) return
  try {
    const list = await listActions<PhoneAction>(d.token)
    for (const a of list) {
      const before = phoneActions.value.find(x => x.id === a.id)
      if (before?.status === 'pending' && a.status !== 'pending') {
        if (a.kind === 'giveaway_chat_log' && a.status === 'done' && a.data?.entries) {
          chatLog.value = { channel: a.data.channel ?? '', entries: a.data.entries }
          chatLogOpen.value = true
          continue
        }
        const [done, failed] = ACTION_TITLES[a.kind] ?? ACTION_TITLES.se_buy!
        toast.add(a.status === 'done'
          ? { title: done, description: a.message ?? '', color: 'success', icon: 'i-lucide-check' }
          : { title: failed, description: a.message ?? '', color: 'error' })
      }
    }
    phoneActions.value = list
  } catch { /* serveur plus ancien : pas d'achats */ }
}
function onRequested(a: PhoneAction) {
  phoneActions.value = [a, ...phoneActions.value]
}
// Label StreamElements d'un giveaway : page StreamElements filtrée sur sa chaîne, avec un compte
// qui l'a dans sa liste
const seOnly = ref<string | null>(null)
function openSe(channel: string) {
  const login = channel.toLowerCase()
  const acc = (data.value?.state?.streamelements ?? []).find(a => a.channels.some(c => c.login.toLowerCase() === login))
  if (acc) seAccount.value = acc.login
  seOnly.value = login
  tab.value = 'se'
}
watch(tab, (t) => { if (t !== 'se') seOnly.value = null })
providePhone({ token: computed(() => device.value?.token ?? null), actions: phoneActions, requested: onRequested, openSe })
const toast = useToast()

function connect(d: Device) {
  events?.close()
  events = openEvents(d.token, {
    // À l'ouverture du flux (événement « seq » initial) puis à chaque nouvelle : relecture ;
    // état du PC seul : relecture légère ; achat traité : résultat
    onEvent: (type) => {
      if (type === 'state') refreshStateSoon()
      else if (type === 'action') void loadActions()
      else refreshSoon()
    },
    onStatus: (s) => { status.value = s },
    onUnauthorized: () => forget('Ce téléphone a été déconnecté depuis le PC. Demande un nouveau code pour le reconnecter.')
  })
}

watch(device, (d) => {
  if (d) {
    void load()
    connect(d)
    void refreshPush(d.token)
    void loadActions()
  }
}, { immediate: true })

// Retour au premier plan : le flux a pu être coupé pendant la veille
function onVisible() {
  if (document.visibilityState === 'visible' && device.value) {
    events?.restart()
    refreshSoon()
  }
}
document.addEventListener('visibilitychange', onVisible)
onBeforeUnmount(() => {
  document.removeEventListener('visibilitychange', onVisible)
  events?.close()
})

function paired(d: Device) {
  saveDevice(d)
  notice.value = null
  device.value = d
}

// Notifications : réglages, et invitation tant qu'elles ne sont pas activées (masquable)
const notificationsOpen = ref(false)
const HINT_KEY = 'twitch-desk:push-hint-hidden'
const hintHidden = ref((() => { try { return localStorage.getItem(HINT_KEY) === '1' } catch { return false } })())
const showHint = computed(() => !pushActive.value && !hintHidden.value && !serverTooOld.value && pushSupport() !== 'unsupported')
function hideHint() {
  hintHidden.value = true
  try { localStorage.setItem(HINT_KEY, '1') } catch { /* stockage indisponible */ }
}

const logoutOpen = ref(false)
const updateOpen = ref(false)
// Pause générale de l'application du PC (plus de récolte, participation ni envoi)
const phoneCtx = { token: computed(() => device.value?.token ?? null), actions: phoneActions, requested: onRequested }
function togglePause(paused: boolean) {
  void sendAction(phoneCtx, toast, { kind: 'app_pause', enabled: paused })
}
// Réponses au gain en attente sur le PC : annulables d'ici pendant leur délai
const now = ref(Date.now())
const clock = setInterval(() => { now.value = Date.now() }, 1000)
onBeforeUnmount(() => clearInterval(clock))
const winReplies = computed(() => (data.value?.state?.winReplies ?? []).filter(r => new Date(r.sendAt).getTime() > now.value - 60_000))
const secondsLeft = (at: string) => Math.max(0, Math.round((new Date(at).getTime() - now.value) / 1000))
function cancelReply(id: string) {
  void sendAction(phoneCtx, toast, { kind: 'win_reply_cancel', item: id })
}

const menu = computed<DropdownMenuItem[][]>(() => [
  [
    { label: 'Notifications', icon: pushActive.value ? 'i-lucide-bell-ring' : 'i-lucide-bell', onSelect: () => { notificationsOpen.value = true } },
    { label: 'Mise à jour du PC', icon: 'i-lucide-download', onSelect: () => { updateOpen.value = true } },
    { label: 'Actualiser', icon: 'i-lucide-refresh-cw', onSelect: () => { void load() } },
    data.value?.state?.paused === undefined
      ? []
      : data.value.state.paused
        ? { label: 'Relancer l\'application du PC', icon: 'i-lucide-play', onSelect: () => togglePause(false) }
        : { label: 'Mettre le PC en pause', icon: 'i-lucide-pause', onSelect: () => togglePause(true) }
  ].flat(),
  [{ label: 'Se déconnecter de cet appareil', icon: 'i-lucide-log-out', color: 'error', onSelect: () => { logoutOpen.value = true } }]
])
</script>

<template>
  <UApp :toaster="{ position: 'top-center' }">
    <PairScreen v-if="!device" :notice="notice" @paired="paired" />
    <div v-else class="min-h-dvh flex flex-col">
      <header class="sticky top-0 z-10 border-b border-default bg-default/90 backdrop-blur pt-[env(safe-area-inset-top)]">
        <div class="flex items-center gap-3 px-4 h-14 max-w-2xl mx-auto">
          <img src="/icon.svg" alt="" width="28" height="28">
          <div class="min-w-0 flex-1">
            <div class="font-semibold truncate leading-tight">{{ instanceName }}</div>
            <div class="text-xs text-muted leading-tight">{{ TABS.find(t => t.key === tab)?.title ?? TABS.find(t => t.key === tab)?.label }}</div>
          </div>
          <UBadge :label="connection.label" :color="connection.color" :icon="connection.icon" variant="subtle" size="sm" />
          <UDropdownMenu :items="menu" :content="{ align: 'end' }">
            <UButton icon="i-lucide-ellipsis-vertical" color="neutral" variant="ghost" aria-label="Menu" />
          </UDropdownMenu>
        </div>
      </header>

      <main class="flex-1 w-full max-w-2xl mx-auto px-4 py-4 flex flex-col gap-4 pb-[calc(5rem+env(safe-area-inset-bottom))]">
        <!-- PC en pause ; réponses au gain en attente, annulables pendant leur délai -->
        <UAlert v-if="data?.state?.paused" color="warning" variant="subtle" icon="i-lucide-pause" title="Application du PC en pause" description="Ni récolte, ni participation, ni envoi dans le chat." :actions="[{ label: 'Relancer', onClick: () => togglePause(false) }]" />
        <UAlert
          v-for="r in winReplies"
          :key="r.id"
          color="success"
          variant="subtle"
          icon="i-lucide-trophy"
          :title="`${r.login} a gagné sur ${r.channel}`"
          :description="secondsLeft(r.sendAt) > 0 ? `Réponse dans ${secondsLeft(r.sendAt)} s : « ${r.text.replace(/\n/g, ' / ')} »` : `Réponse en cours d'envoi : « ${r.text.replace(/\n/g, ' / ')} »`"
          :actions="[{ label: 'Annuler', color: 'error', onClick: () => cancelReply(r.id) }]"
        />
        <UAlert v-if="error" color="error" variant="subtle" icon="i-lucide-circle-alert" :description="error" />
        <UAlert
          v-if="staleAt"
          color="warning"
          variant="subtle"
          icon="i-lucide-history"
          :description="`Serveur injoignable : dernière vue du ${new Date(staleAt).toLocaleString('fr-FR', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}, mise à jour dès qu'il répond.`"
        />
        <UAlert
          v-if="showHint"
          color="primary"
          variant="subtle"
          icon="i-lucide-bell"
          title="Recevoir les alertes sur ce téléphone"
          description="Giveaway détecté, gain, rappel de tirage… même web app fermée."
          :actions="[{ label: 'Activer', onClick: () => { notificationsOpen = true } }]"
          close
          @update:open="hideHint"
        />
        <div v-if="!data && loading" class="flex justify-center py-12">
          <UIcon name="i-lucide-loader" class="size-6 animate-spin text-muted" />
        </div>
        <template v-else-if="data && tab === 'points'">
          <PointsView v-model:account="pointsAccount" :state="data.state" />
        </template>
        <template v-else-if="data && tab === 'scanner'">
          <ScannerView :state="data.state" :highlight="highlight" />
        </template>
        <template v-else-if="data && tab === 'drops'">
          <DropsView v-model:account="dropsAccount" :state="data.state" />
        </template>
        <template v-else-if="data && tab === 'se'">
          <SeView v-model:account="seAccount" v-model:only="seOnly" :state="data.state" :token="device.token" :pending="pendingActions" @requested="onRequested" />
        </template>
        <template v-else-if="data">
          <p v-if="!data.state" class="text-sm text-muted">L'application du PC n'a pas encore publié ses participations (mets-la à jour et active la synchronisation).</p>
          <div v-if="!data.giveaways.length" class="flex flex-col items-center gap-2 py-12 text-center">
            <UIcon name="i-lucide-gift" class="size-10 text-muted" />
            <p class="font-medium">Aucun giveaway en cours</p>
            <p class="text-sm text-muted">Ils apparaissent ici dès que l'application en détecte un.</p>
          </div>
          <USelect v-if="data.giveaways.length" v-model="listChoice" :items="listItems" class="w-full" aria-label="Giveaways affichés" />
          <section v-if="shown.length" class="flex flex-col gap-3" aria-labelledby="shown-title">
            <h1 id="shown-title" class="text-sm font-medium text-muted uppercase tabular-nums">{{ shownTitle }} · {{ Math.min(shownIndex, shown.length - 1) + 1 }} / {{ shown.length }}</h1>
            <p v-if="mode === 'paused'" class="text-xs text-muted -mt-1">Live terminé, tirage pas encore fait.</p>
            <p v-else-if="mode === 'hidden'" class="text-xs text-muted -mt-1">Masqués sur le PC : ni alerte, ni participation automatique.</p>
            <GiveawayCarousel :key="mode" v-model:index="shownIndex" :giveaways="shown" :state="data.state" />
          </section>
          <p v-else-if="data.giveaways.length" class="text-sm text-muted py-6 text-center">{{ mode === 'hidden' ? 'Aucun giveaway masqué.' : mode === 'paused' ? 'Aucun giveaway en pause.' : 'Aucun giveaway en cours : choisis une autre liste ci-dessus.' }}</p>
          <MentionList v-if="data.state?.mentions?.length" id="mentions" :mentions="data.state.mentions" :langs="data.state.languages" :avatars="data.state.avatars" :highlight="highlight" class="scroll-mt-20" />
          <p v-if="data.stateAt" class="text-xs text-muted text-center">Participations publiées par le PC {{ timeAgo(data.stateAt) }}</p>
        </template>
      </main>

      <!-- Onglets en bas de l'écran -->
      <nav class="fixed bottom-0 inset-x-0 z-10 border-t border-default bg-default/95 backdrop-blur pb-[env(safe-area-inset-bottom)]" aria-label="Pages">
        <div class="max-w-2xl mx-auto grid grid-cols-5">
          <button
            v-for="t in TABS"
            :key="t.key"
            type="button"
            class="flex flex-col items-center gap-0.5 py-2 text-xs font-medium"
            :class="tab === t.key ? 'text-primary' : 'text-muted'"
            :aria-current="tab === t.key ? 'page' : undefined"
            :aria-label="t.title"
            @click="tab = t.key"
          >
            <UIcon :name="t.icon" class="size-5" />
            {{ t.label }}
          </button>
        </div>
      </nav>
    </div>

    <NotificationsModal v-if="device" v-model:open="notificationsOpen" :token="device.token" />
    <ChatLogModal v-model:open="chatLogOpen" :log="chatLog" />
    <UpdateModal v-if="device" v-model:open="updateOpen" :state="data?.state ?? null" :token="device.token" :pending="pendingActions" @requested="onRequested" />

    <UModal v-model:open="logoutOpen" title="Se déconnecter de cet appareil ?">
      <template #body>
        <p class="text-sm">Ce téléphone oublie son accès. Pour le reconnecter, il faudra un nouveau code depuis le PC. Pense aussi à le retirer de la liste sur le PC.</p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="logoutOpen = false" />
          <UButton label="Se déconnecter" color="error" @click="logoutOpen = false; forget(null)" />
        </div>
      </template>
    </UModal>
  </UApp>
</template>
