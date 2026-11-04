<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../composables/useSnapshot'
import { GIVEAWAY_KIND } from '../utils/giveaways'
import type { GiveawayKind } from '../types'
import { usePersisted } from '../composables/usePersisted'

// Journal du chat d'une chaîne : commandes des spectateurs, nos messages, annonces de gagnant
// et réponses des bots aux commandes d'information (!giveaway…), gardés 14 jours. Découpé par
// giveaway : un message va au premier giveaway de la chaîne fini après lui (dernier gagnant
// annoncé, sinon clôture), sinon au giveaway en cours
interface ChatLogEntry { id: number, channel: string, at: string, author: string, kind: Kind, command: string | null, message: string }
interface ChatLogSection {
  id: number
  kind: GiveawayKind
  command: string | null
  status: 'open' | 'paused' | 'closed'
  detectedAt: string
  endedAt: string | null
  winners: string[]
  gifts: string[]
}
interface BotReply { channel: string, command: string, bot: string, message: string, askedBy: string, updatedAt: string, seenAt: string }
type Kind = 'command' | 'own' | 'streamer' | 'bot' | 'winner' | 'reply'

// `giveawayId` : journal ouvert sur la section de ce giveaway
const props = defineProps<{ channel: string | null, giveawayId?: number | null }>()
const open = defineModel<boolean>('open', { required: true })
const toast = useToast()

const KINDS: Record<Kind, { label: string, color: 'neutral' | 'primary' | 'success' | 'info' | 'warning' }> = {
  command: { label: 'Commande', color: 'neutral' },
  own: { label: 'Mon message', color: 'primary' },
  streamer: { label: 'Streamer', color: 'warning' },
  bot: { label: 'Bot', color: 'neutral' },
  winner: { label: 'Gagnant', color: 'success' },
  reply: { label: 'Réponse de bot', color: 'info' }
}
// Filtres à choix multiples ; rien de choisi : tout
const kindItems = [
  { label: 'Commandes', value: 'command' },
  { label: 'Mes messages', value: 'own' },
  { label: 'Messages du streamer', value: 'streamer' },
  { label: 'Messages des bots', value: 'bot' },
  { label: 'Gagnants', value: 'winner' },
  { label: 'Réponses de bots', value: 'reply' }
]

const entries = ref<ChatLogEntry[]>([])
const commands = ref<[string, number][]>([])
const replies = ref<BotReply[]>([])
const sections = ref<ChatLogSection[]>([])
// Ouvert sur un giveaway fini : messages jusqu'à sa fin (les plus récents sur demande)
const beforeAt = ref<string | null>(null)
// Types de message retenus sur ce poste, pour toutes les chaînes ; les commandes non : elles
// changent d'un giveaway à l'autre (un filtre « !join » retenu cacherait le « !gw » suivant)
const kinds = usePersisted('chatLog.kinds', [] as Kind[], {
  sanitize: v => Array.isArray(v) ? v.filter((k): k is Kind => typeof k === 'string' && k in KINDS) : undefined
})
const selectedCommands = ref<string[]>([])
const author = ref('')
// Mots-clés cherchés dans les messages (tous présents)
const text = ref('')
const loading = ref(false)
const more = ref(false)
const PAGE = 200

const commandItems = computed(() => commands.value.map(([c, n]) => ({ label: `${c} (${n})`, value: c })))

// Chaque lecture porte un numéro : seule la plus récente est affichée (une réponse lente
// d'anciens filtres n'écrase pas la liste)
let loadSeq = 0
// Filtres de la dernière lecture demandée
let requestedKey = ''
const filterKey = () => JSON.stringify([props.channel, kinds.value, selectedCommands.value, author.value.trim(), text.value.trim(), beforeAt.value])

