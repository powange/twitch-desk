<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { ApiError, pcAction } from '../api'
import { usePhone } from '../phone'
import { GIVEAWAY_KIND } from '../giveaways'
import { formatNumber } from '../format'
import { formatDuration, parseDuration } from '../duration'
import GiftImage from './GiftImage.vue'
import type { GiveawayFormData, GiveawayKind, PhoneAction } from '../types'

// Modification d'un giveaway depuis le téléphone : même formulaire que sur le PC (mêmes sections,
// mêmes champs, dans le même ordre), lu sur le PC puis renvoyé au PC qui l'enregistre
const props = defineProps<{ item: string | null, data: GiveawayFormData | null }>()
const open = defineModel<boolean>('open', { default: false })
const phone = usePhone()
const toast = useToast()

type TriState = 'yes' | 'no' | 'unknown'
const TRI_OPTIONS: { label: string, value: TriState }[] = [
  { label: 'Oui', value: 'yes' },
  { label: 'Non', value: 'no' },
  { label: '?', value: 'unknown' }
]
// Une condition par ligne, dans l'ordre du PC
const CONDITIONS = [
  { key: 'presence', label: 'Présence au tirage', icon: 'i-lucide-user-check' },
  { key: 'discord', label: 'Discord du streamer', icon: 'i-simple-icons-discord' },
  { key: 'follow', label: 'Follow requis', icon: 'i-lucide-heart' },
  { key: 'greeting', label: 'Avoir dit bonjour dans le chat', icon: 'i-lucide-hand' },
  { key: 'chatTime', label: 'Temps de présence minimum dans le chat', icon: 'i-lucide-timer' },
  { key: 'canGift', label: 'Gain offrable à quelqu\'un d\'autre', icon: 'i-lucide-hand-heart' }
] as const
const toTri = (v: boolean | null | undefined): TriState => v === true ? 'yes' : v === false ? 'no' : 'unknown'
const fromTri = (v: TriState): boolean | null => v === 'yes' ? true : v === 'no' ? false : null
const KIND_ICONS: Record<GiveawayKind, string> = {
  command: 'i-lucide-terminal',
  ticket: 'i-lucide-ticket',
  streamelements: 'i-lucide-ticket-check',
  channelpoints: 'i-lucide-coins'
}
const KIND_ITEMS = (Object.keys(GIVEAWAY_KIND) as GiveawayKind[]).map(value => ({ label: GIVEAWAY_KIND[value].label, value }))

// Tirage : date et heure facultative (heure locale) ; sans heure, 23 h 59 (toute la journée)
const DAY_END = '23:59'
const pad = (n: number) => String(n).padStart(2, '0')
function drawParts(iso: string | null): { date: string, time: string } {
  if (!iso) return { date: '', time: '' }
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return { date: '', time: '' }
  const time = `${pad(d.getHours())}:${pad(d.getMinutes())}`
  return { date: `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`, time: time === DAY_END ? '' : time }
}
function drawIso(date: string, time: string): string | null {
  if (!date) return null
  const d = new Date(`${date}T${time || DAY_END}`)
  return Number.isNaN(d.getTime()) ? null : d.toISOString()
}
const chanceInput = (c: string | null | undefined) => (c ?? '').trim().replace(/\s*%$/, '').replace(',', '.')
const winnersKey = (list: { login: string, chance: string | null }[]) => list.map(w => `${w.login.toLowerCase()}=${chanceInput(w.chance)}`).join()

const emptyForm = () => ({
  channel: '',
  kind: 'command' as GiveawayKind,
  command: '',
  url: '',
  ticket: 'unknown' as 'unique' | 'multi' | 'unknown',
  ticketMax: null as number | null,
  rewardName: '',
  rewardCost: null as number | null,
  drawDate: '',
  drawTime: '',
  follow: 'unknown' as TriState,
  presence: 'unknown' as TriState,
  discord: 'unknown' as TriState,
  greeting: 'unknown' as TriState,
  chatTime: 'unknown' as TriState,
  chatMinutes: null as number | null,
  canGift: 'unknown' as TriState,
  followDuration: '',
  notes: '',
  giftIds: [] as number[],
  winners: [] as { login: string, chance: string }[]
})
const form = ref(emptyForm())
// Fiche du streamer
const streamerDiscord = ref('')
const streamerNotes = ref('')
const streamerIgnored = ref<string[]>([])
const streamerAuto = ref<{ command: string, on: boolean }[]>([])
const streamerWinReply = ref('')
const streamerWinDelay = ref<number | null>(null)
const streamerWinGap = ref<number | null>(null)
const streamerSeName = ref('')
const autoKey = () => streamerAuto.value.map(a => `${a.command}:${a.on}`).join()
const streamerLoaded = ref({ discord: '', notes: '', ignored: '', auto: '', winReply: '', winDelay: null as number | null, winGap: null as number | null, seName: '' })
const shownGiftIds = ref<number[]>([])
const winnersBefore = ref('')
const reopen = ref(false)
const closed = computed(() => props.data?.giveaway.status === 'closed')

type Section = 'channel' | 'giveaway' | 'gifts' | 'conditions' | 'auto'
const section = ref<Section>('channel')

