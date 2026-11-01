<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getVersion } from '@tauri-apps/api/app'
import type { NavigationMenuItem } from '@nuxt/ui'
import { useSnapshot } from './composables/useSnapshot'
import { useUpdater } from './composables/useUpdater'
import type { Settings } from './types'
import UpdateBanner from './components/UpdateBanner.vue'

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
  { label: 'Giveaways', icon: 'i-lucide-gift', to: '/giveaways', badge: alertCount.value ? { label: String(alertCount.value), color: 'primary' } : undefined },
  { label: 'Points de chaîne', icon: 'i-lucide-coins', to: '/points', badge: watchingCount.value || undefined },
  { label: 'Drops', icon: 'i-lucide-package-open', to: '/drops', badge: dropsCount.value || undefined },
  { label: 'StreamElements', icon: 'i-lucide-ticket', to: '/streamelements', badge: inChatCount.value || undefined },
  { label: 'Comptes', icon: 'i-lucide-users', to: '/accounts', badge: needsAttention.value ? { label: String(needsAttention.value), color: 'error' } : undefined },
  { label: 'Paramètres', icon: 'i-lucide-settings', to: '/settings' }
])

async function togglePause() {
  await invoke('set_paused', { paused: !paused.value })
}

onMounted(async () => {
  version.value = await getVersion()
  const settings = await invoke<Settings>('get_settings')
  if (settings.autoUpdate) updater.startAutoCheck()
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
          <div>v{{ version }}</div>
        </div>
      </aside>
      <main class="flex-1 min-w-0 flex flex-col">
        <UpdateBanner />
        <div class="flex-1 overflow-y-auto">
          <RouterView />
        </div>
      </main>
    </div>
  </UApp>
</template>