async function load(append = false) {
  if (!props.channel) return
  const key = filterKey()
  // Filtres changés depuis la dernière lecture : « plus anciens » repart du début
  if (append && key !== requestedKey) append = false
  requestedKey = key
  const seq = ++loadSeq
  loading.value = true
  try {
    const before = append ? entries.value.at(-1)?.id ?? null : null
    const page = await invoke<ChatLogEntry[]>('get_chat_log', {
      channel: props.channel,
      kinds: kinds.value,
      commands: selectedCommands.value,
      author: author.value.trim() || null,
      text: text.value.trim() || null,
      beforeId: before,
      beforeAt: beforeAt.value
    })
    if (seq !== loadSeq) return
    // Surlignage d'après la recherche lancée (pas à chaque lettre tapée)
    searched.value = text.value.trim()
    entries.value = append ? [...entries.value, ...page] : page
    more.value = page.length === PAGE
  } catch (e) {
    if (seq === loadSeq) toast.add({ title: 'Journal illisible', description: String(e), color: 'error' })
  } finally {
    if (seq === loadSeq) loading.value = false
  }
}
// Filtre modifié : relu seulement s'il diffère de la dernière lecture demandée
function reloadIfChanged() {
  if (open.value && filterKey() !== requestedKey) void load()
}

async function refresh() {
  if (!props.channel) return
  try {
    const meta = await invoke<{ commands: [string, number][], replies: BotReply[] }>('get_chat_log_meta', { channel: props.channel })
    commands.value = meta.commands
    replies.value = meta.replies
  } catch {
    // Les filtres restent ceux d'avant
  }
  try {
    sections.value = await invoke<ChatLogSection[]>('get_chat_log_sections', { channel: props.channel })
  } catch {
    // Journal affiché sans découpage
  }
  await load()
}

// Heures enregistrées à la seconde : un message de la même seconde que la fin en fait partie
const SAME_MOMENT_MS = 2000
// Giveaways finis, du plus ancien au plus récent ; en cours (ouvert ou en pause) : le plus récent
const ended = computed(() => sections.value.filter(s => s.endedAt).map(s => ({ s, end: Date.parse(s.endedAt!) })).sort((a, b) => a.end - b.end))
const current = computed(() => sections.value.find(s => !s.endedAt) ?? null)
function sectionOf(e: ChatLogEntry): ChatLogSection | null {
  const at = Date.parse(e.at)
  return ended.value.find(x => at <= x.end + SAME_MOMENT_MS)?.s ?? current.value
}
// Messages (les plus récents d'abord) regroupés par giveaway, dans le même ordre
const groups = computed(() => {
  const list: { key: string, section: ChatLogSection | null, entries: ChatLogEntry[] }[] = []
  for (const e of entries.value) {
    const s = sectionOf(e)
    const key = s ? `g${s.id}` : 'none'
    const last = list.at(-1)
    if (last?.key === key) last.entries.push(e)
    else list.push({ key, section: s, entries: [e] })
  }
  return list
})
// Fin du giveaway précédent : début de la section
function startOf(s: ChatLogSection): string | null {
  const end = s.endedAt ? Date.parse(s.endedAt) : Infinity
  const before = ended.value.filter(x => x.end < end).at(-1)
  return before ? before.s.endedAt : null
}
// « du 3 nov. 20:10:00 au 21:30:12 (tirage) », « depuis … · en cours »
function periodOf(s: ChatLogSection) {
  const start = startOf(s)
  const from = start ? `du ${time(start)}` : ''
  if (!s.endedAt) return `${start ? `depuis le ${time(start)} · ` : ''}${s.status === 'paused' ? 'en pause' : 'en cours'}`
  const end = `${time(s.endedAt)} (${s.winners.length ? 'tirage' : 'clos'})`
  return start ? `${from} au ${end}` : `jusqu'au ${end}`
}
function sectionTitle(s: ChatLogSection) {
  return `${GIVEAWAY_KIND[s.kind]?.label ?? s.kind}${s.command ? ` ${s.command}` : ''}`
}

// Section du giveaway demandé, montrée une fois le journal lu
async function showSection() {
  if (!props.giveawayId) return
  await nextTick()
  document.getElementById(`chat-log-g${props.giveawayId}`)?.scrollIntoView({ block: 'start' })
}
function showRecent() {
  beforeAt.value = null
  void load()
}

