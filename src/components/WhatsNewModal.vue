<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { CHANGELOG } from '../changelog'
import ChangelogView from './ChangelogView.vue'

// Premier lancement après une mise à jour : changelog des versions arrivées depuis la
// dernière vue sur ce poste (plusieurs si on en a sauté), une seule fois
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

function remember(v: string) {
  try {
    localStorage.setItem(KEY, v)
  } catch {
    // Stockage indisponible : la fenêtre pourra revenir au prochain lancement
  }
}

onMounted(async () => {
  const now = await getVersion()
  current.value = now
  let seen: string | null = null
  try {
    seen = localStorage.getItem(KEY)
  } catch {
    return
  }
  // Rien de retenu (avant la v0.48.12 ou première installation) : seulement la version installée
  const fresh = CHANGELOG
    .map(r => r.version)
    .filter(v => compare(v, now) <= 0 && (seen ? compare(v, seen) > 0 : v === now))
  remember(now)
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
