<script setup lang="ts">
import ChannelCell from '../../components/ChannelCell.vue'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../../composables/useSnapshot'
import { useOpenExternal } from '../../utils/external'
import { useWatchStream } from '../../utils/viewer'
import { boxArt, formatNumber, timeAgo } from '../../utils/format'
import { LANGUAGES } from '../../utils/languages'
import { FAVORITE_ROW, hasFavoriteGift, suggestGifts, useGiftCatalog } from '../../composables/useGiftCatalog'
import { GIVEAWAY_KIND } from '../../utils/giveaways'
import type { Gift, ScanMatch } from '../../types'
import GiftBadge from '../../components/GiftBadge.vue'
import { useSettings } from '../../composables/useSettings'
import { isBoolean, oneOf, usePersisted } from '../../composables/usePersisted'

const { snapshot } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()
const watchStream = useWatchStream()

const scanner = computed(() => snapshot.value?.scanner)
const { gifts } = useGiftCatalog()

// Cadeaux du catalogue repérés : titre du live, description de la chaîne, message épinglé,
// et titre et description du giveaway StreamElements actif
// Calculés une fois par mise à jour pour chaque stream (affichage et tri)
const giftsByMatch = computed(() => new Map((scanner.value?.matches ?? []).map(m => [
  m,
  suggestGifts(gifts.value, [m.title, m.description, m.pinnedMessage, m.giveawayText].filter(Boolean).join('\n'))
])))
const giftsOf = (m: ScanMatch): Gift[] => giftsByMatch.value.get(m) ?? []
const games = computed(() => snapshot.value?.giveaways.games ?? [])
// Choix d'affichage retenus sur ce poste
const showIgnored = usePersisted('scanner.showIgnored', false, { sanitize: isBoolean })
// Tri par colonne (le titre n'en a pas) : spectateurs décroissants par défaut
const SORT_KEYS = ['channel', 'game', 'type', 'gifts', 'viewers', 'live', 'status'] as const
type SortKey = typeof SORT_KEYS[number]
const sortKey = usePersisted<SortKey>('scanner.sortKey', 'viewers', { sanitize: oneOf(SORT_KEYS) })
const sortDesc = usePersisted('scanner.sortDesc', true, { sanitize: isBoolean })
// Premier clic : du plus grand au plus petit pour les nombres, de A à Z pour les textes
const DESC_FIRST: SortKey[] = ['gifts', 'viewers', 'live']
function sortBy(key: SortKey) {
  if (sortKey.value === key) sortDesc.value = !sortDesc.value
  else {
    sortKey.value = key
    sortDesc.value = DESC_FIRST.includes(key)
  }
}
function sortIcon(key: SortKey) {
  if (sortKey.value !== key) return 'i-lucide-chevrons-up-down'
  return sortDesc.value ? 'i-lucide-chevron-down' : 'i-lucide-chevron-up'
}
const STATUS_ORDER = (m: ScanMatch) => m.monitored ? 2 : m.ignored ? 1 : 0
function sortValue(m: ScanMatch, key: SortKey): string | number {
  switch (key) {
    case 'channel': return (m.displayName || m.login).toLowerCase()
    case 'game': return m.gameName.toLowerCase()
    // Les types inconnus après les types connus
    case 'type': return m.giveawayKind ? GIVEAWAY_KIND[m.giveawayKind].label.toLowerCase() : '\uffff'
    case 'gifts': return giftsOf(m).length
    case 'viewers': return m.viewers
    // Durée du live : début le plus ancien = live le plus long
    case 'live': return m.startedAt ? -new Date(m.startedAt).getTime() : -Infinity
    case 'status': return STATUS_ORDER(m)
  }
}
const matches = computed(() => {
  // Stream ignoré mis en avant par une notification : montré cette fois, sans changer le réglage
  const list = (scanner.value?.matches ?? []).filter(m => showIgnored.value || !m.ignored || m.login === pinned.value)
  const dir = sortDesc.value ? -1 : 1
  return [...list].sort((a, b) => {
    const x = sortValue(a, sortKey.value)
    const y = sortValue(b, sortKey.value)
    const c = typeof x === 'string' && typeof y === 'string' ? x.localeCompare(y, 'fr') : (x as number) - (y as number)
    // À égalité, les plus regardés d'abord
    return c * dir || b.viewers - a.viewers
  })
})
const ignoredCount = computed(() => (scanner.value?.matches ?? []).filter(m => m.ignored).length)