// Formulaire rempli à chaque ouverture, d'après ce que le PC a envoyé
watch(() => [open.value, props.data] as const, ([isOpen, d]) => {
  if (!isOpen || !d) return
  const g = d.giveaway
  const st = d.streamer
  const draw = drawParts(g.drawAt)
  form.value = {
    channel: g.channel,
    kind: g.kind,
    command: g.command ?? '',
    url: g.url ?? '',
    ticket: g.ticketUnique === true ? 'unique' : g.ticketUnique === false ? 'multi' : 'unknown',
    ticketMax: g.ticketMax,
    rewardName: g.rewardName ?? '',
    rewardCost: g.rewardCost,
    drawDate: draw.date,
    drawTime: draw.time,
    follow: toTri(g.requireFollow),
    presence: toTri(g.requirePresence),
    discord: toTri(g.requireDiscord),
    greeting: toTri(g.requireGreeting),
    chatTime: g.minChatMinutes == null ? 'unknown' : g.minChatMinutes > 0 ? 'yes' : 'no',
    chatMinutes: g.minChatMinutes ? g.minChatMinutes : null,
    canGift: toTri(g.canGift),
    followDuration: g.followMinSecs ? formatDuration(g.followMinSecs) : '',
    notes: g.notes ?? '',
    giftIds: [...g.giftIds],
    winners: g.winners.map(w => ({ login: w.login, chance: chanceInput(w.chance) }))
  }
  shownGiftIds.value = [...g.giftIds]
  winnersBefore.value = winnersKey(g.winners)
  streamerLoaded.value = {
    discord: st?.discordUrl ?? '', notes: st?.notes ?? '', ignored: (st?.ignoredCommands ?? []).join(),
    auto: (st?.autoCommands ?? []).map(c => `${c}:${(st?.autoCommandsOn ?? []).includes(c)}`).join(),
    winReply: st?.winReply ?? '', winDelay: st?.winReplyDelay ?? null, winGap: st?.winReplyGap ?? null, seName: st?.seName ?? ''
  }
  // Pas de Discord sur la fiche : celui trouvé sur sa chaîne Twitch (comme sur le PC)
  streamerDiscord.value = st?.discordUrl ?? d.profileDiscord ?? ''
  streamerNotes.value = st?.notes ?? ''
  streamerIgnored.value = [...(st?.ignoredCommands ?? [])]
  streamerAuto.value = (st?.autoCommands ?? []).map(command => ({ command, on: (st?.autoCommandsOn ?? []).includes(command) }))
  streamerWinReply.value = st?.winReply ?? ''
  streamerWinDelay.value = st?.winReplyDelay ?? null
  streamerWinGap.value = st?.winReplyGap ?? null
  streamerSeName.value = st?.seName ?? ''
  reopen.value = false
  saveId.value = null
  winnerInput.value = ''
  giftSearch.value = ''
  ignoredInput.value = ''
  autoInput.value = ''
  section.value = 'channel'
})

// --- Sections ---
const sections = computed(() => {
  const f = form.value
  const conditions = [f.follow, f.presence, f.discord, f.greeting, f.chatTime, f.canGift].filter(v => v !== 'unknown').length
  return [
    { key: 'channel' as const, label: 'Chaîne', icon: 'i-lucide-tv', summary: f.channel.trim() || 'à choisir', missing: !f.channel.trim() },
    { key: 'giveaway' as const, label: 'Giveaway', icon: 'i-lucide-gift', summary: (f.kind === 'command' || f.kind === 'ticket') && !f.command.trim() ? 'commande à détecter' : GIVEAWAY_KIND[f.kind].label, missing: false },
    { key: 'gifts' as const, label: 'Cadeaux', icon: 'i-lucide-package', summary: f.giftIds.length ? `${f.giftIds.length} choisi(s)` : 'aucun', missing: false },
    { key: 'conditions' as const, label: 'Conditions', icon: 'i-lucide-list-checks', summary: conditions ? `${conditions} renseignée(s)` : 'inconnues', missing: false },
    { key: 'auto' as const, label: 'Participation automatique', icon: 'i-lucide-bot', summary: streamerAuto.value.length ? `${streamerAuto.value.filter(a => a.on).length} sur ${streamerAuto.value.length} active(s)` : 'aucune commande', missing: false }
  ]
})

// --- Chaîne : commandes ignorées ---
const ignoredInput = ref('')
function addIgnored() {
  const list = [...streamerIgnored.value]
  for (const raw of ignoredInput.value.split(/[\s,]+/)) {
    const c = raw.trim().toLowerCase()
    if (c && !list.includes(c)) list.push(c)
  }
  streamerIgnored.value = list
  ignoredInput.value = ''
}

// --- Giveaway ---
const seGiveawayId = computed(() => /\/giveaway\/([0-9a-f]+)\/?$/i.exec(form.value.url.trim())?.[1] ?? null)
// Ce que StreamElements dit du giveaway, lu par le PC à l'ouverture (pour le lien d'alors)
const seInfo = computed(() => form.value.url.trim() === (props.data?.giveaway.url ?? '').trim() ? props.data?.seInfo ?? null : null)
const seStateLabel = computed(() => {
  const i = seInfo.value
  if (i?.winners.length) return { label: 'Tiré', color: 'success' as const }
  switch (i?.state) {
    case 'running': return { label: 'Participations ouvertes', color: 'primary' as const }
    case 'closed': return { label: 'En attente du tirage', color: 'warning' as const }
    case 'completed': case 'ended': return { label: 'Terminé', color: 'neutral' as const }
    case 'canceled': case 'cancelled': return { label: 'Annulé', color: 'error' as const }
    default: return { label: i?.state || 'Inconnu', color: 'neutral' as const }
  }
})
const seStreamerUrl = computed(() => `https://streamelements.com/${encodeURIComponent((streamerSeName.value.trim() || form.value.channel.trim().replace(/^@/, '')).toLowerCase())}`)
// Récompenses de la chaîne (lues par le PC pour un giveaway de points de chaîne)
const GIVEAWAY_WORDS = /giveaway|ticket|tirage|concours|participation|gagner|lot|cadeau/i
const rewards = computed(() => props.data?.rewards ?? [])
const rewardItems = computed(() => [...rewards.value]
  .sort((a, b) => Number(GIVEAWAY_WORDS.test(b.title)) - Number(GIVEAWAY_WORDS.test(a.title)))
  .map(r => ({
    label: r.title,
    value: r.id,
    description: [`${formatNumber(r.cost)} points`, r.paused ? 'en pause' : null, r.inStock ? null : 'épuisée'].filter(Boolean).join(' · ')
  })))
