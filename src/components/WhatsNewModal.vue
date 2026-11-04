<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { invoke } from '@tauri-apps/api/core'
import { CHANGELOG } from '../changelog'
import ChangelogView from './ChangelogView.vue'

// Premier lancement après une mise à jour : changelog des versions arrivées depuis la
// dernière vue (gardée en base ; plusieurs si on en a sauté), une seule fois
// Ancien emplacement (v0.48.12) : relu si la base n'a encore rien
const KEY = 'td:lastSeenVersion'
const open = ref(false)
const current = ref<string | null>(null)
const versions = ref<string[]>([])

// Comparaison « 0.48.10 » > « 0.48.9 »
function compare(a: string, b: string) {
  const pa = a.split('.').map(Number)
  const pb = b.split('.').map(Number)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0)
    if (d) return d
  }
  return 0
}

function storedLocally(): string | null {
  try {
    const v = localStorage.getItem(KEY)
    localStorage.removeItem(KEY)
    return v
  } catch {
    return null
  }
}

onMounted(async () => {
  const now = await getVersion()
  current.value = now
  // Version d'avant : en base (sinon d'après les journaux des 7 derniers jours) ; la version
  // installée y est retenue à la place
  let seen: string | null = null
  try {
    seen = await invoke<string | null>('take_previous_version')
  } catch {
    return
  }
  const local = storedLocally()
  if (!seen) seen = local
  // Rien de connu (première installation) : seulement la version installée
  const fresh = CHANGELOG
    .map(r => r.version)
    .filter(v => compare(v, now) <= 0 && (seen ? compare(v, seen) > 0 : v === now))
  if (!seen || compare(now, seen) > 0) {
    versions.value = fresh
    open.value = fresh.length > 0
  }
})
const title = computed(() => versions.value.length > 1
  ? `Mise à jour : ${versions.value.length} nouvelles versions`
  : `Nouveautés de la version ${current.value ?? ''}`)
</script>

<template>
  <UModal v-model:open="open" :title="title" description="Ce qui a changé depuis ta dernière ouverture de Twitch Desk" :ui="{ content: 'max-w-3xl' }">
    <template #body>
      <ChangelogView :current="current" :only="versions" />
    </template>
    <template #footer>
      <div class="flex justify-end w-full">
        <UButton label="Compris" @click="open = false" />
      </div>
    </template>
  </UModal>
</template>
