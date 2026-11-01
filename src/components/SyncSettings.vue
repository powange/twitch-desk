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
// enabled : enregistré ; wanted : interrupteur (coché, il affiche l'adresse et le code
// jusqu'à l'enregistrement)
const enabled = ref(false)
const wanted = ref(false)
const hasCode = ref(false)
const saving = ref(false)
const testing = ref(false)
const tested = ref<{ version: string, collections: string[] } | null>(null)

onMounted(async () => {
  const config = await invoke<{ url: string, enabled: boolean, hasCode: boolean }>('get_sync_config')
  url.value = config.url
  enabled.value = config.enabled
  wanted.value = config.enabled
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

// Données proposées par le serveur, à cocher ou décocher sur ce poste
const offered = computed(() => syncView.value?.serverCollections ?? [])
const syncing = (col: string) => syncView.value?.collections.includes(col) ?? false
async function setCollection(col: string, on: boolean) {
  const skipped = offered.value.filter(c => c !== col && !syncing(c))
  if (!on) skipped.push(col)
  try {
    await invoke('set_sync_skipped', { skipped })
  } catch (e) {
    toast.add({ title: 'Choix non enregistré', description: String(e), color: 'error' })
  }
}

const resyncOpen = ref(false)
async function resync() {
  resyncOpen.value = false
  try {
    await invoke('sync_resync')
    toast.add({ title: 'Relecture complète lancée', description: 'Tout le contenu du serveur va être relu.', color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Relecture impossible', description: String(e), color: 'error' })
  }
}

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

// Décoché : coupé tout de suite (adresse et code gardés) ; coché : activé tout de suite si
// l'adresse et le code sont déjà connus, sinon après « Enregistrer »
async function toggle(value: boolean) {
  wanted.value = value
  if (!value) {
    if (enabled.value) await save(false)
  } else if (url.value.trim() && hasCode.value) {
    await save(true)
  }
}

async function save(value = wanted.value) {
  saving.value = true
  try {
    await invoke('save_sync_config', { url: url.value, code: code.value || null, enabled: value })
    enabled.value = value
    wanted.value = value
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
      <USwitch :model-value="wanted" :loading="saving" aria-label="Synchronisation" @update:model-value="toggle" />
    </div>
    <div v-if="wanted" class="flex flex-col gap-3 px-4 py-3">
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
    <div v-if="wanted && syncView && syncView.status !== 'off'" class="flex flex-col gap-2 px-4 py-3">
      <div class="flex items-center gap-2">
        <UBadge :label="status.label" :color="status.color" variant="subtle" />
        <span v-if="syncView.lastSync" class="text-sm text-muted">Dernière synchronisation {{ timeAgo(syncView.lastSync) }}</span>
        <span class="flex-1" />
        <UButton label="Synchroniser maintenant" icon="i-lucide-refresh-cw" size="sm" color="neutral" variant="soft" :loading="syncView.status === 'syncing'" @click="invoke('sync_now')" />
      </div>
      <p v-if="syncView.error" class="text-sm text-error">{{ syncView.error }}</p>
      <div v-if="offered.length" class="flex flex-col gap-2 pt-1">
        <div class="text-sm font-medium">Données synchronisées</div>
        <div class="text-xs text-muted">Celles que propose le serveur. Une donnée décochée reste sur ce poste sans être envoyée ni reçue.</div>
        <table class="text-sm w-full max-w-md">
          <thead class="text-xs uppercase text-muted">
            <tr>
              <th class="text-left py-1 font-medium">Donnée</th>
              <th class="text-right py-1 font-medium">Ici</th>
              <th class="text-right py-1 font-medium">Serveur</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="col in offered" :key="col" class="border-t border-default">
              <td class="py-1.5">
                <UCheckbox :model-value="syncing(col)" :label="labelOf(col)" @update:model-value="v => setCollection(col, v === true)" />
              </td>
              <td class="py-1.5 text-right tabular-nums">{{ syncView.localCounts[col] ?? '—' }}</td>
              <td
                class="py-1.5 text-right tabular-nums"
                :class="syncing(col) && syncView.serverCounts && syncView.serverCounts[col] !== undefined && syncView.serverCounts[col] !== syncView.localCounts[col] ? 'text-warning' : ''"
              >
                {{ syncView.serverCounts ? (syncView.serverCounts[col] ?? 0) : '?' }}
              </td>
            </tr>
          </tbody>
        </table>
        <div class="text-xs text-muted">
          Un écart (en orange) peut venir d'éléments propres à ce poste ou de giveaways de plus de 60 jours.
          <template v-if="!syncView.serverCounts">Mets le serveur à jour pour voir ses comptes.</template>
        </div>
        <div>
          <UButton label="Tout relire depuis le serveur" icon="i-lucide-rotate-ccw" size="sm" color="neutral" variant="soft" @click="resyncOpen = true" />
        </div>
      </div>
      <div class="text-xs text-muted">Depuis le démarrage : {{ syncView.sent }} modification(s) envoyée(s), {{ syncView.received }} reçue(s).</div>
    </div>
    <UModal v-model:open="resyncOpen" title="Tout relire depuis le serveur ?">
      <template #body>
        <p class="text-sm">Tout le contenu du serveur est relu. Pour ce qui est déjà partagé, la version du serveur remplace celle de ce poste ; ce qui n'existe qu'ici est envoyé au serveur.</p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="resyncOpen = false" />
          <UButton label="Tout relire" @click="resync" />
        </div>
      </template>
    </UModal>
  </div>
</template>