const chosenReward = computed<string | undefined>({
  get: () => rewards.value.find(r => r.title === form.value.rewardName && (form.value.rewardCost === null || r.cost === form.value.rewardCost))?.id,
  set: (id) => {
    const r = rewards.value.find(x => x.id === id)
    if (!r) return
    form.value.rewardName = r.title
    form.value.rewardCost = r.cost
  }
})
const winnerInput = ref('')
function addWinner() {
  const login = winnerInput.value.trim().replace(/^@/, '').toLowerCase()
  winnerInput.value = ''
  if (login && !form.value.winners.some(w => w.login.toLowerCase() === login)) form.value.winners.push({ login, chance: '' })
}
function removeWinner(login: string) {
  form.value.winners = form.value.winners.filter(w => w.login !== login)
}

// --- Cadeaux : par catégorie (celle du live d'abord), puis par groupe ; cochés et suggérés d'abord ---
const giftSearch = ref('')
const suggested = computed(() => new Set((props.data?.gifts ?? []).filter(g => g.suggested).map(g => g.id)))
const giftSections = computed(() => {
  const d = props.data
  if (!d) return []
  const q = giftSearch.value.trim().toLowerCase()
  const list = d.gifts.filter(g => !q || g.title.toLowerCase().includes(q) || (g.gameName ?? '').toLowerCase().includes(q))
  const rank = (id: number) => (form.value.giftIds.includes(id) ? 2 : 0) + (suggested.value.has(id) ? 1 : 0)
  const byCategory = new Map<string, typeof list>()
  for (const g of list) byCategory.set(g.gameId ?? '', [...(byCategory.get(g.gameId ?? '') ?? []), g])
  return [...byCategory].map(([key, items]) => {
    const byGroup = new Map<number | null, typeof list>()
    for (const g of items) {
      const group = d.groups.some(x => x.id === g.groupId) ? g.groupId : null
      byGroup.set(group, [...(byGroup.get(group) ?? []), g])
    }
    return {
      id: key || null,
      name: key ? (items[0]?.gameName ?? key) : 'Sans catégorie',
      live: !!key && key === d.liveGameId,
      groups: [...byGroup].map(([id, members]) => ({
        id,
        name: d.groups.find(x => x.id === id)?.name ?? 'Sans groupe',
        gifts: [...members].sort((a, b) => rank(b.id) - rank(a.id) || a.title.localeCompare(b.title))
      })).sort((a, b) => (a.id === null ? 1 : 0) - (b.id === null ? 1 : 0) || a.name.localeCompare(b.name))
    }
  }).sort((a, b) => Number(b.live) - Number(a.live) || (a.id === null ? 1 : 0) - (b.id === null ? 1 : 0) || a.name.localeCompare(b.name))
})
function toggleGift(id: number) {
  const ids = form.value.giftIds
  form.value.giftIds = ids.includes(id) ? ids.filter(x => x !== id) : [...ids, id]
}

// --- Conditions ---
const FOLLOW_MAX_SECS = 3650 * 86_400
const followDurationError = computed(() => {
  const f = form.value
  if (f.follow !== 'yes' || !f.followDuration.trim()) return null
  const secs = parseDuration(f.followDuration)
  if (secs === null) return 'Écris une durée comme 7j, 12h, 30min, 45s ou 2h 30min.'
  if (secs > FOLLOW_MAX_SECS) return '3650 jours au plus.'
  return null
})
const WIN_REPLY_DELAY = 10
const WIN_REPLY_GAP = 3
const winReplyLines = computed(() => streamerWinReply.value.split('\n').filter(l => l.trim()).length)
// Conditions de la fiche du streamer, sous la forme du formulaire (celles qu'elle connaît)
const sheetConditions = computed(() => {
  const st = props.data?.streamer
  if (!st || st.login.toLowerCase() !== form.value.channel.trim().toLowerCase()) return null
  const out: Partial<Record<'follow' | 'presence' | 'discord' | 'greeting' | 'chatTime' | 'canGift', TriState>> & { followDuration?: string, chatMinutes?: number | null } = {}
  if (st.requireFollow !== null) {
    out.follow = toTri(st.requireFollow)
    out.followDuration = st.requireFollow && st.followMinSecs ? formatDuration(st.followMinSecs) : ''
  }
  if (st.requirePresence !== null) out.presence = toTri(st.requirePresence)
  if (st.requireDiscord !== null) out.discord = toTri(st.requireDiscord)
  if (st.requireGreeting !== null) out.greeting = toTri(st.requireGreeting)
  if (st.minChatMinutes !== null) {
    out.chatTime = st.minChatMinutes > 0 ? 'yes' : 'no'
    out.chatMinutes = st.minChatMinutes > 0 ? st.minChatMinutes : null
  }
  if (st.canGift !== null) out.canGift = toTri(st.canGift)
  return out
})
const sheetDiffers = computed(() => {
  const c = sheetConditions.value
  if (!c) return false
  const f = form.value as Record<string, unknown>
  return Object.entries(c).some(([k, v]) => f[k] !== v)
})
function applySheetConditions() {
  const c = sheetConditions.value
  if (c) Object.assign(form.value, c)
}

