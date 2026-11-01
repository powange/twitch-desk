<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getVersion } from '@tauri-apps/api/app'
import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import { useColorMode } from '@vueuse/core'
import { useUpdater } from '../composables/useUpdater'
import { useSnapshot } from '../composables/useSnapshot'
import SyncSettings from '../components/SyncSettings.vue'
import type { Settings } from '../types'

const toast = useToast()
const updater = useUpdater()
const { snapshot } = useSnapshot()
// « auto » suit le thème de Windows
const { store: theme } = useColorMode()

const settings = ref<Settings | null>(null)
const autostart = ref(false)
const version = ref('')
const paths = ref<{ data: string, logs: string } | null>(null)

const themeItems = [
  { label: 'Système', value: 'auto' },
  { label: 'Clair', value: 'light' },
  { label: 'Sombre', value: 'dark' }
]

onMounted(async () => {
  settings.value = await invoke<Settings>('get_settings')
  version.value = await getVersion()
  paths.value = await invoke('get_paths')
  try {
    autostart.value = await isEnabled()
  } catch {
    autostart.value = false
  }
})

watch(settings, async (value, old) => {
  if (!value || !old) return
  try {
    await invoke('save_settings', { settings: value })
    if (value.autoUpdate) updater.startAutoCheck()
    else updater.stopAutoCheck()
  } catch (e) {
    toast.add({ title: 'Réglage non enregistré', description: String(e), color: 'error' })
  }
}, { deep: true })

async function setAutostart(value: boolean) {
  try {
    if (value) await enable()
    else await disable()
    autostart.value = value
  } catch (e) {
    toast.add({ title: 'Démarrage automatique indisponible', description: String(e), color: 'error' })
  }
}

async function checkUpdates() {
  const found = await updater.checkNow()
  if (updater.error.value) toast.add({ title: 'Vérification impossible', description: updater.error.value, color: 'error' })
  else if (!found) toast.add({ title: 'Twitch Desk est à jour', color: 'success', icon: 'i-lucide-check' })
}
</script>

<template>
  <div class="p-6 flex flex-col gap-6 max-w-3xl">
    <h1 class="text-xl font-semibold">Paramètres</h1>

    <section v-if="settings" class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">Général</h2>
      <div class="border border-default rounded-lg divide-y divide-default">
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div>
            <div class="font-medium">Démarrer avec Windows</div>
            <div class="text-sm text-muted">L'application démarre cachée, dans la zone de notification.</div>
          </div>
          <USwitch :model-value="autostart" aria-label="Démarrer avec Windows" @update:model-value="setAutostart" />
        </div>
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div>
            <div class="font-medium">Fermer la fenêtre garde l'application active</div>
            <div class="text-sm text-muted">La récolte continue ; l'icône de la zone de notification rouvre la fenêtre.</div>
          </div>
          <USwitch v-model="settings.closeToTray" aria-label="Fermer dans la zone de notification" />
        </div>
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div>
            <div class="font-medium">Démarrer réduit</div>
            <div class="text-sm text-muted">Ne pas afficher la fenêtre au lancement.</div>
          </div>
          <USwitch v-model="settings.startMinimized" aria-label="Démarrer réduit" />
        </div>
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div class="font-medium">Thème</div>
          <USelect v-model="theme" :items="themeItems" class="w-36" />
        </div>
      </div>
    </section>

    <section v-if="settings" class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">Mises à jour</h2>
      <div class="border border-default rounded-lg divide-y divide-default">
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div>
            <div class="font-medium">Chercher les mises à jour automatiquement</div>
            <div class="text-sm text-muted">Au démarrage puis toutes les 6 heures. L'installation se lance quand tu la confirmes.</div>
          </div>
          <USwitch v-model="settings.autoUpdate" aria-label="Mises à jour automatiques" />
        </div>
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div>
            <div class="font-medium">Version {{ version }}</div>
            <div class="text-sm text-muted">
              <template v-if="updater.available.value">Version {{ updater.available.value.version }} disponible.</template>
              <template v-else-if="updater.lastCheck.value">Dernière vérification à {{ updater.lastCheck.value.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' }) }}.</template>
              <template v-else>Pas encore vérifié.</template>
            </div>
          </div>
          <UButton
            v-if="updater.available.value"
            label="Installer et redémarrer"
            :loading="updater.installing.value"
            @click="updater.install"
          />
          <UButton v-else label="Vérifier maintenant" color="neutral" variant="soft" :loading="updater.checking.value" @click="checkUpdates" />
        </div>
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div>
            <div class="font-medium">Requêtes Twitch</div>
            <div class="text-sm text-muted">
              Version {{ snapshot?.operationsVersion }} du {{ snapshot?.operationsUpdated }}. Mises à jour à chaque démarrage, sans nouvelle version de l'application.
            </div>
          </div>
        </div>
      </div>
    </section>

    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">Synchronisation</h2>
      <SyncSettings />
    </section>

    <section v-if="paths" class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">Données</h2>
      <div class="border border-default rounded-lg divide-y divide-default">
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div class="min-w-0">
            <div class="font-medium">Dossier des données</div>
            <div class="text-sm text-muted truncate">{{ paths.data }}</div>
          </div>
          <UButton label="Ouvrir" color="neutral" variant="soft" @click="revealItemInDir(paths.data)" />
        </div>
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div class="min-w-0">
            <div class="font-medium">Journaux</div>
            <div class="text-sm text-muted truncate">{{ paths.logs }}, 7 jours conservés.</div>
          </div>
          <UButton label="Ouvrir" color="neutral" variant="soft" @click="revealItemInDir(paths.logs)" />
        </div>
      </div>
    </section>
  </div>
</template>
