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
  const msg = e.data as { tab?: unknown, focus?: unknown } | null
  const t = asTab(msg?.tab)
  if (t) tab.value = t
  if (typeof msg?.focus === 'string') focus.value = msg.focus
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

const open = computed(() => data.value?.giveaways.filter(g => g.status === 'open') ?? [])
const paused = computed(() => data.value?.giveaways.filter(g => g.status === 'paused') ?? [])
// Giveaway affiché dans chaque carrousel
const openIndex = ref(0)
const pausedIndex = ref(0)
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
  update_install: ['Mise à jour lancée', 'Mise à jour impossible']
}
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
const menu = computed<DropdownMenuItem[][]>(() => [
  [
    { label: 'Notifications', icon: pushActive.value ? 'i-lucide-bell-ring' : 'i-lucide-bell', onSelect: () => { notificationsOpen.value = true } },
    { label: 'Mise à jour du PC', icon: 'i-lucide-download', onSelect: () => { updateOpen.value = true } },
    { label: 'Actualiser', icon: 'i-lucide-refresh-cw', onSelect: () => { void load() } }
  ],
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
          <ScannerView :state="data.state" />
        </template>
        <template v-else-if="data && tab === 'drops'">
          <DropsView v-model:account="dropsAccount" :state="data.state" />
        </template>
        <template v-else-if="data && tab === 'se'">
          <SeView v-model:account="seAccount" :state="data.state" :token="device.token" :pending="pendingActions" @requested="onRequested" />
        </template>
        <template v-else-if="data">
          <p v-if="!data.state" class="text-sm text-muted">L'application du PC n'a pas encore publié ses participations (mets-la à jour et active la synchronisation).</p>
          <div v-if="!data.giveaways.length" class="flex flex-col items-center gap-2 py-12 text-center">
            <UIcon name="i-lucide-gift" class="size-10 text-muted" />
            <p class="font-medium">Aucun giveaway en cours</p>
            <p class="text-sm text-muted">Ils apparaissent ici dès que l'application en détecte un.</p>
          </div>
          <section v-if="open.length" class="flex flex-col gap-3" aria-labelledby="open-title">
            <h1 id="open-title" class="text-sm font-medium text-muted uppercase tabular-nums">En cours · {{ Math.min(openIndex, open.length - 1) + 1 }} / {{ open.length }}</h1>
            <GiveawayCarousel v-model:index="openIndex" :giveaways="open" :state="data.state" />
          </section>
          <section v-if="paused.length" class="flex flex-col gap-3" aria-labelledby="paused-title">
            <h1 id="paused-title" class="text-sm font-medium text-muted uppercase tabular-nums">En pause · {{ Math.min(pausedIndex, paused.length - 1) + 1 }} / {{ paused.length }}</h1>
            <p class="text-xs text-muted -mt-1">Live terminé, tirage pas encore fait.</p>
            <GiveawayCarousel v-model:index="pausedIndex" :giveaways="paused" :state="data.state" />
          </section>
          <MentionList v-if="data.state?.mentions?.length" id="mentions" :mentions="data.state.mentions" class="scroll-mt-20" />
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
