<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { DropdownMenuItem } from '@nuxt/ui'
import { ApiError, loadDevice, openEvents, overview, saveDevice, type Device, type LiveStatus } from './api'
import type { Overview } from './types'
import { timeAgo } from './format'
import PairScreen from './components/PairScreen.vue'
import GiveawayCard from './components/GiveawayCard.vue'
import NotificationsModal from './components/NotificationsModal.vue'
import { active as pushActive, disable as disablePush, refresh as refreshPush, serverTooOld, support as pushSupport } from './push'

// Lien d'association scanné alors que l'appareil est déjà associé : nouvelle association
const device = ref<Device | null>(/pair=\d{6}/.test(location.hash) ? null : loadDevice())
const notice = ref<string | null>(null)
const data = ref<Overview | null>(null)
const error = ref<string | null>(null)
const loading = ref(false)
const status = ref<LiveStatus>('connecting')
let events: ReturnType<typeof openEvents> | null = null
let refreshTimer: ReturnType<typeof setTimeout> | null = null

const open = computed(() => data.value?.giveaways.filter(g => g.status === 'open') ?? [])
const paused = computed(() => data.value?.giveaways.filter(g => g.status === 'paused') ?? [])
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
  notice.value = message
}

async function load() {
  const d = device.value
  if (!d) return
  loading.value = true
  try {
    data.value = await overview<Overview>(d.token)
    error.value = null
    if (data.value.instance.name && data.value.instance.name !== d.instanceName) {
      d.instanceName = data.value.instance.name
      saveDevice(d)
    }
  } catch (e) {
    if (e instanceof ApiError && e.status === 401) forget('Ce téléphone a été déconnecté depuis le PC. Demande un nouveau code pour le reconnecter.')
    else error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

// Plusieurs annonces rapprochées : une seule relecture
function refreshSoon() {
  if (refreshTimer) clearTimeout(refreshTimer)
  refreshTimer = setTimeout(() => { refreshTimer = null; void load() }, 300)
}

function connect(d: Device) {
  events?.close()
  events = openEvents(d.token, {
    // À l'ouverture du flux (événement « seq » initial) puis à chaque nouvelle : relecture
    onEvent: refreshSoon,
    onStatus: (s) => { status.value = s },
    onUnauthorized: () => forget('Ce téléphone a été déconnecté depuis le PC. Demande un nouveau code pour le reconnecter.')
  })
}

watch(device, (d) => {
  if (d) {
    void load()
    connect(d)
    void refreshPush(d.token)
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
const menu = computed<DropdownMenuItem[][]>(() => [
  [
    { label: 'Notifications', icon: pushActive.value ? 'i-lucide-bell-ring' : 'i-lucide-bell', onSelect: () => { notificationsOpen.value = true } },
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
            <div class="text-xs text-muted leading-tight">Giveaways</div>
          </div>
          <UBadge :label="connection.label" :color="connection.color" :icon="connection.icon" variant="subtle" size="sm" />
          <UDropdownMenu :items="menu" :content="{ align: 'end' }">
            <UButton icon="i-lucide-ellipsis-vertical" color="neutral" variant="ghost" aria-label="Menu" />
          </UDropdownMenu>
        </div>
      </header>

      <main class="flex-1 w-full max-w-2xl mx-auto px-4 py-4 flex flex-col gap-4 pb-[calc(1rem+env(safe-area-inset-bottom))]">
        <UAlert v-if="error" color="error" variant="subtle" icon="i-lucide-circle-alert" :description="error" />
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
        <template v-else-if="data">
          <p v-if="!data.state" class="text-sm text-muted">L'application du PC n'a pas encore publié ses participations (mets-la à jour et active la synchronisation).</p>
          <div v-if="!data.giveaways.length" class="flex flex-col items-center gap-2 py-12 text-center">
            <UIcon name="i-lucide-gift" class="size-10 text-muted" />
            <p class="font-medium">Aucun giveaway en cours</p>
            <p class="text-sm text-muted">Ils apparaissent ici dès que l'application en détecte un.</p>
          </div>
          <section v-if="open.length" class="flex flex-col gap-3" aria-labelledby="open-title">
            <h1 id="open-title" class="text-sm font-medium text-muted uppercase">En cours · {{ open.length }}</h1>
            <GiveawayCard v-for="g in open" :key="g.id" :giveaway="g" :state="data.state" />
          </section>
          <section v-if="paused.length" class="flex flex-col gap-3" aria-labelledby="paused-title">
            <h1 id="paused-title" class="text-sm font-medium text-muted uppercase">En pause · {{ paused.length }}</h1>
            <p class="text-xs text-muted -mt-1">Live terminé, tirage pas encore fait.</p>
            <GiveawayCard v-for="g in paused" :key="g.id" :giveaway="g" :state="data.state" />
          </section>
          <p v-if="data.stateAt" class="text-xs text-muted text-center">Participations publiées par le PC {{ timeAgo(data.stateAt) }}</p>
        </template>
      </main>
    </div>

    <NotificationsModal v-if="device" v-model:open="notificationsOpen" :token="device.token" />

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