// --- Participation automatique ---
const autoInput = ref('')
function addAutoCommands() {
  for (const raw of autoInput.value.split(/[\s,]+/)) {
    let c = raw.trim().toLowerCase()
    if (!c) continue
    if (!c.startsWith('!')) c = `!${c}`
    if (!streamerAuto.value.some(a => a.command === c)) streamerAuto.value.push({ command: c, on: true })
  }
  autoInput.value = ''
}

// --- Enregistrement : envoyé au PC, suivi jusqu'à son résultat ---
const saveId = ref<string | null>(null)
const saving = computed(() => !!saveId.value && phone.actions.value.some(a => a.id === saveId.value && a.status === 'pending'))
function sectionForError(message: string): Section | null {
  const m = message.toLowerCase()
  if (m.includes('fiche') || m.includes('chaîne') || m.includes('discord')) return 'channel'
  if (m.includes('gagnant') || m.includes('chance')) return 'giveaway'
  if (['commande', 'ticket', 'lien', 'coût', 'tirage', 'type'].some(w => m.includes(w))) return 'giveaway'
  return null
}
watch(() => phone.actions.value.find(a => a.id === saveId.value), (a) => {
  if (!a || a.status === 'pending') return
  saveId.value = null
  if (a.status === 'done') open.value = false
  else {
    const target = sectionForError(a.message ?? '')
    if (target) section.value = target
  }
})
function discordError(): string | null {
  const url = streamerDiscord.value.trim()
  if (!url) return null
  if (!url.startsWith('https://') || url.length > 300 || /\s/.test(url)) return 'Le lien Discord doit commencer par https:// (sans espace).'
  return null
}
async function save(withReopen = false) {
  const token = phone.token.value
  if (!token || !props.item || saving.value) return
  const f = form.value
  if (followDurationError.value) {
    toast.add({ title: 'Ancienneté du follow invalide', description: followDurationError.value, color: 'error' })
    section.value = 'conditions'
    return
  }
  addWinner()
  const badChance = f.winners.find((w) => {
    const c = chanceInput(w.chance)
    if (!c) return false
    const n = Number(c)
    return !Number.isFinite(n) || n < 0 || n > 100
  })
  if (badChance) {
    toast.add({ title: 'Chance de gagner invalide', description: `${badChance.login} : « ${badChance.chance.trim()} ». Indique un nombre entre 0 et 100 (ex. 20,92), ou laisse vide.`, color: 'error' })
    section.value = 'giveaway'
    return
  }
  const loaded = streamerLoaded.value
  const streamerChanged = streamerDiscord.value !== loaded.discord || streamerNotes.value !== loaded.notes || streamerIgnored.value.join() !== loaded.ignored
    || streamerWinReply.value.trim() !== loaded.winReply || streamerWinDelay.value !== loaded.winDelay || streamerWinGap.value !== loaded.winGap
    || streamerSeName.value.trim() !== loaded.seName || autoKey() !== loaded.auto
  const invalid = streamerChanged ? discordError() : null
  if (invalid) {
    toast.add({ title: 'Lien Discord invalide', description: invalid, color: 'error' })
    section.value = 'channel'
    return
  }
  const input = {
    channel: f.channel,
    kind: f.kind,
    command: f.command || null,
    url: f.url || null,
    ticketUnique: f.ticket === 'unique' ? true : f.ticket === 'multi' ? false : null,
    ticketMax: (f.kind === 'ticket' && f.ticket === 'multi') || f.kind === 'streamelements' ? f.ticketMax : null,
    rewardName: f.rewardName || null,
    rewardCost: f.rewardCost,
    drawAt: drawIso(f.drawDate, f.drawTime),
    requireFollow: fromTri(f.follow),
    requirePresence: fromTri(f.presence),
    requireDiscord: fromTri(f.discord),
    requireGreeting: fromTri(f.greeting),
    minChatMinutes: f.chatTime === 'yes' ? (f.chatMinutes ?? null) : f.chatTime === 'no' ? 0 : null,
    followMinSecs: f.follow === 'yes' ? parseDuration(f.followDuration) : null,
    canGift: fromTri(f.canGift),
    notes: f.notes || null,
    giftIds: f.giftIds,
    shownGiftIds: shownGiftIds.value
  }
  const winners = winnersKey(f.winners) !== winnersBefore.value ? f.winners.map(w => ({ login: w.login, chance: w.chance.trim() || null })) : null
  const streamer = streamerChanged
    ? {
        discordUrl: streamerDiscord.value.trim() || null, notes: streamerNotes.value || null, ignoredCommands: streamerIgnored.value,
        winReply: streamerWinReply.value.trim() || null, winReplyDelay: streamerWinDelay.value, winReplyGap: streamerWinGap.value, seName: streamerSeName.value.trim() || null,
        autoCommands: streamerAuto.value.map(a => a.command), autoCommandsOn: streamerAuto.value.filter(a => a.on).map(a => a.command)
      }
    : null
  reopen.value = withReopen
  try {
    const a = await pcAction<PhoneAction>(token, { kind: 'giveaway_save', item: props.item, form: { input, reopen: withReopen && closed.value, winners, streamer } })
    phone.requested(a)
    saveId.value = a.id
  } catch (e) {
    toast.add({ title: 'Demande non envoyée', description: e instanceof ApiError && e.status === 400 ? 'Mets à jour le serveur de synchronisation (version 0.8.16 ou plus).' : String(e), color: 'error' })
  }
}
</script>

