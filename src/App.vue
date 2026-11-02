<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getVersion } from '@tauri-apps/api/app'
import AppUsage from './components/AppUsage.vue'
import GiftPreviewModal from './components/GiftPreviewModal.vue'
import type { NavigationMenuItem } from '@nuxt/ui'
import { useSnapshot } from './composables/useSnapshot'
import { useUpdater } from './composables/useUpdater'
import { installSoundAlerts } from './composables/useSoundAlerts'
import { useSettings } from './composables/useSettings'
import { installLogin } from './composables/useLogin'
import UpdateBanner from './components/UpdateBanner.vue'
import GiveawayFormModal from './components/GiveawayFormModal.vue'

const { snapshot, accounts, paused } = useSnapshot()
const updater = useUpdater()
const version = ref('')

const needsAttention = computed(() => accounts.value.filter(a => a.status === 'expired').length)
const liveCount = computed(() => accounts.value.flatMap(a => a.channels).filter(c => c.live).length)
const watchingCount = computed(() => accounts.value.flatMap(a => a.channels).filter(c => c.state === 'watching').length)
const inChatCount = computed(() => accounts.value.flatMap(a => a.channels).filter(c => c.inChat).length)
const dropsCount = computed(() => accounts.value.filter(a => a.drops.target).length)
// Alertes en attente : giveaways ouverts non traités et mentions
const alertCount = computed(() => {
  const g = snapshot.value?.giveaways
  return g ? g.giveaways.filter(x => x.open && !x.handled).length + g.mentions.length : 0
})

const items = computed<NavigationMenuItem[]>(() => [
  { label: 'Tableau de bord', icon: 'i-lucide-layout-dashboard', to: '/' },
  {
    label: 'Giveaways',
    icon: 'i-lucide-gift',
    defaultOpen: true,
    badge: alertCount.value ? { label: String(alertCount.value), color: 'primary' } : undefined,
    children: [
      { label: 'Alertes', icon: 'i-lucide-bell', to: '/giveaways', exact: true },
      { label: 'Chaînes', icon: 'i-lucide-tv', to: '/giveaways/channels' },
      { label: 'Catégories', icon: 'i-lucide-gamepad-2', to: '/giveaways/categories' },
      { label: 'Scanner', icon: 'i-lucide-radar', to: '/giveaways/scanner' },
      { label: 'Cadeaux', icon: 'i-lucide-package', to: '/giveaways/gifts' },
      { label: 'Mes gains', icon: 'i-lucide-trophy', to: '/giveaways/wins' },
      { label: 'Réglages', icon: 'i-lucide-sliders-horizontal', to: '/giveaways/settings' }
    ]
  },
  { label: 'Points de chaîne', icon: 'i-lucide-coins', to: '/points', badge: watchingCount.value || undefined },
  { label: 'StreamElements', icon: 'i-lucide-ticket', to: '/streamelements', badge: inChatCount.value || undefined },
  { label: 'Drops', icon: 'i-lucide-package-open', to: '/drops', badge: dropsCount.value || undefined },
  { label: 'Comptes', icon: 'i-lucide-users', to: '/accounts', badge: needsAttention.value ? { label: String(needsAttention.value), color: 'error' } : undefined },
  { label: 'Paramètres', icon: 'i-lucide-settings', to: '/settings' }
])

async function togglePause() {
  await invoke('set_paused', { paused: !paused.value })
}

installSoundAlerts()
// Connexion Twitch / StreamElements : suivie même en changeant de page
installLogin()

// Mises à jour automatiques : selon le réglage (chargé au démarrage, suivi s'il change)
const { settings } = useSettings()
watch(() => settings.value?.autoUpdate, (on) => {
  if (on === undefined) return
  if (on) updater.startAutoCheck()
  else updater.stopAutoCheck()
}, { immediate: true })

// Fiche d'un streamer demandée depuis la fenêtre des streams
const streamerOpen = ref(false)
const streamerLogin = ref<string | null>(null)
void listen<string>('open-streamer', async (e) => {
  // Une autre fiche déjà ouverte : refermée d'abord pour être relue
  if (streamerOpen.value) {
    streamerOpen.value = false
    await nextTick()
  }
  streamerLogin.value = e.payload
  streamerOpen.value = true
})

// Clic sur une notification Windows : page de l'alerte (scanner, gains, giveaways), aussi
// quand c'est lui qui a démarré l'application
const router = useRouter()
function navigate(page: string | null) {
  if (page?.startsWith('/') && router.currentRoute.value.path !== page) void router.push(page)
}
void listen<string>('navigate', e => navigate(e.payload))
void invoke<string | null>('take_start_page').then(navigate)

onMounted(async () => {
  version.value = await getVersion()
})
</script>

<template>
  <UApp :toaster="{ position: 'bottom-right' }">
    <div class="flex h-screen">
      <aside class="w-56 shrink-0 border-r border-default bg-elevated/40 flex flex-col">
        <div class="px-4 py-4 flex items-center gap-2">
          <img src="/icon.svg" alt="" class="size-7">
          <span class="font-semibold">Twitch Desk</span>
        </div>
        <UNavigationMenu :items="items" orientation="vertical" class="px-2" />
        <div class="mt-auto p-3 flex flex-col gap-2 text-xs text-muted">
          <UButton
            :icon="paused ? 'i-lucide-play' : 'i-lucide-pause'"
            :label="paused ? 'Reprendre' : 'Tout mettre en pause'"
            :color="paused ? 'primary' : 'neutral'"
            variant="soft"
            block
            @click="togglePause"
          />
          <div class="flex items-center gap-1.5">
            <span class="size-2 rounded-full" :class="paused ? 'bg-warning' : 'bg-success'" />
            {{ paused ? 'En pause' : `Actif · ${liveCount} en live` }}
          </div>
          <AppUsage />
          <div>v{{ version }}</div>
        </div>
      </aside>
      <main class="flex-1 min-w-0 flex flex-col">
        <UpdateBanner />
        <div class="flex-1 overflow-y-auto">
          <RouterView />
        </div>
      </main>
      <GiveawayFormModal v-model:open="streamerOpen" :giveaway="null" :channel="streamerLogin" streamer-only />
      <GiftPreviewModal />
    </div>
  </UApp>
</template>
