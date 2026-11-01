import { effectScope, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { Settings } from '../types'

// Réglages de l'application, partagés par toutes les pages : chargés une fois (relus à
// l'ouverture d'une page s'il n'y a rien en attente), enregistrés 400 ms après la dernière
// modification (un curseur qu'on fait glisser n'envoie qu'un enregistrement).
const SAVE_DELAY = 400

const settings = ref<Settings | null>(null)
const loading = ref(false)
const saving = ref(false)
let loadPromise: Promise<void> | null = null
let saveTimer: ReturnType<typeof setTimeout> | null = null
// Fonctions appelées après chaque enregistrement réussi (mise à jour automatique…)
const afterSave = new Set<(s: Settings) => void>()
let toast: ReturnType<typeof useToast> | null = null

async function load(): Promise<void> {
  if (loadPromise) return loadPromise
  loading.value = true
  loadPromise = (async () => {
    try {
      // Réponse ignorée si une modification est arrivée pendant la lecture (elle sera enregistrée)
      const fresh = await invoke<Settings>('get_settings')
      if (!saveTimer && !saving.value) settings.value = fresh
    } catch (e) {
      toast?.add({ title: 'Réglages non chargés', description: String(e), color: 'error' })
    } finally {
      loading.value = false
      loadPromise = null
    }
  })()
  return loadPromise
}

async function saveNow() {
  if (saveTimer) clearTimeout(saveTimer)
  saveTimer = null
  const value = settings.value
  if (!value) return
  saving.value = true
  try {
    await invoke('save_settings', { settings: value })
    for (const f of afterSave) f(value)
  } catch (e) {
    toast?.add({ title: 'Réglage non enregistré', description: String(e), color: 'error' })
  } finally {
    saving.value = false
  }
}

function scheduleSave() {
  if (saveTimer) clearTimeout(saveTimer)
  saveTimer = setTimeout(() => void saveNow(), SAVE_DELAY)
}

// Surveillance hors de toute page : elle continue quand la page qui l'a créée se ferme
let watching = false
function startWatching() {
  if (watching) return
  watching = true
  effectScope(true).run(() => {
    // Objet remplacé : réglages (re)chargés, rien à enregistrer ; modifié sur place : enregistré
    watch(settings, (value, old) => {
      if (value && old && value === old) scheduleSave()
    }, { deep: true })
  })
}

export function useSettings() {
  toast ??= useToast()
  startWatching()
  // Relus à chaque ouverture de page, sauf modification en attente d'enregistrement
  if (!saveTimer && !saving.value) void load()
  return { settings, loading, saving, reload: load, saveNow, afterSave }
}