watch(open, (isOpen) => {
  if (!isOpen) return
  if (authorTimer) clearTimeout(authorTimer)
  // Réponses d'une ouverture précédente ignorées
  loadSeq++
  selectedCommands.value = []
  author.value = ''
  text.value = ''
  searched.value = ''
  entries.value = []
  sections.value = []
  more.value = false
  beforeAt.value = null
  // Filtres remis à zéro (types de message retenus gardés) : leurs observateurs ne relancent
  // pas de lecture, refresh() en fait une seule
  requestedKey = filterKey()
  void refresh().then(async () => {
    // Giveaway fini : messages jusqu'à sa fin, sa section en tête
    const s = sections.value.find(x => x.id === props.giveawayId)
    if (s?.endedAt) {
      beforeAt.value = new Date(Date.parse(s.endedAt) + SAME_MOMENT_MS).toISOString()
      await load()
    }
    await showSection()
  })
})
watch([kinds, selectedCommands], reloadIfChanged, { deep: true })
// Clic sur une commande du tableau : ajoutée au filtre, ou retirée
function toggleCommand(c: string) {
  selectedCommands.value = selectedCommands.value.includes(c) ? selectedCommands.value.filter(x => x !== c) : [...selectedCommands.value, c]
}
// Recherche par auteur : relue une fois la frappe terminée
let authorTimer: ReturnType<typeof setTimeout> | null = null
watch([author, text], () => {
  if (authorTimer) clearTimeout(authorTimer)
  authorTimer = setTimeout(reloadIfChanged, 300)
})
// Message découpé pour surligner les mots-clés cherchés : expression calculée une fois par
// recherche lancée
const searched = ref('')
const highlight = computed(() => {
  const words = searched.value.toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return null
  const escaped = words.map(w => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
  return { words: new Set(words), re: new RegExp(`(${escaped.join('|')})`, 'gi') }
})
function messageParts(message: string): { text: string, hit: boolean }[] {
  const h = highlight.value
  if (!h) return [{ text: message, hit: false }]
  return message.split(h.re).filter(Boolean)
    .map(part => ({ text: part, hit: h.words.has(part.toLowerCase()) }))
}

// Gagnant non détecté : désigné à la main, sur le giveaway de la commande du message (sinon
// le giveaway ouvert de la chaîne, sinon en pause, sinon le plus récent), modifiable avant de valider
const { snapshot } = useSnapshot()
const STATUS_RANK = { open: 0, paused: 1, closed: 2 } as const
const channelGiveaways = computed(() => (snapshot.value?.giveaways.giveaways ?? [])
  .filter(g => g.channel === props.channel)
  .sort((a, b) => STATUS_RANK[a.status] - STATUS_RANK[b.status] || b.detectedAt.localeCompare(a.detectedAt)))
const giveawayItems = computed(() => channelGiveaways.value.map(g => ({
  label: `${GIVEAWAY_KIND[g.kind].label}${g.command ? ` ${g.command}` : ''}${g.status === 'open' ? '' : g.status === 'paused' ? ' (en pause)' : ' (clos)'}${g.winners.length ? ` · gagnant${g.winners.length > 1 ? 's' : ''} : ${g.winners.join(', ')}` : ''}`,
  value: g.id
})))
const confirmWinner = ref<string | null>(null)
const winnerGiveaway = ref<number | undefined>(undefined)
// Gagnants déjà notés sur le giveaway choisi : le nouveau s'y ajoute
const chosenWinners = computed(() => channelGiveaways.value.find(g => g.id === winnerGiveaway.value)?.winners ?? [])
function askWinner(e: ChatLogEntry) {
  const list = channelGiveaways.value
  const byCommand = e.command ? list.find(g => g.command?.toLowerCase() === e.command!.toLowerCase()) : undefined
  winnerGiveaway.value = (byCommand ?? list[0])?.id
  confirmWinner.value = e.author
}
async function setWinner() {
  const winner = confirmWinner.value
  if (!winner || !props.channel) return
  confirmWinner.value = null
  try {
    await invoke('set_manual_winner', { channel: props.channel, winner, giveawayId: winnerGiveaway.value ?? null })
    toast.add({ title: `${winner} ajouté aux gagnants`, color: 'success', icon: 'i-lucide-trophy' })
  } catch (e) {
    toast.add({ title: 'Gagnant non enregistré', description: String(e), color: 'error' })
  }
}

function time(iso: string) {
  const d = new Date(iso)
  const today = new Date().toDateString() === d.toDateString()
  const t = d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit', second: '2-digit' })
  return today ? t : `${d.toLocaleDateString('fr-FR', { day: 'numeric', month: 'short' })} ${t}`
}
</script>

<template>
  <UModal v-model:open="open" :title="`Journal du chat de ${channel ?? ''}`" description="Commandes des spectateurs, tes messages, ceux du streamer, annonces de gagnant et réponses des bots, gardés 14 jours." :ui="{ content: 'max-w-5xl' }">
    <template #body>
      <div class="flex flex-col gap-3">
        <!-- Réponses des bots aux commandes d'information -->
        <div v-if="replies.length" class="flex flex-col gap-1.5">
          <div class="text-sm font-medium">Réponses des bots</div>
          <div v-for="r in replies" :key="r.command" class="border border-default rounded-md px-3 py-2 text-sm flex flex-col gap-0.5">
            <div class="flex items-center gap-2 flex-wrap text-xs text-muted">
              <span class="font-mono text-default">{{ r.command }}</span>
              <span>→ {{ r.bot }}</span>
              <span>· demandé par {{ r.askedBy }}</span>
              <span>· modifiée {{ time(r.updatedAt) }}</span>
              <span v-if="r.seenAt !== r.updatedAt">· revue {{ time(r.seenAt) }}</span>
            </div>
            <div class="break-words">{{ r.message }}</div>
          </div>
        </div>

        <div class="flex items-center gap-2 flex-wrap">
          <USelect v-model="kinds" :items="kindItems" multiple placeholder="Tous les messages" class="w-56" aria-label="Types de message" />
          <UButton v-if="kinds.length" label="Tous les messages" size="xs" color="neutral" variant="ghost" icon="i-lucide-x" @click="kinds = []" />
          <USelectMenu
            v-model="selectedCommands"
            :items="commandItems"
            value-key="value"
            multiple
            placeholder="Toutes les commandes"
            :search-input="{ placeholder: 'Rechercher une commande' }"
            class="w-60"
            aria-label="Commandes"
          />
          <UButton v-if="selectedCommands.length" label="Toutes les commandes" size="xs" color="neutral" variant="ghost" icon="i-lucide-x" @click="selectedCommands = []" />
          <UInput id="chat-log-author" v-model="author" icon="i-lucide-user-search" placeholder="Rechercher un auteur" class="w-48" />
          <UInput id="chat-log-text" v-model="text" icon="i-lucide-search" placeholder="Mots dans les messages" class="w-56" />
          <span class="flex-1" />
          <UButton icon="i-lucide-refresh-cw" color="neutral" variant="soft" :loading="loading" aria-label="Actualiser" title="Actualiser" @click="refresh" />
        </div>

        <div v-if="beforeAt" class="flex items-center gap-3 border border-primary/40 bg-primary/10 rounded-lg px-3 py-2 text-sm">
          <UIcon name="i-lucide-scroll-text" class="size-4 text-primary shrink-0" />
          <span class="flex-1">Journal ouvert sur ce giveaway : messages jusqu'à son tirage ou sa clôture.</span>
          <UButton label="Voir les plus récents" size="xs" color="neutral" variant="soft" @click="showRecent" />
        </div>

        <div class="border border-default rounded-lg overflow-auto max-h-[55vh]">
          <table class="w-full text-sm">
            <thead class="bg-elevated/60 text-xs uppercase text-muted sticky top-0">
              <tr>
                <th class="text-left px-3 py-2">Heure</th>
                <th class="text-left px-3 py-2">Auteur</th>
                <th class="text-left px-3 py-2">Type</th>
                <th class="text-left px-3 py-2">Commande</th>
                <th class="text-left px-3 py-2">Message</th>
                <th class="text-center px-3 py-2">Gagnant</th>
              </tr>
            </thead>
            <tbody v-for="grp in groups" :key="grp.key">
              <!-- En-tête de section : le giveaway pendant lequel ces messages ont été écrits -->
              <tr :id="grp.section ? `chat-log-g${grp.section.id}` : undefined" class="border-t border-default bg-primary/5 scroll-mt-10" :class="grp.section && grp.section.id === giveawayId ? 'bg-primary/15' : ''">
                <td colspan="6" class="px-3 py-2">
                  <div v-if="grp.section" class="flex items-center gap-x-3 gap-y-1 flex-wrap text-sm">
                    <UIcon name="i-lucide-gift" class="size-4 text-primary shrink-0" />
                    <span class="font-semibold">{{ sectionTitle(grp.section) }}</span>
                    <span v-if="grp.section.gifts.length" class="text-muted">{{ grp.section.gifts.join(', ') }}</span>
                    <span v-if="grp.section.winners.length"><UIcon name="i-lucide-trophy" class="size-3.5 text-success align-[-2px]" /> {{ grp.section.winners.join(', ') }}</span>
                    <span class="text-xs text-muted">{{ periodOf(grp.section) }}</span>
                  </div>
                  <div v-else class="flex items-center gap-2 text-sm text-muted">
                    <UIcon name="i-lucide-clock" class="size-4 shrink-0" />
                    {{ sections.length ? 'Depuis le dernier tirage (aucun giveaway en cours)' : 'Aucun giveaway connu sur cette chaîne pendant ces messages' }}
                  </div>
                </td>
              </tr>
              <tr v-for="e in grp.entries" :key="e.id" class="border-t border-default align-top">
                <td class="px-3 py-1.5 whitespace-nowrap text-muted tabular-nums">{{ time(e.at) }}</td>
                <td class="px-3 py-1.5 whitespace-nowrap font-medium">{{ e.author }}</td>
                <td class="px-3 py-1.5"><UBadge :label="KINDS[e.kind]?.label ?? e.kind" :color="KINDS[e.kind]?.color ?? 'neutral'" variant="subtle" size="sm" class="whitespace-nowrap" /></td>
                <td class="px-3 py-1.5 whitespace-nowrap font-mono">
                  <button
                    v-if="e.command"
                    type="button"
                    class="hover:underline"
                    :class="selectedCommands.includes(e.command) ? 'text-primary' : ''"
                    :title="selectedCommands.includes(e.command) ? 'Retirer du filtre' : 'Ajouter au filtre'"
                    @click="toggleCommand(e.command)"
                  >{{ e.command }}</button>
                  <span v-else class="text-muted">—</span>
                </td>
                <td class="px-3 py-1.5 break-words min-w-64">
                  <template v-for="(p, i) in messageParts(e.message)" :key="i"><mark v-if="p.hit" class="bg-primary/20 text-primary rounded px-0.5">{{ p.text }}</mark><template v-else>{{ p.text }}</template></template>
                </td>
                <td class="px-3 py-1.5">
                  <div class="flex justify-center">
                    <UButton
                      v-if="e.kind !== 'winner' && e.kind !== 'reply'"
                      icon="i-lucide-trophy"
                      size="xs"
                      color="neutral"
                      variant="ghost"
                      :aria-label="`Désigner ${e.author} gagnant`"
                      title="Désigner gagnant du giveaway (tirage non détecté)"
                      @click="askWinner(e)"
                    />
                  </div>
                </td>
              </tr>
            </tbody>
            <tbody v-if="!loading && entries.length === 0">
              <tr>
                <td colspan="6" class="px-3 py-8 text-center text-muted">Rien d'enregistré pour l'instant sur cette chaîne avec ces filtres.</td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-if="more" class="flex justify-center">
          <UButton label="Messages plus anciens" color="neutral" variant="soft" :loading="loading" @click="load(true)" />
        </div>
      </div>
    </template>
  </UModal>

  <UModal :open="confirmWinner !== null" :title="`Désigner ${confirmWinner ?? ''} gagnant ?`" @update:open="v => { if (!v) confirmWinner = null }">
    <template #body>
      <div class="flex flex-col gap-3 text-sm">
        <UFormField v-if="giveawayItems.length" label="Giveaway">
          <USelect id="winner-giveaway" v-model="winnerGiveaway" :items="giveawayItems" class="w-full" />
        </UFormField>
        <p v-else>Aucun giveaway connu sur {{ channel }} : ajoute-le d'abord.</p>
        <p class="text-muted">
          {{ confirmWinner }} sera ajouté aux gagnants de ce giveaway, qui sera clos<template v-if="chosenWinners.length"> (déjà gagnant{{ chosenWinners.length > 1 ? 's' : '' }} : {{ chosenWinners.join(', ') }})</template>.
        </p>
      </div>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton label="Annuler" color="neutral" variant="ghost" @click="confirmWinner = null" />
        <UButton label="Désigner gagnant" icon="i-lucide-trophy" :disabled="winnerGiveaway === undefined" @click="setWinner" />
      </div>
    </template>
  </UModal>
</template>