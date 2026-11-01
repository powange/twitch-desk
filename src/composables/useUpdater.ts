import { ref, shallowRef } from 'vue'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'

// Mise à jour automatique : vérification au démarrage puis toutes les 6 h.
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

async function checkNow(): Promise<boolean> {
  if (checking.value || installing.value) return available.value !== null
  checking.value = true
  error.value = null
  try {
    available.value = await check()
  } catch (e) {
    error.value = String(e)
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
    error.value = String(e)
    installing.value = false
  }
}

function startAutoCheck() {
  if (timer) return
  void checkNow()
  timer = setInterval(() => void checkNow(), CHECK_EVERY)
}

function stopAutoCheck() {
  if (timer) clearInterval(timer)
  timer = null
}

export function useUpdater() {
  return { available, checking, installing, progress, lastCheck, error, checkNow, install, startAutoCheck, stopAutoCheck }
}
