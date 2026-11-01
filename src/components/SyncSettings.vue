<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../composables/useSnapshot'
import { timeAgo } from '../utils/format'

// Serveur de synchronisation partagé : adresse et code communiqués par celui qui l'héberge
const { snapshot } = useSnapshot()
const toast = useToast()

const COLLECTION_LABELS: Record<string, string> = {
  groups: 'Groupes de cadeaux',
  gifts: 'Cadeaux',
  giveaways: 'Giveaways',
  channels: 'Chaînes surveillées',
  streamers: 'Fiches des streamers'
}
const labelOf = (c: string) => COLLECTION_LABELS[c] ?? c

const url = ref('')
const code = ref('')
const enabled = ref(false)
const hasCode = ref(false)
const saving = ref(false)
const testing = ref(false)
const tested = ref<{ version: string, collections: string[] } | null>(null)

onMounted(async () => {
  const config = await invoke<{ url: string, enabled: boolean, hasCode: boolean }>('get_sync_config')
  url.value = config.url
  enabled.value = config.enabled
  hasCode.value = config.hasCode
})

const syncView = computed(() => snapshot.value?.sync)
const status = computed(() => {
  const s = syncView.value
  if (!s || s.status === 'off') return { label: 'Désactivée', color: 'neutral' as const }
  if (s.status === 'error') return { label: 'Erreur', color: 'error' as const }
  if (s.status === 'syncing') return { label: 'Synchronisation…', color: 'info' as const }
  return { label: 'Synchronisée', color: 'success' as const }
})

async function test() {
  testing.value = true
  tested.value = null
  try {
    tested.value = await invoke('test_sync', { url: url.value, code: code.value || null })
  } catch (e) {
    toast.add({ title: 'Connexion impossible', description: String(e), color: 'error' })
  } finally {
    testing.value = false
  }
}

async function save(value = enabled.value) {
  saving.value = true
  try {
    await invoke('save_sync_config', { url: url.value, code: code.value || null, enabled: value })
    enabled.value = value
    if (code.value) hasCode.value = true
    code.value = ''
    toast.add({ title: value ? 'Synchronisation activée' : 'Synchronisation enregistrée', color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Synchronisation non enregistrée', description: String(e), color: 'error' })
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div class="border border-default rounded-lg divide-y divide-default">
    <div class="flex items-center justify-between gap-4 px-4 py-3">
      <div>
        <div class="font-medium">Mettre des données en commun</div>
        <div class="text-sm text-muted">Ce que les autres ajoutent arrive chez toi, et inversement. Le serveur choisit ce qui est partagé.</div>
      </div>
      <USwitch :model-value="enabled" :loading="saving" aria-label="Synchronisation" @update:model-value="v => save(v)" />
    </div>
    <div class="flex flex-col gap-3 px-4 py-3">
      <UFormField label="Adresse du serveur">
        <UInput id="sync-url" v-model="url" placeholder="https://sync.exemple.fr" class="w-full" autocomplete="off" />
      </UFormField>
      <UFormField label="Code du serveur" :help="hasCode ? 'Un code est enregistré : laisse vide pour le garder' : 'Communiqué par la personne qui héberge le serveur'">
        <UInput id="sync-code" v-model="code" type="password" :placeholder="hasCode ? '••••••••' : ''" class="w-full" autocomplete="off" />
      </UFormField>
      <div class="flex items-center gap-2">
        <UButton label="Tester la connexion" icon="i-lucide-plug" color="neutral" variant="soft" :loading="testing" :disabled="!url.trim()" @click="test" />
        <UButton label="Enregistrer" :loading="saving" :disabled="!url.trim()" @click="save()" />
      </div>
      <UAlert
        v-if="tested"
        icon="i-lucide-check"
        color="success"
        variant="subtle"
        :title="`Serveur joignable (version ${tested.version})`"
        :description="`Partagé : ${tested.collections.map(col => labelOf(col)).join(', ') || 'rien'}`"
      />
    </div>
    <div v-if="syncView && syncView.status !== 'off'" class="flex flex-col gap-2 px-4 py-3">
      <div class="flex items-center gap-2">
        <UBadge :label="status.label" :color="status.color" variant="subtle" />
        <span v-if="syncView.lastSync" class="text-sm text-muted">Dernière synchronisation {{ timeAgo(syncView.lastSync) }}</span>
        <span class="flex-1" />
        <UButton label="Synchroniser maintenant" icon="i-lucide-refresh-cw" size="sm" color="neutral" variant="soft" :loading="syncView.status === 'syncing'" @click="invoke('sync_now')" />
      </div>
      <p v-if="syncView.error" class="text-sm text-error">{{ syncView.error }}</p>
      <div v-if="syncView.collections.length" class="flex items-center gap-1.5 flex-wrap text-sm">
        <span class="text-muted">Partagé :</span>
        <UBadge v-for="col in syncView.collections" :key="col" :label="labelOf(col)" color="neutral" variant="subtle" />
      </div>
      <div class="text-xs text-muted">Depuis le démarrage : {{ syncView.sent }} modification(s) envoyée(s), {{ syncView.received }} reçue(s).</div>
    </div>
  </div>
</template>
