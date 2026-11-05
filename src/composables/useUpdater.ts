import { ref, shallowRef } from 'vue'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { invoke } from '@tauri-apps/api/core'
import { relaunch } from '@tauri-apps/plugin-process'

// Mise à jour automatique : vérification au démarrage (mise à jour trouvée installée tout de suite)
// puis toutes les 6 h ; installation sans confirmation (réglage) : vérification toutes les N
// minutes, mise à jour installée dès trouvée.
// Le paquet est signé ; l'updater refuse tout fichier qui ne porte pas notre signature.
const CHECK_EVERY = 6 * 3600 * 1000

// shallowRef : l'objet Update de Tauri a des champs privés (#rid) qu'un proxy
// réactif de Vue ne peut pas lire (« Cannot read private member »)
const available = shallowRef<Update | null>(null)
const checking = ref(false)
const installing = ref(false)
const progress = ref(0)
const lastCheck = ref<Date | null>(null)
const error = ref<string | null>(null)
let timer: ReturnType<typeof setInterval> | null = null
// Réglage du minuteur en cours : relancé seulement s'il change
let timerKey = ''

// Messages du module de mise à jour traduits en phrases compréhensibles
export function friendlyUpdateError(e: unknown): string {
  const raw = String(e)
  const m = raw.toLowerCase()
  // latest.json publié par la compilation Linux avant celle de Windows
  if (m.includes('platforms') && (m.includes('fallback') || m.includes('not found') || m.includes('were found'))) {
    return 'Une nouvelle version est en cours de publication, mais celle pour Windows n\'est pas encore prête. Réessaie dans quelques minutes.'
  }
  if (m.includes('signature')) {
    return 'La mise à jour téléchargée n\'a pas une signature valide : elle n\'a pas été installée.'
  }
  if (m.includes('successful status code') || m.includes('status code')) {
    return 'Le serveur des mises à jour ne répond pas pour l\'instant. Réessaie plus tard.'
  }
  if (m.includes('error sending request') || m.includes('dns') || m.includes('timed out') || m.includes('connect')) {
    return 'Impossible de joindre GitHub pour vérifier les mises à jour. Vérifie ta connexion à internet.'
  }
  return raw
}

async function checkNow(): Promise<boolean> {
  if (checking.value || installing.value) return available.value !== null
  checking.value = true
  error.value = null
  try {
    available.value = await check()
  } catch (e) {
    error.value = friendlyUpdateError(e)
  } finally {
    checking.value = false
    lastCheck.value = new Date()
  }
  return available.value !== null
}

async function install() {
  const update = available.value
  if (!update || installing.value) return
  installing.value = true
  progress.value = 0
  error.value = null
  let total = 0
  let received = 0
  try {
    // L'installateur ferme l'application lui-même : taille de la fenêtre enregistrée avant
    await invoke('save_window_state', { updating: true }).catch(() => {})
    await update.downloadAndInstall((event) => {
      if (event.event === 'Started') total = event.data.contentLength ?? 0
      if (event.event === 'Progress') {
        received += event.data.chunkLength
        progress.value = total ? Math.round(received / total * 100) : 0
      }
      if (event.event === 'Finished') progress.value = 100
    })
    await relaunch()
  } catch (e) {
    // Installation ratée : l'application ne sera pas relancée par l'installateur
    void invoke('save_window_state', { updating: false }).catch(() => {})
    error.value = friendlyUpdateError(e)
    installing.value = false
  }
}

async function checkAndMaybeInstall(autoInstall: boolean) {
  if (await checkNow() && autoInstall) await install()
}

// `minutes` : intervalle quand `autoInstall` (sinon CHECK_EVERY)
function startAutoCheck(autoInstall = false, minutes = 360) {
  const every = autoInstall ? Math.min(360, Math.max(1, Math.round(minutes))) * 60_000 : CHECK_EVERY
  const key = `${autoInstall}:${every}`
  if (timer && key === timerKey) return
  const first = !timer
  if (timer) clearInterval(timer)
  timerKey = key
  // Premier lancement : vérification tout de suite, installée sans attendre ; installation
  // automatique cochée ensuite : vérification tout de suite aussi
  if (first || autoInstall) void checkAndMaybeInstall(true)
  timer = setInterval(() => void checkAndMaybeInstall(autoInstall), every)
}

function stopAutoCheck() {
  if (timer) clearInterval(timer)
  timer = null
  timerKey = ''
}

export function useUpdater() {
  return { available, checking, installing, progress, lastCheck, error, checkNow, install, startAutoCheck, stopAutoCheck }
}
