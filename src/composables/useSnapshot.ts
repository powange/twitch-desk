import { computed, shallowRef } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { Snapshot } from '../types'

// État complet poussé par le cœur Rust (événement « snapshot », au plus 4 fois par seconde).
// Un seul abonnement pour toute l'application.
const snapshot = shallowRef<Snapshot | null>(null)
let started = false

async function start() {
  if (started) return
  started = true
  // Un événement arrivé pendant la lecture initiale est plus récent qu'elle : il est gardé
  let gotEvent = false
  try {
    await listen<Snapshot>('snapshot', (e) => {
      gotEvent = true
      snapshot.value = e.payload
    })
    const first = await invoke<Snapshot>('get_snapshot')
    if (!gotEvent) snapshot.value = first
  } catch (e) {
    // Les événements suivants rempliront l'état
    console.error('état initial illisible', e)
  }
}

export function useSnapshot() {
  void start()
  const accounts = computed(() => snapshot.value?.accounts ?? [])
  const events = computed(() => snapshot.value?.events ?? [])
  const paused = computed(() => snapshot.value?.paused ?? false)
  const channels = computed(() => accounts.value.flatMap(a => a.channels.map(c => ({ account: a, channel: c }))))
  return { snapshot, accounts, events, paused, channels }
}
