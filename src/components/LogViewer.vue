<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { save as saveDialog } from '@tauri-apps/plugin-dialog'

// Journaux de l'application (un fichier par jour, 7 jours) : lecture et téléchargement
interface LogFile { name: string, size: number, modified: string | null }
type Level = 'all' | 'info' | 'warn' | 'error'

const toast = useToast()
const files = ref<LogFile[]>([])
const selected = ref<string | undefined>(undefined)
const text = ref('')
const truncated = ref(false)
const loading = ref(false)
const level = ref<Level>('all')
const search = ref('')
const viewer = ref<HTMLElement | null>(null)

const LEVELS = [
  { label: 'Tout', value: 'all' },
  { label: 'Infos et plus', value: 'info' },
  { label: 'Avertissements et erreurs', value: 'warn' },
  { label: 'Erreurs', value: 'error' }
]
const RANK: Record<string, number> = { TRACE: 0, DEBUG: 1, INFO: 2, WARN: 3, ERROR: 4 }
const MIN_RANK: Record<Level, number> = { all: 0, info: 2, warn: 3, error: 4 }
const MAX_LINES = 3000

function sizeLabel(bytes: number) {
  if (bytes < 1024) return `${bytes} o`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} Ko`
  return `${(bytes / 1024 / 1024).toFixed(1)} Mo`
}
// « twitch-desk.2026-11-01.log » → « 1 nov. 2026 »
function dayLabel(name: string) {
  const m = name.match(/(\d{4}-\d{2}-\d{2})/)
  if (!m) return name
  return new Date(`${m[1]}T12:00:00`).toLocaleDateString('fr-FR', { day: 'numeric', month: 'short', year: 'numeric' })
}
const fileItems = computed(() => files.value.map(f => ({ label: `${dayLabel(f.name)} · ${sizeLabel(f.size)}`, value: f.name })))

async function loadFiles() {
  try {
    files.value = await invoke<LogFile[]>('list_logs')
    if (!selected.value || !files.value.some(f => f.name === selected.value)) selected.value = files.value[0]?.name
  } catch (e) {
    toast.add({ title: 'Journaux illisibles', description: String(e), color: 'error' })
  }
}

async function loadContent() {
  if (!selected.value) {
    text.value = ''
    return
  }
  loading.value = true
  try {
    const content = await invoke<{ text: string, truncated: boolean }>('read_log', { name: selected.value })
    text.value = content.text
    truncated.value = content.truncated
    // Les lignes les plus récentes sont en bas
    await nextTick()
    if (viewer.value) viewer.value.scrollTop = viewer.value.scrollHeight
  } catch (e) {
    toast.add({ title: 'Journal illisible', description: String(e), color: 'error' })
  } finally {
    loading.value = false
  }
}

async function refresh() {
  await loadFiles()
  await loadContent()
}

onMounted(refresh)
watch(selected, loadContent)

// Lignes : horodatage, niveau, source puis message ; une ligne sans niveau prolonge la précédente
interface Line { text: string, level: string }
const lines = computed<Line[]>(() => {
  const out: Line[] = []
  let current = 'INFO'
  for (const raw of text.value.split('\n')) {
    if (!raw) continue
    const m = raw.match(/^\S+\s+(TRACE|DEBUG|INFO|WARN|ERROR)\s/)
    if (m?.[1]) current = m[1]
    out.push({ text: raw, level: current })
  }
  return out
})
const filtered = computed(() => {
  const min = MIN_RANK[level.value]
  const q = search.value.trim().toLowerCase()
  const list = lines.value.filter(l => (RANK[l.level] ?? 2) >= min && (!q || l.text.toLowerCase().includes(q)))
  return list.slice(-MAX_LINES)
})
const LEVEL_CLASS: Record<string, string> = {
  ERROR: 'text-error',
  WARN: 'text-warning',
  DEBUG: 'text-muted',
  TRACE: 'text-dimmed'
}

async function download() {
  if (!selected.value) return
  const path = await saveDialog({ defaultPath: selected.value, filters: [{ name: 'Journal', extensions: ['log', 'txt'] }] })
  if (!path) return
  try {
    await invoke('export_log', { name: selected.value, path })
    toast.add({ title: 'Journal téléchargé', description: path, color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Téléchargement impossible', description: String(e), color: 'error' })
  }
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex items-center gap-2 flex-wrap">
      <USelect v-model="selected" :items="fileItems" placeholder="Aucun journal" class="w-56" aria-label="Jour du journal" />
      <USelect v-model="level" :items="LEVELS" class="w-56" aria-label="Niveau" />
      <UInput id="log-search" v-model="search" icon="i-lucide-search" placeholder="Rechercher" class="w-48" />
      <span class="flex-1" />
      <UButton icon="i-lucide-refresh-cw" color="neutral" variant="soft" :loading="loading" aria-label="Actualiser" title="Actualiser" @click="refresh" />
      <UButton label="Télécharger" icon="i-lucide-download" color="neutral" variant="soft" :disabled="!selected" @click="download" />
    </div>
    <p v-if="truncated" class="text-xs text-muted">Journal volumineux : seule la fin (2 Mo) est affichée. Le téléchargement contient tout le fichier.</p>
    <div
      ref="viewer"
      class="h-[60vh] overflow-auto rounded-lg border border-default bg-elevated/40 p-3 font-mono text-xs leading-relaxed"
    >
      <div v-for="(l, i) in filtered" :key="i" class="whitespace-pre-wrap break-all" :class="LEVEL_CLASS[l.level] ?? ''">{{ l.text }}</div>
      <p v-if="filtered.length === 0" class="text-muted font-sans text-sm">
        {{ selected ? 'Aucune ligne ne correspond.' : 'Aucun journal pour l\'instant.' }}
      </p>
    </div>
    <p class="text-xs text-muted">
      {{ filtered.length }} ligne(s) affichée(s){{ filtered.length === MAX_LINES ? ` (les ${MAX_LINES} dernières)` : '' }}. Un fichier par jour, 7 jours conservés.
      Avant de partager un journal, vérifie qu'il ne contient rien de personnel.
    </p>
  </div>
</template>
