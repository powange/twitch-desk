import { computed, shallowRef, type ShallowRef } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { GiveawaysView, LogEvent, Snapshot } from '../types'

// État poussé par le cœur Rust. Lecture complète au démarrage (get_snapshot, avec sa
// version), puis mises à jour partielles (« snapshot-patch », au plus 4 fois par seconde) :
// seules les parties changées arrivent, chacune dans sa propre référence. Lire
// `snapshot.value.giveaways.giveaways` ne suit donc que cette partie-là.
// Un seul abonnement pour toute l'application.

type TopKey = Exclude<keyof Snapshot, 'giveaways'>
type GiveawaysKey = keyof GiveawaysView

const TOP_KEYS: TopKey[] = ['paused', 'accounts', 'events', 'operationsVersion', 'operationsUpdated', 'languages', 'scanner', 'insights', 'sync']
const GIVEAWAYS_KEYS: GiveawaysKey[] = ['channels', 'games', 'giveaways', 'mentions', 'botReplies']

interface SnapshotPatch {
  version: number
  // Clés de premier niveau de Snapshot, ou « giveaways.<clé> »
  domains: Record<string, unknown>
  // Nouveaux événements, les plus récents d'abord, à placer en tête du fil
  eventsAppend?: LogEvent[]
  eventsMax?: number
}

const top = Object.fromEntries(TOP_KEYS.map(k => [k, shallowRef()])) as { [K in TopKey]: ShallowRef<Snapshot[K]> }
const giveawaysParts = Object.fromEntries(GIVEAWAYS_KEYS.map(k => [k, shallowRef()])) as { [K in GiveawaysKey]: ShallowRef<GiveawaysView[K]> }

// Objets stables dont chaque champ lit sa référence : pas de dépendance à tout l'état
const giveawaysView = {} as GiveawaysView
for (const k of GIVEAWAYS_KEYS) Object.defineProperty(giveawaysView, k, { enumerable: true, get: () => giveawaysParts[k].value })
const state = {} as Snapshot
for (const k of TOP_KEYS) Object.defineProperty(state, k, { enumerable: true, get: () => top[k].value })
Object.defineProperty(state, 'giveaways', { enumerable: true, get: () => giveawaysView })

const ready = shallowRef(false)
const snapshot = computed<Snapshot | null>(() => (ready.value ? state : null))

// Version de l'état affiché ; -1 : lecture complète en cours (mises à jour mises de côté)
let version = -1
let pending: SnapshotPatch[] = []
let loading = false
let started = false

function setPart(key: string, value: unknown) {
  if (key.startsWith('giveaways.')) {
    const sub = key.slice('giveaways.'.length) as GiveawaysKey
    if (sub in giveawaysParts) giveawaysParts[sub].value = value as never
  } else if (key in top) {
    top[key as TopKey].value = value as never
  }
}

function applyPatch(p: SnapshotPatch) {
  for (const [key, value] of Object.entries(p.domains)) setPart(key, value)
  if (p.eventsAppend?.length) {
    top.events.value = [...p.eventsAppend, ...(top.events.value ?? [])].slice(0, p.eventsMax ?? 300)
  }
  version = p.version
}

function onPatch(p: SnapshotPatch) {
  if (version < 0) {
    pending.push(p)
    return
  }
  // Déjà contenue dans l'état lu, ou arrivée en retard
  if (p.version <= version) return
  // Mise à jour manquée : l'état n'est plus fiable, tout est relu
  if (p.version !== version + 1) {
    void reload()
    return
  }
  applyPatch(p)
}

async function reload() {
  if (loading) return
  loading = true
  version = -1
  try {
    const full = await invoke<Snapshot & { version: number }>('get_snapshot')
    for (const k of TOP_KEYS) top[k].value = full[k] as never
    for (const k of GIVEAWAYS_KEYS) giveawaysParts[k].value = full.giveaways[k] as never
    version = full.version
    ready.value = true
    loading = false
    // Mises à jour arrivées pendant la lecture : seules les plus récentes qu'elle s'appliquent
    const queued = pending.sort((a, b) => a.version - b.version)
    pending = []
    for (const p of queued) onPatch(p)
  } catch (e) {
    loading = false
    console.error('état initial illisible', e)
    setTimeout(() => void reload(), 2000)
  }
}

async function start() {
  if (started) return
  started = true
  try {
    await listen<SnapshotPatch>('snapshot-patch', e => onPatch(e.payload))
  } catch (e) {
    console.error('abonnement à l’état impossible', e)
  }
  await reload()
}

export function useSnapshot() {
  void start()
  const accounts = computed(() => snapshot.value?.accounts ?? [])
  const events = computed(() => snapshot.value?.events ?? [])
  const paused = computed(() => snapshot.value?.paused ?? false)
  const channels = computed(() => accounts.value.flatMap(a => a.channels.map(c => ({ account: a, channel: c }))))
  return { snapshot, accounts, events, paused, channels }
}