// Clic sur une notification du scanner (?highlight=<chaîne>) : stream montré et mis en avant
const route = useRoute()
const router = useRouter()
const highlighted = ref<string | null>(null)
// Stream ignoré affiché le temps de cette visite (notification touchée)
const pinned = ref<string | null>(null)
let highlightTimer: ReturnType<typeof setTimeout> | null = null
async function highlight(login: string) {
  const m = (scanner.value?.matches ?? []).find(x => x.login === login)
  if (!m) return false
  if (m.ignored) pinned.value = login
  highlighted.value = login
  await nextTick()
  document.getElementById(`scan-${login}`)?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  if (highlightTimer) clearTimeout(highlightTimer)
  highlightTimer = setTimeout(() => { highlighted.value = null }, 8000)
  return true
}
// Demande gardée jusqu'à ce que le stream soit dans la liste (scanner pas encore relu)
const wanted = computed(() => typeof route.query.highlight === 'string' ? route.query.highlight.toLowerCase() : null)
watch([wanted, () => scanner.value?.matches], async ([login]) => {
  if (login && await highlight(login)) void router.replace({ query: {} })
}, { immediate: true })
onBeforeUnmount(() => { if (highlightTimer) clearTimeout(highlightTimer) })

// Rafraîchit les « il y a … » et les durées de live
const now = ref(Date.now())
let clock: ReturnType<typeof setInterval> | null = null
onMounted(() => { clock = setInterval(() => { now.value = Date.now() }, 30_000) })
onBeforeUnmount(() => { if (clock) clearInterval(clock) })

// --- Réglages (partagés, enregistrés peu après chaque modification) ---
const { settings, afterSave } = useSettings()
onMounted(() => {
  // Premier passage sur la page : un scan tout de suite
  if (!scanner.value?.scannedAt && games.value.length) void scanNow()
})
// Chaque filtre enregistré : les résultats sont refaits sur les streams déjà relevés
const refilter = () => { void invoke('scanner_refilter').catch(() => {}) }
afterSave.add(refilter)
onBeforeUnmount(() => afterSave.delete(refilter))

function gameOn(gameId: string) {
  return !settings.value?.scannerGamesOff.includes(gameId)
}
function toggleGame(gameId: string) {
  if (!settings.value) return
  const off = settings.value.scannerGamesOff
  settings.value.scannerGamesOff = off.includes(gameId) ? off.filter(x => x !== gameId) : [...off, gameId]
}

// Mots-clés : cocher ou décocher sans perdre la liste, édition de la liste dans une fenêtre
function keywordOn(k: string) {
  return !settings.value?.scannerKeywordsOff.includes(k)
}
function toggleKeyword(k: string) {
  if (!settings.value) return
  const off = settings.value.scannerKeywordsOff
  settings.value.scannerKeywordsOff = off.includes(k) ? off.filter(x => x !== k) : [...off, k]
}
const editKeywords = ref(false)
const keywordInput = ref('')
function addKeyword() {
  if (!settings.value) return
  for (const raw of keywordInput.value.split(',')) {
    const k = raw.trim().toLowerCase()
    if (k && !settings.value.scannerKeywords.includes(k)) settings.value.scannerKeywords.push(k)
  }
  keywordInput.value = ''
}
function removeKeyword(k: string) {
  if (!settings.value) return
  settings.value.scannerKeywords = settings.value.scannerKeywords.filter(x => x !== k)
  settings.value.scannerKeywordsOff = settings.value.scannerKeywordsOff.filter(x => x !== k)
}