<template>
  <UModal v-model:open="open" :title="data ? `Giveaway de ${data.giveaway.channel}` : 'Giveaway'" description="Enregistré par le PC, comme son formulaire" :ui="{ content: 'max-w-4xl' }">
    <template #body>
      <form v-if="data" id="phone-giveaway-form" class="flex flex-col gap-4" @submit.prevent="save()">
        <!-- Sections en onglets au-dessus (comme le PC quand sa fenêtre est étroite) -->
        <nav class="flex flex-row gap-1 overflow-x-auto border-b border-default pb-2 -mx-1 px-1" aria-label="Sections du giveaway">
          <button
            v-for="s in sections"
            :key="s.key"
            type="button"
            class="flex items-start gap-2 rounded-md px-2.5 py-2 text-left transition-colors shrink-0"
            :class="section === s.key ? 'bg-primary/10 text-primary' : 'hover:bg-elevated'"
            :aria-current="section === s.key ? 'step' : undefined"
            @click="section = s.key"
          >
            <UIcon :name="s.icon" class="size-4 mt-0.5 shrink-0" />
            <span class="flex flex-col min-w-0">
              <span class="text-sm font-medium whitespace-nowrap">{{ s.label }}</span>
              <span class="text-xs whitespace-nowrap" :class="s.missing ? 'text-error' : 'text-muted'">{{ s.summary }}</span>
            </span>
          </button>
        </nav>

        <!-- Chaîne -->
        <div v-show="section === 'channel'" class="flex flex-col gap-4">
          <UFormField label="Chaîne">
            <UInput v-model="form.channel" placeholder="lunarsea" autocomplete="off" class="w-full" />
          </UFormField>
          <div v-if="data.botReplies?.length" class="flex flex-col gap-1.5">
            <div class="text-sm font-medium">Réponses des bots</div>
            <div v-for="r in data.botReplies" :key="r.command" class="border border-default rounded-md px-3 py-2 text-sm flex flex-col gap-0.5">
              <div class="text-xs text-muted"><span class="font-mono text-default">{{ r.command }}</span> → {{ r.bot }}</div>
              <div class="break-words">{{ r.message }}</div>
            </div>
          </div>
          <div class="flex flex-col gap-3 border-t border-default pt-4">
            <div class="text-xs uppercase tracking-wide text-muted">Sur ce streamer (gardé pour ses prochains giveaways)</div>
            <UFormField label="Discord du streamer" :help="data.profileDiscord && streamerDiscord === data.profileDiscord && !streamerLoaded.discord ? 'Trouvé sur sa chaîne Twitch' : undefined">
              <div class="flex gap-2">
                <UInput v-model="streamerDiscord" icon="i-simple-icons-discord" placeholder="https://discord.gg/…" class="flex-1 min-w-0" />
                <UButton v-if="streamerDiscord.startsWith('https://')" :to="streamerDiscord" target="_blank" icon="i-lucide-external-link" color="neutral" variant="soft" aria-label="Ouvrir le Discord" />
              </div>
            </UFormField>
            <UFormField label="Pseudo StreamElements" help="Seulement si sa page StreamElements n'a pas son pseudo Twitch (streamelements.com/pseudo) : sert à suivre ses points, ses giveaways et ses liens. Le pseudo ou l'adresse de la page.">
              <UInput v-model="streamerSeName" icon="i-lucide-ticket" :placeholder="form.channel ? form.channel.toLowerCase() : 'pseudo'" class="w-full" />
            </UFormField>
            <UFormField label="Commandes ignorées" help="Commandes de son chat qui ne déclenchent jamais d'alerte de giveaway (!points, !discord…)">
              <div class="flex items-center gap-1.5 flex-wrap">
                <UBadge v-for="c in streamerIgnored" :key="c" color="neutral" variant="subtle" class="font-mono">
                  {{ c }}
                  <button type="button" class="ml-1 hover:text-error" :aria-label="`Ne plus ignorer ${c}`" @click="streamerIgnored = streamerIgnored.filter(x => x !== c)">×</button>
                </UBadge>
                <UInput v-model="ignoredInput" placeholder="!commande" size="sm" class="w-36 font-mono" @keydown.enter.prevent="addIgnored" @blur="addIgnored" />
              </div>
            </UFormField>
            <UFormField label="Notes sur le streamer">
              <UTextarea v-model="streamerNotes" :rows="3" autoresize placeholder="Habitudes de tirage, règles, horaires…" class="w-full" />
            </UFormField>
          </div>
        </div>

        <!-- Giveaway : type, puis les champs de ce type -->
        <div v-show="section === 'giveaway'" class="flex flex-col gap-4">
          <UFormField label="Type de giveaway">
            <div class="grid grid-cols-2 gap-2">
              <button
                v-for="k in KIND_ITEMS"
                :key="k.value"
                type="button"
                class="flex flex-col items-center gap-1.5 rounded-lg border p-3 text-sm transition-colors"
                :class="form.kind === k.value ? 'border-primary bg-primary/10 text-primary' : 'border-default'"
                :aria-pressed="form.kind === k.value"
                @click="form.kind = k.value"
              >
                <UIcon :name="KIND_ICONS[k.value]" class="size-5" />
                {{ k.label }}
              </button>
            </div>
          </UFormField>

          <template v-if="form.kind === 'streamelements'">
            <div v-if="form.channel.trim()" class="flex items-center gap-2 text-sm flex-wrap">
              <UIcon name="i-lucide-globe" class="size-4 text-muted shrink-0" />
              <span class="text-muted">Page StreamElements de {{ form.channel.trim() }} :</span>
              <a :href="seStreamerUrl" target="_blank" rel="noopener" class="text-primary font-medium inline-flex items-center gap-1 break-all">
                {{ seStreamerUrl.replace('https://', '') }}<UIcon name="i-lucide-external-link" class="size-3.5 shrink-0" />
              </a>
            </div>
            <UFormField
              label="Lien du giveaway StreamElements"
              :help="seGiveawayId ? `Giveaway StreamElements n° ${seGiveawayId} : clos dès que StreamElements annonce son gagnant` : 'Avec le lien, le giveaway est clos dès que StreamElements annonce son gagnant'"
            >
              <UInput v-model="form.url" placeholder="https://streamelements.com/…/giveaway/…" class="w-full" />
            </UFormField>
            <div v-if="seGiveawayId && seInfo" class="border border-default rounded-lg px-4 py-3 flex flex-col gap-2 text-sm">
              <div class="flex items-start gap-2">
                <p class="font-semibold flex-1 whitespace-pre-line break-words">{{ seInfo.title || 'Sans titre' }}</p>
                <UBadge :label="seStateLabel.label" :color="seStateLabel.color" variant="subtle" size="sm" class="shrink-0" />
              </div>
              <p v-if="seInfo.description" class="text-muted whitespace-pre-line break-words">{{ seInfo.description }}</p>
              <div class="flex flex-wrap gap-x-5 gap-y-1">
                <span class="inline-flex items-center gap-1.5"><UIcon name="i-lucide-users" class="size-4 text-muted" />{{ formatNumber(seInfo.totalUsers) }} participant(s)</span>
                <span class="inline-flex items-center gap-1.5"><UIcon name="i-lucide-ticket" class="size-4 text-muted" />{{ formatNumber(seInfo.totalTickets) }} ticket(s) acheté(s)</span>
                <span v-if="seInfo.winners.length" class="inline-flex items-center gap-1.5 text-success"><UIcon name="i-lucide-trophy" class="size-4" />{{ seInfo.winners.join(', ') }}</span>
              </div>
            </div>
            <UFormField label="Tickets maximum par personne">
              <UInputNumber v-model="form.ticketMax" :min="1" :max="100000" placeholder="Inconnu" class="w-full" />
            </UFormField>
          </template>

          <UFormField
            v-if="form.kind === 'command' || form.kind === 'ticket' || form.kind === 'streamelements'"
            label="Commande à taper"
            :help="form.kind !== 'streamelements' && !form.command.trim() ? 'Pas encore connue ? Laisse vide : elle sera remplie dès que la détection la repère dans le chat.' : undefined"
          >
            <UInput v-model="form.command" placeholder="!coffre" class="w-full font-mono" />
          </UFormField>
          <div v-if="form.kind === 'ticket'" class="grid grid-cols-2 gap-4">
            <UFormField label="Tickets">
              <USelect
                v-model="form.ticket"
                :items="[{ label: 'Un seul par personne', value: 'unique' }, { label: 'Plusieurs par personne', value: 'multi' }, { label: 'On ne sait pas', value: 'unknown' }]"
                class="w-full"
              />
            </UFormField>
            <UFormField v-if="form.ticket === 'multi'" label="Tickets maximum par personne">
              <UInputNumber v-model="form.ticketMax" :min="1" :max="100000" placeholder="Inconnu" class="w-full" />
            </UFormField>
          </div>
          <UFormField
            v-if="form.kind === 'channelpoints' && rewards.length"
            label="Récompenses de la chaîne"
            help="Choisir la récompense du giveaway remplit son nom et son coût"
          >
            <USelectMenu
              v-model="chosenReward"
              :items="rewardItems"
              value-key="value"
              :search-input="{ placeholder: 'Rechercher une récompense' }"
              placeholder="Choisir dans la liste de la chaîne"
              icon="i-lucide-coins"
              class="w-full"
            />
          </UFormField>
          <div v-if="form.kind === 'channelpoints'" class="grid grid-cols-2 gap-4">
            <UFormField label="Récompense à acheter">
              <UInput v-model="form.rewardName" placeholder="Ticket giveaway" class="w-full" />
            </UFormField>
            <UFormField label="Coût en points de chaîne">
              <UInputNumber v-model="form.rewardCost" :min="0" placeholder="Inconnu" class="w-full" />
            </UFormField>
          </div>
          <UFormField label="Tirage" help="Heure facultative : sans heure, toute la journée. Avec une heure, rappel 5 min avant si la présence est requise">
            <div class="flex gap-2">
              <UInput v-model="form.drawDate" type="date" class="flex-1 min-w-0" aria-label="Jour du tirage" />
              <UInput v-model="form.drawTime" type="time" class="w-28" :disabled="!form.drawDate" aria-label="Heure du tirage (facultative)" />
            </div>
          </UFormField>
          <UAlert
            v-if="form.kind === 'channelpoints'"
            icon="i-lucide-info"
            color="info"
            variant="subtle"
            title="La chaîne sera ajoutée à la page Points de chaîne de tes comptes, et regardée en priorité tant que ce giveaway est ouvert."
          />
          <UAlert
            v-else-if="form.kind === 'streamelements'"
            icon="i-lucide-info"
            color="info"
            variant="subtle"
            title="La chaîne sera ajoutée à la page StreamElements de tes comptes (présence dans le chat, achat des tickets)."
          />

          <USeparator label="Pour tous les types de giveaway" :ui="{ label: 'text-xs text-muted' }" />
          <UFormField label="Gagnants" help="Autant que nécessaire, avec leur chance de gagner si elle est connue. Tous retirés : le giveaway est rouvert">
            <div class="flex flex-col gap-2">
              <div v-for="w in form.winners" :key="w.login" class="flex items-center gap-2">
                <UIcon name="i-lucide-trophy" class="size-4 text-success shrink-0" />
                <span class="font-medium flex-1 truncate">{{ w.login }}</span>
                <UInput v-model="w.chance" size="sm" placeholder="Chance %" class="w-28" :aria-label="`Chance de gagner de ${w.login}`">
                  <template #trailing><span class="text-xs text-muted">%</span></template>
                </UInput>
                <UButton icon="i-lucide-x" size="xs" color="neutral" variant="ghost" :aria-label="`Retirer ${w.login} des gagnants`" @click="removeWinner(w.login)" />
              </div>
              <div class="flex gap-2">
                <UInput v-model="winnerInput" icon="i-lucide-trophy" :placeholder="form.winners.length ? 'Autre gagnant' : 'Pas encore tiré'" class="flex-1 min-w-0" @keydown.enter.prevent="addWinner" />
                <UButton label="Ajouter" icon="i-lucide-plus" color="neutral" variant="soft" :disabled="!winnerInput.trim()" @click="addWinner" />
              </div>
            </div>
          </UFormField>
          <UFormField label="Notes du giveaway">
            <UTextarea v-model="form.notes" :rows="3" autoresize placeholder="Lot, règles particulières de ce giveaway…" class="w-full" />
          </UFormField>
        </div>

        <!-- Cadeaux -->
        <div v-show="section === 'gifts'" class="flex flex-col gap-3">
          <div class="flex items-center gap-2">
            <span class="text-sm font-medium flex-1">Cadeaux <span v-if="form.giftIds.length" class="text-muted font-normal">({{ form.giftIds.length }})</span></span>
            <UInput v-model="giftSearch" icon="i-lucide-search" placeholder="Rechercher" size="sm" class="w-40" />
          </div>
          <div v-if="data?.gifts.length" class="flex flex-col gap-3">
            <section v-for="cat in giftSections" :key="cat.id ?? ''" class="flex flex-col gap-1.5">
              <div class="flex items-center gap-1.5 text-sm font-semibold">
                <UIcon name="i-lucide-gamepad-2" class="size-4 text-muted" />
                {{ cat.name }}
                <UBadge v-if="cat.live" label="en live" color="success" variant="subtle" size="sm" />
              </div>
              <div v-for="grp in cat.groups" :key="grp.id ?? 'none'" class="flex flex-col gap-1 pl-2">
                <div class="flex items-center gap-1 text-xs text-muted">
                  <UIcon :name="grp.id === null ? 'i-lucide-folder-x' : 'i-lucide-folder'" class="size-3.5" />{{ grp.name }}
                </div>
                <div class="grid grid-cols-1 min-[420px]:grid-cols-2 gap-2">
                  <button
                    v-for="g in grp.gifts"
                    :key="g.id"
                    type="button"
                    class="relative flex items-center gap-2 border rounded-md p-1.5 text-left text-sm transition-colors min-w-0"
                    :class="form.giftIds.includes(g.id) ? 'border-primary bg-primary/10' : 'border-default'"
                    :aria-pressed="form.giftIds.includes(g.id)"
                    @click="toggleGift(g.id)"
                  >
                    <GiftImage :src="g.image" />
                    <span class="flex flex-col min-w-0 pr-4">
                      <span class="leading-tight line-clamp-2">{{ g.title }}</span>
                      <span v-if="g.gameName" class="text-xs text-muted truncate">{{ g.gameName }}</span>
                    </span>
                    <UIcon v-if="suggested.has(g.id)" name="i-lucide-sparkles" class="absolute top-1 right-1 size-3.5 text-primary" aria-label="Suggéré" />
                  </button>
                </div>
              </div>
            </section>
            <p v-if="giftSections.length === 0" class="text-sm text-muted">Aucun cadeau ne correspond.</p>
          </div>
          <p v-else class="text-sm text-muted">Catalogue vide : ajoute des cadeaux sur le PC.</p>
          <p class="text-xs text-muted">Suggérés (étincelle) : le titre du cadeau ou un mot-clé apparaît dans le titre du live, la description, le message épinglé, le giveaway StreamElements ou la réponse d'un bot.</p>
        </div>

        <!-- Conditions : une par ligne -->
        <div v-show="section === 'conditions'" class="flex flex-col gap-3">
          <p class="text-sm text-muted">Ce qu'il faut pour participer. « ? » : on ne sait pas.</p>
          <div v-if="sheetDiffers" class="flex flex-wrap items-center justify-between gap-2 rounded-lg bg-elevated px-4 py-2.5 text-sm">
            <span class="text-muted">La fiche de {{ form.channel }} a d'autres conditions que ce giveaway.</span>
            <UButton label="Reprendre celles de la fiche" icon="i-lucide-clipboard-copy" size="sm" color="primary" variant="soft" @click="applySheetConditions" />
          </div>
          <div v-for="c in CONDITIONS" :key="c.key" class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border border-default rounded-lg px-4 py-3">
            <span class="inline-flex items-center gap-2.5 min-w-0">
              <UIcon :name="c.icon" class="size-5 text-muted shrink-0" /><span>{{ c.label }}</span>
            </span>
            <UFieldGroup size="sm" class="shrink-0">
              <UButton
                v-for="o in TRI_OPTIONS"
                :key="o.value"
                :label="o.label"
                class="w-11 justify-center"
                :color="form[c.key] === o.value ? 'primary' : 'neutral'"
                :variant="form[c.key] === o.value ? 'solid' : 'outline'"
                :aria-pressed="form[c.key] === o.value"
                @click="form[c.key] = o.value"
              />
            </UFieldGroup>
            <span v-if="c.key === 'follow' && form.follow === 'yes'" class="basis-full flex flex-wrap items-center justify-end gap-2 text-sm text-muted">
              depuis au moins
              <UInput v-model="form.followDuration" placeholder="ex. 7j, 2h 30min" size="sm" class="w-36" :color="followDurationError ? 'error' : undefined" :highlight="!!followDurationError" />
              <span v-if="followDurationError" class="basis-full text-right text-xs text-error">{{ followDurationError }}</span>
            </span>
            <div v-if="c.key === 'presence' && form.presence === 'yes'" class="basis-full flex flex-col gap-2 pt-1">
              <UFormField
                label="Réponse automatique si tu gagnes"
                :help="`Une phrase par ligne (5 au plus), écrites dans le chat par le compte gagnant l'une après l'autre, la première ${streamerWinDelay ?? WIN_REPLY_DELAY} s après l'annonce (annulable jusqu'à la dernière). Vide : pas de réponse. Gardée dans la fiche de ${form.channel || 'ce streamer'}, sur le PC seulement.`"
              >
                <UTextarea v-model="streamerWinReply" :rows="2" autoresize placeholder="Merci beaucoup, je suis là !" class="w-full" />
              </UFormField>
              <span v-if="streamerWinReply.trim()" class="flex items-center justify-end gap-2 text-sm text-muted">
                {{ winReplyLines > 1 ? 'première phrase après' : 'après' }}
                <UInputNumber v-model="streamerWinDelay" :min="0" :max="600" :placeholder="String(WIN_REPLY_DELAY)" size="sm" class="w-28" />
                seconde(s)
              </span>
              <span v-if="winReplyLines > 1" class="flex items-center justify-end gap-2 text-sm text-muted">
                puis
                <UInputNumber v-model="streamerWinGap" :min="0" :max="600" :placeholder="String(WIN_REPLY_GAP)" size="sm" class="w-28" />
                seconde(s) entre deux phrases
              </span>
            </div>
            <span v-if="c.key === 'chatTime' && form.chatTime === 'yes'" class="basis-full flex items-center justify-end gap-2 text-sm text-muted">
              au moins
              <UInputNumber v-model="form.chatMinutes" :min="1" :max="1440" placeholder="—" size="sm" class="w-28" />
              minute(s)
            </span>
          </div>
        </div>

        <!-- Participation automatique (fiche du streamer) -->
        <div v-show="section === 'auto'" class="flex flex-col gap-3">
          <p class="text-sm text-muted">
            Commandes que le streamer propose de taper pendant un court moment (pour gagner des points…), hors giveaway.
            Quand plusieurs spectateurs en tapent une, le PC la tape avec tes comptes après quelques secondes, puis attend 10 min avant de recommencer.
          </p>
          <p class="text-xs text-muted">Réglage de la fiche de {{ form.channel }} (pas seulement de ce giveaway), enregistré avec le giveaway. La liste est partagée avec tes autres PC ; y répondre se choisit sur chaque PC (ici : le PC qui enregistre).</p>
          <div class="flex items-center gap-2">
            <UInput v-model="autoInput" placeholder="!commande" size="sm" class="w-40 font-mono" @keydown.enter.prevent="addAutoCommands" />
            <UButton label="Ajouter" icon="i-lucide-plus" size="sm" color="neutral" variant="soft" :disabled="!autoInput.trim()" @click="addAutoCommands" />
          </div>
          <div v-if="streamerAuto.length" class="border border-default rounded-lg divide-y divide-default">
            <div v-for="a in streamerAuto" :key="a.command" class="flex items-center justify-between gap-3 px-3 py-2.5">
              <span class="font-mono break-all">{{ a.command }}</span>
              <div class="flex items-center gap-2 shrink-0">
                <USwitch v-model="a.on" label="Répondre sur ce PC" size="sm" :aria-label="`Répondre automatiquement à ${a.command} sur ce PC`" />
                <UButton icon="i-lucide-trash-2" size="xs" color="error" variant="ghost" :aria-label="`Retirer ${a.command}`" @click="streamerAuto = streamerAuto.filter(x => x.command !== a.command)" />
              </div>
            </div>
          </div>
          <p v-else class="text-sm text-muted">Aucune commande.</p>
        </div>
      </form>
    </template>
    <template #footer>
      <div class="flex justify-end items-center gap-2 w-full flex-wrap">
        <!-- Giveaway terminé : enregistré et rouvert (gagnants retirés) -->
        <UButton v-if="closed" label="Rouvrir" icon="i-lucide-rotate-ccw" color="neutral" variant="soft" class="mr-auto" :loading="saving && reopen" :disabled="saving" @click="save(true)" />
        <UButton label="Annuler" color="neutral" variant="ghost" @click="open = false" />
        <UButton type="submit" form="phone-giveaway-form" label="Enregistrer" :loading="saving && !reopen" :disabled="saving" />
      </div>
    </template>
  </UModal>
</template>