const languageItems = Object.entries(LANGUAGES).map(([code, l]) => ({ label: l.name, value: code })).sort((a, b) => a.label.localeCompare(b.label))

// --- Actions ---
async function scanNow() {
  try {
    await invoke('scan_streams')
  } catch (e) {
    toast.add({ title: 'Scan impossible', description: String(e), color: 'error' })
  }
}

const adding = ref<string | null>(null)
async function monitor(m: ScanMatch) {
  adding.value = m.login
  try {
    await invoke('add_giveaway_channel', { login: m.login })
    toast.add({ title: `${m.displayName || m.login} surveillée`, color: 'success', icon: 'i-lucide-check' })
    await invoke('scan_streams')
  } catch (e) {
    toast.add({ title: 'Chaîne non ajoutée', description: String(e), color: 'error' })
  } finally {
    adding.value = null
  }
}

async function ignore(m: ScanMatch, value: boolean) {
  try {
    await invoke('scanner_ignore', { login: m.login, ignore: value })
  } catch (e) {
    toast.add({ title: value ? 'Chaîne non ignorée' : 'Chaîne non rétablie', description: String(e), color: 'error' })
  }
}

// Titre découpé pour surligner les mots-clés trouvés, avec la règle du cœur : début de mot, sans
// tenir compte des majuscules ni des accents, mots d'un mot-clé séparés par n'importe quels
// signes (« free key » surligne « FREE-KEYS », « cle » surligne « CLÉ »). Calculé une fois par
// mise à jour pour chaque stream
const fold = (t: string) => t.toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '')
function titleParts(m: ScanMatch): { text: string, hit: boolean }[] {
  const keys = m.keywords
    .map(k => fold(k).trim().split(/[^\p{L}\p{N}]+/u).filter(Boolean).map(w => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('[^\\p{L}\\p{N}]+'))
    .filter(Boolean)
  if (!keys.length) return [{ text: m.title, hit: false }]
  // Titre replié, et pour chaque position du titre replié celle du titre d'origine
  let folded = ''
  const origin: number[] = []
  let at = 0
  for (const ch of m.title) {
    const f = fold(ch)
    for (let k = 0; k < f.length; k++) origin.push(at)
    folded += f
    at += ch.length
  }
  origin.push(m.title.length)
  const re = new RegExp(`(?<![\\p{L}\\p{N}])(${keys.join('|')})[\\p{L}\\p{N}]*`, 'gu')
  const parts: { text: string, hit: boolean }[] = []
  let last = 0
  for (const match of folded.matchAll(re)) {
    const i = origin[match.index ?? 0]!
    const j = origin[(match.index ?? 0) + match[0].length]!
    if (i > last) parts.push({ text: m.title.slice(last, i), hit: false })
    parts.push({ text: m.title.slice(i, j), hit: true })
    last = j
  }
  if (last < m.title.length) parts.push({ text: m.title.slice(last), hit: false })
  return parts
}
const partsByMatch = computed(() => new Map((scanner.value?.matches ?? []).map(m => [m, titleParts(m)])))
const titleOf = (m: ScanMatch) => partsByMatch.value.get(m) ?? titleParts(m)

function liveFor(iso: string | null) {
  if (!iso) return '—'
  const min = Math.max(0, Math.round((now.value - new Date(iso).getTime()) / 60000))
  const h = Math.floor(min / 60)
  return h ? `${h} h ${String(min % 60).padStart(2, '0')}` : `${min} min`
}
// Vignette d'un cadeau : affichée en grand
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <div class="flex items-center gap-3 flex-wrap">
      <h1 class="text-xl font-semibold flex-1">Scanner</h1>
      <span v-if="scanner?.scannedAt" class="text-sm text-muted">
        Scan {{ timeAgo(scanner.scannedAt, now) }} · {{ formatNumber(scanner.scannedStreams) }} streams parcourus
      </span>
      <UButton label="Scanner maintenant" icon="i-lucide-radar" :loading="scanner?.scanning" :disabled="games.length === 0" @click="scanNow" />
    </div>

    <UAlert
      v-if="games.length === 0"
      icon="i-lucide-info"
      color="warning"
      variant="subtle"
      title="Le scanner parcourt les lives des catégories surveillées : ajoute au moins une catégorie."
      :actions="[{ label: 'Catégories', to: '/giveaways/categories' }]"
    />
    <UAlert v-else-if="scanner?.error" icon="i-lucide-triangle-alert" color="warning" variant="subtle" :title="scanner.error" />

    <!-- Réglages -->
    <div v-if="settings" class="border border-default rounded-lg divide-y divide-default">
      <div class="flex items-center gap-4 px-4 py-3">
        <div class="w-44 shrink-0">
          <div class="font-medium">Catégories</div>
          <div class="text-xs text-muted">Aucune cochée : toutes</div>
        </div>
        <div class="flex items-center gap-2 flex-wrap flex-1">
          <UButton
            v-for="g in games"
            :key="g.id"
            size="xs"
            :color="gameOn(g.gameId) ? 'primary' : 'neutral'"
            :variant="gameOn(g.gameId) ? 'subtle' : 'outline'"
            :class="gameOn(g.gameId) ? '' : 'text-muted'"
            @click="toggleGame(g.gameId)"
          >
            <UIcon :name="gameOn(g.gameId) ? 'i-lucide-square-check' : 'i-lucide-square'" class="size-4" />
            <img :src="boxArt(g.boxArt, g.gameId)" alt="" class="w-4 h-5 rounded-sm object-cover" :class="gameOn(g.gameId) ? '' : 'opacity-40 grayscale'">
            <span :class="gameOn(g.gameId) ? '' : 'line-through'">{{ g.name }}</span>
          </UButton>
          <span v-if="games.length === 0" class="text-sm text-muted">Aucune</span>
          <UButton label="Modifier" size="xs" color="neutral" variant="link" to="/giveaways/categories" />
        </div>
      </div>
      <div class="flex items-center gap-4 px-4 py-3">
        <div class="w-44 shrink-0">
          <div class="font-medium">Mots-clés</div>
          <div class="text-xs text-muted">Cherchés au début des mots du titre. Aucun coché : pas de filtre</div>
        </div>
        <div class="flex items-center gap-1.5 flex-wrap flex-1">
          <UButton
            v-for="k in settings.scannerKeywords"
            :key="k"
            :label="k"
            size="xs"
            :icon="keywordOn(k) ? 'i-lucide-square-check' : 'i-lucide-square'"
            :color="keywordOn(k) ? 'primary' : 'neutral'"
            :variant="keywordOn(k) ? 'subtle' : 'outline'"
            :class="keywordOn(k) ? '' : 'text-muted line-through'"
            @click="toggleKeyword(k)"
          />
          <span v-if="settings.scannerKeywords.length === 0" class="text-sm text-muted">Aucun</span>
          <UButton label="Modifier la liste" icon="i-lucide-pencil" size="xs" color="neutral" variant="link" @click="editKeywords = true" />
        </div>
        <UModal v-model:open="editKeywords" title="Mots-clés du scanner" description="Cherchés au début des mots du titre des lives. Les accents sont ignorés.">
          <template #body>
            <div class="flex flex-col gap-3">
              <form class="flex gap-2" @submit.prevent="addKeyword">
                <UInput id="scanner-keyword" v-model="keywordInput" placeholder="Nouveau mot-clé (plusieurs : séparés par des virgules)" class="flex-1" autofocus />
                <UButton type="submit" label="Ajouter" icon="i-lucide-plus" :disabled="!keywordInput.trim()" />
              </form>
              <ul class="border border-default rounded-lg divide-y divide-default max-h-80 overflow-y-auto">
                <li v-for="k in settings.scannerKeywords" :key="k" class="flex items-center gap-2 px-3 py-1.5">
                  <span class="flex-1" :class="keywordOn(k) ? '' : 'text-muted'">{{ k }}</span>
                  <span v-if="!keywordOn(k)" class="text-xs text-muted">décoché</span>
                  <UButton icon="i-lucide-trash-2" size="xs" color="error" variant="ghost" :aria-label="`Supprimer ${k}`" @click="removeKeyword(k)" />
                </li>
                <li v-if="settings.scannerKeywords.length === 0" class="px-3 py-4 text-center text-sm text-muted">La liste est vide.</li>
              </ul>
            </div>
          </template>
        </UModal>
      </div>
      <div class="flex items-center gap-4 px-4 py-3">
        <div class="w-44 shrink-0 font-medium">Langues</div>
        <USelect v-model="settings.scannerLanguages" :items="languageItems" multiple placeholder="Toutes les langues" class="w-72" />
      </div>
      <div class="flex items-center gap-4 px-4 py-3">
        <div class="w-44 shrink-0 font-medium">Spectateurs minimum</div>
        <UInputNumber id="scanner-min-viewers" v-model="settings.scannerMinViewers" :min="0" :max="100000" size="sm" class="w-32" />
      </div>
      <div class="flex items-center gap-4 px-4 py-3">
        <div class="w-44 shrink-0">
          <div class="font-medium">Scan automatique</div>
          <div class="text-xs text-muted">Toutes les 3 min</div>
        </div>
        <USwitch v-model="settings.scannerAuto" aria-label="Scan automatique" />
        <span class="text-sm text-muted">Notification Windows pour chaque nouveau stream repéré, une fois par live.</span>
      </div>
    </div>

    <!-- Résultats -->
    <section class="flex flex-col gap-2">
      <div class="flex items-center gap-3">
        <h2 class="text-sm font-semibold uppercase text-muted tracking-wide flex-1">
          Streams repérés <span v-if="scanner?.matches.length" class="text-primary">({{ matches.length }})</span>
        </h2>
        <UCheckbox v-if="ignoredCount" v-model="showIgnored" :label="`Afficher les ignorés (${ignoredCount})`" />
      </div>
      <div class="border border-default rounded-lg overflow-x-auto">
        <table class="w-full text-sm">
          <thead class="bg-elevated/60 text-xs uppercase text-muted">
            <tr>
              <th class="px-2 py-2"><span class="sr-only">Regarder</span></th>
              <th v-for="c in ([['channel', 'Chaîne'], ['game', 'Catégorie']] as const)" :key="c[0]" class="text-left px-3 py-2">
                <button class="inline-flex items-center gap-1 uppercase hover:text-default" :class="sortKey === c[0] ? 'text-default' : ''" @click="sortBy(c[0])">
                  {{ c[1] }}<UIcon :name="sortIcon(c[0])" class="size-3.5" />
                </button>
              </th>
              <th class="text-left px-3 py-2">Titre</th>
              <th v-for="c in ([['type', 'Type', 'left'], ['gifts', 'Cadeaux', 'left'], ['viewers', 'Spectateurs', 'right'], ['live', 'En live depuis', 'right'], ['status', 'Statut', 'left']] as const)" :key="c[0]" class="px-3 py-2" :class="c[2] === 'right' ? 'text-right' : 'text-left'">
                <button class="inline-flex items-center gap-1 uppercase whitespace-nowrap hover:text-default" :class="sortKey === c[0] ? 'text-default' : ''" @click="sortBy(c[0])">
                  {{ c[1] }}<UIcon :name="sortIcon(c[0])" class="size-3.5" />
                </button>
              </th>
              <th class="px-3 py-2" />
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="m in matches"
              :id="`scan-${m.login}`"
              :key="m.login"
              class="border-t border-default transition-colors duration-700"
              :class="[m.ignored ? 'opacity-50' : '', highlighted === m.login ? 'bg-primary/15 outline-2 outline-primary -outline-offset-2' : hasFavoriteGift(giftsOf(m)) ? FAVORITE_ROW : '']"
            >
              <td class="px-2 py-2">
                <UButton icon="i-lucide-tv" size="xs" color="neutral" variant="ghost" :aria-label="`Regarder ${m.login}`" title="Regarder (fenêtre multi-stream)" @click="watchStream(m.login)" />
              </td>
              <td class="px-3 py-2">
                <ChannelCell :login="m.login" :name="m.displayName" :avatar="m.profileImage" :lang="m.language" :show-live="false" />
              </td>
              <td class="px-3 py-2 whitespace-nowrap">
                <span class="inline-flex items-center gap-1.5"><img :src="boxArt(null, m.gameId)" alt="" class="w-4 h-5 rounded-sm object-cover">{{ m.gameName }}</span>
              </td>
              <td class="px-3 py-2 min-w-[300px]">
                <template v-for="(p, i) in titleOf(m)" :key="i"><mark v-if="p.hit" class="bg-primary/20 text-primary rounded px-0.5">{{ p.text }}</mark><template v-else>{{ p.text }}</template></template>
              </td>
              <td class="px-3 py-2 whitespace-nowrap">
                <template v-if="m.giveawayKind">
                  <UBadge :label="GIVEAWAY_KIND[m.giveawayKind].label" :color="GIVEAWAY_KIND[m.giveawayKind].color" variant="subtle" />
                  <div v-if="m.giveawayCommand" class="font-mono text-xs mt-0.5">{{ m.giveawayCommand }}</div>
                </template>
                <UTooltip v-else text="Aucun giveaway StreamElements actif ni giveaway connu : type inconnu">
                  <span class="text-muted">Inconnu</span>
                </UTooltip>
              </td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap items-center gap-1 min-w-36 max-w-80">
                  <GiftBadge v-for="g in giftsOf(m)" :key="g.id" :gift="g" />
                  <span v-if="giftsOf(m).length === 0" class="text-muted">—</span>
                </div>
              </td>
              <td class="px-3 py-2 text-right whitespace-nowrap">
                <span class="inline-flex items-center gap-1 text-error"><UIcon name="i-lucide-eye" class="size-3.5" />{{ formatNumber(m.viewers) }}</span>
              </td>
              <td class="px-3 py-2 text-right whitespace-nowrap text-muted">{{ liveFor(m.startedAt) }}</td>
              <td class="px-3 py-2">
                <UBadge v-if="m.monitored" label="Surveillée" color="success" variant="subtle" class="whitespace-nowrap" />
                <UBadge v-else-if="m.ignored" label="Ignorée" color="neutral" variant="subtle" class="whitespace-nowrap" />
                <UBadge v-else label="Nouvelle" color="primary" variant="subtle" class="whitespace-nowrap" />
              </td>
              <td class="px-3 py-2">
                <div class="flex justify-end gap-1">
                  <UButton v-if="!m.monitored" label="Surveiller" icon="i-lucide-plus" size="xs" :loading="adding === m.login" @click="monitor(m)" />
                  <UButton v-if="!m.monitored && !m.ignored" label="Ignorer" size="xs" color="neutral" variant="ghost" title="Jusqu'à la fin de ce live" @click="ignore(m, true)" />
                  <UButton v-if="m.ignored" label="Rétablir" size="xs" color="neutral" variant="ghost" @click="ignore(m, false)" />
                </div>
              </td>
            </tr>
            <tr v-if="matches.length === 0">
              <td colspan="10" class="px-3 py-8 text-center text-muted">
                {{ scanner?.scanning ? 'Scan en cours…' : scanner?.scannedAt ? 'Aucun stream ne correspond pour l\'instant.' : 'Lance un scan avec « Scanner maintenant ».' }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  </div>
</template>
