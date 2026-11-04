<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../composables/useSnapshot'
import { suggestGifts, useGiftCatalog } from '../composables/useGiftCatalog'
import GiftFormModal from './GiftFormModal.vue'
import ChannelProfileCard, { type ChannelProfile } from './ChannelProfileCard.vue'
import { cachedInspect, storeInspect } from '../composables/channelInspectCache'
import { useOpenExternal } from '../utils/external'
import { boxArt, channelUrl, formatNumber } from '../utils/format'
import { GIVEAWAY_KIND_ITEMS } from '../utils/giveaways'
import { formatDuration, parseDuration } from '../utils/duration'
import type { GiveawayInput, GiveawayView } from '../types'
import IgnoredCommandsEditor from './IgnoredCommandsEditor.vue'

// Création (giveaway = null) ou modification d'une fiche de giveaway
// channel : chaîne pré-remplie pour un nouveau giveaway
// streamerOnly : fiche du streamer seulement (chaîne et conditions habituelles), sans giveaway
// past : ancien giveaway ajouté après coup (déjà terminé, à une date passée)
const props = defineProps<{ giveaway: GiveawayView | null, channel?: string | null, streamerOnly?: boolean, past?: boolean }>()
const open = defineModel<boolean>('open', { required: true })

const { snapshot } = useSnapshot()
const toast = useToast()
const { gifts, groups } = useGiftCatalog()
const openExternal = useOpenExternal()

type TriState = 'yes' | 'no' | 'unknown'
const TRI_OPTIONS: { label: string, value: TriState, title: string }[] = [
  { label: 'Oui', value: 'yes', title: 'Requis' },
  { label: 'Non', value: 'no', title: 'Non requis' },
  { label: '?', value: 'unknown', title: 'On ne sait pas' }
]
// Une condition par ligne, dans l'ordre des colonnes des tableaux (ConditionHeaders)
const CONDITIONS = [
  { key: 'presence', label: 'Présence au tirage', icon: 'i-lucide-user-check' },
  { key: 'discord', label: 'Discord du streamer', icon: 'i-simple-icons-discord' },
  { key: 'follow', label: 'Follow requis', icon: 'i-lucide-heart' },
  { key: 'greeting', label: 'Avoir dit bonjour dans le chat', icon: 'i-lucide-hand' },
  { key: 'chatTime', label: 'Temps de présence minimum dans le chat', icon: 'i-lucide-timer' }
] as const
const toTri = (v: boolean | null): TriState => v === true ? 'yes' : v === false ? 'no' : 'unknown'
const fromTri = (v: TriState): boolean | null => v === 'yes' ? true : v === 'no' ? false : null

// <input type="datetime-local"> attend « AAAA-MM-JJTHH:MM » en heure locale
function toLocalInput(iso: string | null): string {
  if (!iso) return ''
  const d = new Date(iso)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
}

// Identifiant StreamElements lu dans le lien (…/giveaway/<id>) : giveaway suivi par l'application
const seGiveawayId = computed(() => /\/giveaway\/([0-9a-f]+)\/?$/i.exec(form.value.url.trim())?.[1] ?? null)

// Giveaway StreamElements lu sur StreamElements d'après son lien : titre, description,
// participants, tickets, gagnants, état
interface SeGiveawayInfo { running: boolean, state: string, winners: string[], title: string, description: string, totalTickets: number, totalUsers: number, maxTickets: number | null }
const seInfo = ref<SeGiveawayInfo | null>(null)
const seInfoLoading = ref(false)
const seInfoError = ref<string | null>(null)
let seInfoTimer: ReturnType<typeof setTimeout> | null = null
let seInfoSeq = 0
watch(() => [open.value, form.value.kind, form.value.url.trim(), seGiveawayId.value] as const, ([isOpen, kind, url, id]) => {
  if (seInfoTimer) clearTimeout(seInfoTimer)
  if (!isOpen || kind !== 'streamelements' || !id) {
    seInfo.value = null
    seInfoError.value = null
    return
  }
  seInfoTimer = setTimeout(async () => {
    const seq = ++seInfoSeq
    seInfoLoading.value = true
    try {
      const info = await invoke<SeGiveawayInfo | null>('se_giveaway_info', { url })
      if (seq === seInfoSeq) {
        seInfo.value = info
        seInfoError.value = null
        // Maximum par personne annoncé par StreamElements, sans écraser une valeur déjà saisie
        if (info?.maxTickets && form.value.ticketMax == null) form.value.ticketMax = info.maxTickets
      }
    } catch (e) {
      if (seq === seInfoSeq) { seInfo.value = null; seInfoError.value = String(e) }
    } finally {
      if (seq === seInfoSeq) seInfoLoading.value = false
    }
  }, 400)
}, { immediate: true })
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

// Page StreamElements publique du streamer (ses giveaways, sa boutique) : son pseudo
// StreamElements s'il diffère de son pseudo Twitch
const seStreamerUrl = computed(() => `https://streamelements.com/${encodeURIComponent((streamerSeName.value.trim() || form.value.channel.trim().replace(/^@/, '')).toLowerCase())}`)

// Gagnants : pseudo tapé ajouté à la liste (sans doublon)
const winnerInput = ref('')
function addWinner() {
  const login = winnerInput.value.trim().replace(/^@/, '').toLowerCase()
  winnerInput.value = ''
  if (login && !form.value.winners.some(w => w.login.toLowerCase() === login)) form.value.winners.push({ login, chance: '' })
}
function removeWinner(login: string) {
  form.value.winners = form.value.winners.filter(w => w.login !== login)
}
// Liste comparable (pseudo et chance) : n'enregistrer que si elle a changé
const winnersKey = (list: { login: string, chance: string }[]) => list.map(w => `${w.login.toLowerCase()}=${chanceInput(w.chance)}`).join()
// Chance affichée dans le champ (le « % » est à côté) : « 20.92% » → « 20.92 »
function chanceInput(chance: string | null | undefined) {
  return (chance ?? '').trim().replace(/\s*%$/, '').replace(',', '.')
}

// Ancien giveaway : sa date (heure locale du formulaire), obligatoire
const pastAt = ref('')
const pastMode = computed(() => !!props.past && !props.giveaway && !props.streamerOnly)

// Giveaway terminé : rouvert à l'enregistrement (gagnant retiré)
const reopen = ref(false)
const closed = computed(() => props.giveaway?.status === 'closed')

const form = ref({
  channel: '',
  kind: 'command' as GiveawayInput['kind'],
  command: '',
  url: '',
  ticket: 'unknown' as 'unique' | 'multi' | 'unknown',
  ticketMax: null as number | null,
  rewardName: '',
  rewardCost: null as number | null,
  drawAt: '',
  follow: 'unknown' as TriState,
  presence: 'unknown' as TriState,
  discord: 'unknown' as TriState,
  greeting: 'unknown' as TriState,
  chatTime: 'unknown' as TriState,
  chatMinutes: null as number | null,
  // Ancienneté du follow écrite « 7j », « 2h 30min »…
  followDuration: '',
  notes: '',
  giftIds: [] as number[],
  // Gagnants et leur chance de gagner (« 20.92% », vide : inconnue)
  winners: [] as { login: string, chance: string }[]
})

// Ce que le cœur sait de la chaîne saisie (fiche Twitch, StreamElements, streamer)
const suggestion = ref<ChannelSuggestion | null>(null)

// --- Cadeaux ---
const giftSearch = ref('')
// Cadeaux dont le titre ou un mot-clé apparaît dans le titre du live, la description de la chaîne
// ou son message épinglé (comme sur le site Giveaway Twitch)
// Réponses des bots de la chaîne à !giveaway… (souvent la description du giveaway)
const botReplies = computed(() => snapshot.value?.giveaways.botReplies[form.value.channel.trim().toLowerCase()] ?? [])
const suggestionText = computed(() => {
  const login = form.value.channel.trim().toLowerCase()
  const p = suggestion.value?.profile
  return [
    ...botReplies.value.map(r => r.message),
    snapshot.value?.giveaways.channels.find(c => c.login === login)?.live?.title,
    p?.title,
    p?.description,
    p?.pinnedMessage,
    // Giveaway StreamElements actif : son titre et sa description annoncent souvent les lots
    suggestion.value?.streamelements?.title,
    suggestion.value?.streamelements?.description
  ].filter(Boolean).join('\n')
})
// Ancien giveaway : rien d'après le live actuel de la chaîne
const suggested = computed(() => new Set(pastMode.value ? [] : suggestGifts(gifts.value, suggestionText.value).map(g => g.id)))
// Jeu streamé en ce moment par la chaîne : ses cadeaux passent en tête
const liveGameId = computed(() => {
  const login = form.value.channel.trim().toLowerCase()
  return snapshot.value?.giveaways.channels.find(c => c.login === login)?.live?.gameId ?? null
})
// Cadeaux par catégorie (celle du live d'abord, sans catégorie à la fin) puis par groupe
// (sans groupe à la fin) ; dans un groupe, cochés et suggérés d'abord
const giftSections = computed(() => {
  const q = giftSearch.value.trim().toLowerCase()
  const list = gifts.value
    .filter(g => !q || g.title.toLowerCase().includes(q) || g.keywords.some(k => k.includes(q)) || (g.gameName ?? '').toLowerCase().includes(q))
  const rank = (id: number) => (form.value.giftIds.includes(id) ? 2 : 0) + (suggested.value.has(id) ? 1 : 0)
  const byCategory = new Map<string, typeof list>()
  for (const g of list) {
    const key = g.gameId ?? ''
    byCategory.set(key, [...(byCategory.get(key) ?? []), g])
  }
  return [...byCategory].map(([key, items]) => {
    const byGroup = new Map<number | null, typeof list>()
    for (const g of items) {
      const group = groups.value.some(x => x.id === g.groupId) ? g.groupId : null
      byGroup.set(group, [...(byGroup.get(group) ?? []), g])
    }
    return {
      id: key || null,
      name: key ? (items[0]?.gameName ?? key) : 'Sans catégorie',
      live: !!key && key === liveGameId.value,
      groups: [...byGroup].map(([id, members]) => ({
        id,
        name: groups.value.find(x => x.id === id)?.name ?? 'Sans groupe',
        gifts: [...members].sort((a, b) => rank(b.id) - rank(a.id) || a.title.localeCompare(b.title))
      })).sort((a, b) => (a.id === null ? 1 : 0) - (b.id === null ? 1 : 0) || a.name.localeCompare(b.name))
    }
  }).sort((a, b) => Number(b.live) - Number(a.live) || (a.id === null ? 1 : 0) - (b.id === null ? 1 : 0) || a.name.localeCompare(b.name))
})
// Cadeaux cochés ou décochés à la main : les suggestions ne les remplacent plus
const giftsTouched = ref(false)
function toggleGift(id: number) {
  giftsTouched.value = true
  const ids = form.value.giftIds
  form.value.giftIds = ids.includes(id) ? ids.filter(x => x !== id) : [...ids, id]
}
// À la création, les cadeaux suggérés sont cochés tant qu'on n'a rien choisi à la main
watch(suggested, (ids) => {
  if (!props.giveaway && createdId.value === null && open.value && !giftsTouched.value && ids.size) form.value.giftIds = [...ids]
})
const giftFormOpen = ref(false)
function onGiftCreated(id: number) {
  giftsTouched.value = true
  if (!form.value.giftIds.includes(id)) form.value.giftIds = [...form.value.giftIds, id]
}

// Giveaway créé par ce formulaire (un nouvel essai après une erreur le modifie au lieu d'en créer un autre)
const createdId = ref<number | null>(null)
const editId = computed(() => props.giveaway?.id ?? createdId.value)

watch(open, (isOpen) => {
  if (!isOpen) return
  if (inspectTimer) clearTimeout(inspectTimer)
  inspectSeq++
  inspecting.value = false
  suggestion.value = null
  createdId.value = null
  giftsTouched.value = false
  streamerDiscord.value = ''
  streamerNotes.value = ''
  streamerIgnored.value = []
  streamerWinReply.value = ''
  streamerWinDelay.value = null
  streamerSeName.value = ''
  streamerLoaded.value = { discord: '', notes: '', ignored: '', winReply: '', winDelay: null, seName: '' }
  const g = props.giveaway
  const channel = g?.channel ?? props.channel ?? ''
  // La chaîne remplie ici ne relance pas l'inspection retardée : elle est faite tout de suite, une fois
  skipChannel = channel
  form.value = {
    channel,
    kind: g?.kind ?? 'command',
    command: g?.command ?? '',
    url: g?.url ?? '',
    ticket: g?.ticketUnique === true ? 'unique' : g?.ticketUnique === false ? 'multi' : 'unknown',
    ticketMax: g?.ticketMax ?? null,
    rewardName: g?.rewardName ?? '',
    rewardCost: g?.rewardCost ?? null,
    drawAt: toLocalInput(g?.drawAt ?? null),
    follow: toTri(g?.requireFollow ?? null),
    presence: toTri(g?.requirePresence ?? null),
    discord: toTri(g?.requireDiscord ?? null),
    greeting: toTri(g?.requireGreeting ?? null),
    chatTime: g?.minChatMinutes == null ? 'unknown' : g.minChatMinutes > 0 ? 'yes' : 'no',
    chatMinutes: g?.minChatMinutes ? g.minChatMinutes : null,
    followDuration: g?.followMinSecs ? formatDuration(g.followMinSecs) : '',
    notes: g?.notes ?? '',
    giftIds: [...(g?.giftIds ?? [])],
    winners: (g?.winners ?? []).map((w, i) => ({ login: w, chance: chanceInput(g?.winnerChances[w.toLowerCase()] ?? (i === 0 ? g?.winnerChance : null)) }))
  }
  reopen.value = false
  pastAt.value = ''
  winnerInput.value = ''
  giftSearch.value = ''
  // Chaîne connue (modification, fiche du streamer, chaîne pré-remplie) : fiche chargée tout de suite,
  // même si c'est la même chaîne qu'à la dernière ouverture
  if (channel.trim()) void inspect(channel)
})

const channelItems = computed(() => (snapshot.value?.giveaways.channels ?? []).map(c => c.login))

// --- Pré-remplissage à la création : giveaway StreamElements actif, commande détectée dans le chat ---
interface ChannelSuggestion {
  streamelements: { url: string, title: string, description: string, command: string | null, cost: number, maxTickets: number | null } | null
  detectedCommand: string | null
  profile: ChannelProfile | null
  streamer: {
    login: string
    discordUrl: string | null
    notes: string | null
    ignoredCommands: string[]
    // Conditions habituelles du streamer
    requireFollow: boolean | null
    followMinSecs: number | null
    requirePresence: boolean | null
    requireDiscord: boolean | null
    requireGreeting: boolean | null
    minChatMinutes: number | null
    // Réponse automatique au gain (présence requise), propre à ce PC
    winReply: string | null
    winReplyDelay: number | null
    // Pseudo de sa page StreamElements s'il diffère de son pseudo Twitch
    seName: string | null
  } | null
}

// --- Informations propres au streamer (mémorisées, retrouvées à chaque giveaway) ---
const streamerDiscord = ref('')
const streamerNotes = ref('')
// Commandes du chat ignorées par la détection sur sa chaîne
const streamerIgnored = ref<string[]>([])
// Réponse écrite dans le chat quand un de nos comptes gagne (présence requise), et son délai
const WIN_REPLY_DELAY = 10
const streamerWinReply = ref('')
const streamerWinDelay = ref<number | null>(null)
// Pseudo StreamElements s'il diffère du pseudo Twitch (vide : le même)
const streamerSeName = ref('')
// Valeurs chargées, pour n'enregistrer que si elles changent
const streamerLoaded = ref({ discord: '', notes: '', ignored: '', winReply: '', winDelay: null as number | null, seName: '' })
const inspecting = ref(false)
let inspectTimer: ReturnType<typeof setTimeout> | null = null
let inspectSeq = 0
// Chaîne mise par l'ouverture du formulaire (déjà inspectée)
let skipChannel: string | null = null

watch(() => form.value.channel, (login) => {
  const skip = skipChannel
  skipChannel = null
  if (!open.value || login === skip) return
  if (inspectTimer) clearTimeout(inspectTimer)
  suggestion.value = null
  if (!login.trim()) return
  inspectTimer = setTimeout(() => void inspect(login), 400)
})

// Récompenses de points de chaîne de la chaîne : choisir celle du giveaway remplit son nom et
// son coût
interface ChannelReward { id: string, title: string, cost: number, prompt: string | null, image: string | null, background: string | null, paused: boolean, inStock: boolean }
const rewards = ref<ChannelReward[]>([])
const rewardsLoading = ref(false)
const rewardsError = ref<string | null>(null)
let rewardsFor = ''
let rewardsTimer: ReturnType<typeof setTimeout> | null = null
// Récompenses qui ressemblent à une participation (ticket, giveaway…) en premier
const GIVEAWAY_WORDS = /giveaway|ticket|tirage|concours|participation|gagner|lot|cadeau/i
const rewardItems = computed(() => {
  const list = [...rewards.value].sort((a, b) => Number(GIVEAWAY_WORDS.test(b.title)) - Number(GIVEAWAY_WORDS.test(a.title)))
  return list.map(r => ({
    label: r.title,
    value: r.id,
    description: [`${formatNumber(r.cost)} points`, r.paused ? 'en pause' : null, r.inStock ? null : 'épuisée'].filter(Boolean).join(' · '),
    avatar: r.image ? { src: r.image, alt: '' } : undefined
  }))
})
const chosenReward = computed<string | undefined>({
  get: () => rewards.value.find(r => r.title === form.value.rewardName && (form.value.rewardCost === null || r.cost === form.value.rewardCost))?.id,
  set: (id) => {
    const r = rewards.value.find(x => x.id === id)
    if (!r) return
    form.value.rewardName = r.title
    form.value.rewardCost = r.cost
  }
})
async function loadRewards(login: string) {
  rewardsFor = login
  rewardsLoading.value = true
  rewardsError.value = null
  try {
    const list = await invoke<ChannelReward[]>('channel_rewards', { channel: login })
    if (rewardsFor === login) rewards.value = list
  } catch (e) {
    if (rewardsFor === login) {
      rewards.value = []
      rewardsError.value = String(e)
      // Erreur : relue au prochain changement (type, chaîne) ou à la réouverture
      rewardsFor = ''
    }
  } finally {
    if (rewardsFor === login || rewardsFor === '') rewardsLoading.value = false
  }
}
watch(() => [open.value, form.value.kind, form.value.channel.trim().toLowerCase()] as const, ([isOpen, kind, login]) => {
  if (rewardsTimer) clearTimeout(rewardsTimer)
  // Formulaire fermé : relues à la prochaine ouverture (prix, pauses changés entre-temps)
  if (!isOpen) { rewardsFor = ''; return }
  if (kind !== 'channelpoints' || !login) {
    if (!login) { rewards.value = []; rewardsFor = ''; rewardsError.value = null }
    return
  }
  if (login === rewardsFor) return
  // Autre chaîne : l'ancienne liste n'est plus choisissable pendant le chargement
  if (rewardsFor && rewardsFor !== login) { rewards.value = []; rewardsError.value = null }
  rewardsTimer = setTimeout(() => { void loadRewards(login) }, 400)
}, { immediate: true })

// Valeurs des champs remplis d'après la fiche : ce que l'utilisateur modifie ensuite n'est pas écrasé
function formSnapshot() {
  const f = form.value
  return {
    discord: streamerDiscord.value, notes: streamerNotes.value, ignored: streamerIgnored.value.join(),
    winReply: streamerWinReply.value, winDelay: streamerWinDelay.value, seName: streamerSeName.value,
    follow: f.follow, followDuration: f.followDuration, presence: f.presence, discordCond: f.discord, greeting: f.greeting,
    chatTime: f.chatTime, chatMinutes: f.chatMinutes
  }
}

// Fiche déjà lue : affichée tout de suite, puis relue (petit indicateur sur l'encart pendant ce temps)
async function inspect(login: string) {
  const seq = ++inspectSeq
  inspecting.value = true
  const f = form.value
  let before = formSnapshot()
  const cached = cachedInspect<ChannelSuggestion>(login)
  if (cached) {
    applySuggestion(cached, before, false)
    before = formSnapshot()
  }
  try {
    const found = await invoke<ChannelSuggestion>('inspect_giveaway_channel', { login })
    storeInspect(login, found)
    // Une saisie plus récente (ou une réouverture) a pris le relais
    if (seq !== inspectSeq || form.value !== f) return
    applySuggestion(found, before, true)
  } catch {
    // Meilleur effort : rien à suggérer (la fiche gardée reste affichée)
  } finally {
    if (seq === inspectSeq) inspecting.value = false
  }
}

// `fresh` : relue à l'instant ; une fiche gardée ne pré-remplit pas le giveaway StreamElements
// ni la commande du chat (ils changent d'un live à l'autre)
function applySuggestion(found: ChannelSuggestion, before: ReturnType<typeof formSnapshot>, fresh: boolean) {
  const f = form.value
  suggestion.value = found
  // Fiche du streamer : ce qu'on a noté, sinon le lien Discord trouvé sur sa chaîne
  streamerLoaded.value = {
    discord: found.streamer?.discordUrl ?? '', notes: found.streamer?.notes ?? '', ignored: (found.streamer?.ignoredCommands ?? []).join(),
    winReply: found.streamer?.winReply ?? '', winDelay: found.streamer?.winReplyDelay ?? null, seName: found.streamer?.seName ?? ''
  }
  if (streamerSeName.value === before.seName) streamerSeName.value = found.streamer?.seName ?? ''
  if (streamerWinReply.value === before.winReply) streamerWinReply.value = found.streamer?.winReply ?? ''
  if (streamerWinDelay.value === before.winDelay) streamerWinDelay.value = found.streamer?.winReplyDelay ?? null
  if (streamerDiscord.value === before.discord) streamerDiscord.value = found.streamer?.discordUrl ?? found.profile?.discord ?? ''
  if (streamerNotes.value === before.notes) streamerNotes.value = found.streamer?.notes ?? ''
  if (streamerIgnored.value.join() === before.ignored) streamerIgnored.value = [...(found.streamer?.ignoredCommands ?? [])]
  const st = found.streamer
  // Fiche du streamer, et nouveau giveaway : toutes les conditions reprises de la fiche (jamais
  // celles changées pendant le chargement). Elles y sont gardées à jour quand on les change sur
  // un giveaway en cours
  if (st && (props.streamerOnly || !props.giveaway)) {
    if (f.follow === before.follow) f.follow = toTri(st.requireFollow)
    if (f.followDuration === before.followDuration) f.followDuration = st.followMinSecs ? formatDuration(st.followMinSecs) : ''
    if (f.presence === before.presence) f.presence = toTri(st.requirePresence)
    if (f.discord === before.discordCond) f.discord = toTri(st.requireDiscord)
    if (f.greeting === before.greeting) f.greeting = toTri(st.requireGreeting)
    if (f.chatTime === before.chatTime && f.chatMinutes === before.chatMinutes) {
      f.chatTime = st.minChatMinutes === null ? 'unknown' : st.minChatMinutes > 0 ? 'yes' : 'no'
      f.chatMinutes = st.minChatMinutes && st.minChatMinutes > 0 ? st.minChatMinutes : null
    }
  }
  // En modification (ou fiche du streamer), la fiche de la chaîne suffit : on ne remplace pas ce qui a été saisi
  // Ancien giveaway : ni le giveaway StreamElements du jour ni la commande vue dans le chat
  if (props.giveaway || props.streamerOnly || pastMode.value || !fresh) return
  if (found.streamelements) {
    f.kind = 'streamelements'
    f.url = found.streamelements.url
    if (found.streamelements.command) f.command = found.streamelements.command
    if (found.streamelements.maxTickets) f.ticketMax = found.streamelements.maxTickets
  } else if (found.detectedCommand && !f.command) {
    f.command = found.detectedCommand
  }
}
const saving = ref(false)

const profile = computed(() => suggestion.value?.profile ?? null)


// --- Sections du formulaire (menu vertical) ---
type Section = 'channel' | 'giveaway' | 'gifts' | 'conditions'
const section = ref<Section>('channel')
watch(open, (isOpen) => { if (isOpen) section.value = 'channel' })

const KIND_ICONS: Record<GiveawayInput['kind'], string> = {
  command: 'i-lucide-terminal',
  ticket: 'i-lucide-ticket',
  streamelements: 'i-lucide-ticket-check',
  channelpoints: 'i-lucide-coins'
}

const sections = computed(() => {
  const f = form.value
  const conditions = [f.follow, f.presence, f.discord, f.greeting, f.chatTime].filter(v => v !== 'unknown').length
  const all = [
    { key: 'channel' as const, label: 'Chaîne', icon: 'i-lucide-tv', summary: f.channel.trim() || 'à choisir', missing: !f.channel.trim() },
    { key: 'giveaway' as const, label: 'Giveaway', icon: 'i-lucide-gift', summary: (f.kind === 'command' || f.kind === 'ticket') && !f.command.trim() ? 'commande à détecter' : GIVEAWAY_KIND_ITEMS.find(k => k.value === f.kind)?.label ?? '', missing: false },
    { key: 'gifts' as const, label: 'Cadeaux', icon: 'i-lucide-package', summary: f.giftIds.length ? `${f.giftIds.length} choisi(s)` : 'aucun', missing: false },
    { key: 'conditions' as const, label: 'Conditions', icon: 'i-lucide-list-checks', summary: conditions ? `${conditions} renseignée(s)` : 'inconnues', missing: false }
  ]
  // Fiche du streamer : ni giveaway ni cadeaux
  return props.streamerOnly ? all.filter(s => s.key === 'channel' || s.key === 'conditions') : all
})

// Erreur du cœur : ouvrir la section concernée
function sectionForError(message: string): Section | null {
  const m = message.toLowerCase()
  if (m.includes('chaîne') || m.includes('discord')) return 'channel'
  if (['commande', 'ticket', 'lien', 'coût', 'tirage', 'type'].some(w => m.includes(w))) return 'giveaway'
  return null
}

// Lien Discord : vide ou https://… (même règle que le cœur), vérifié avant tout enregistrement
function discordError(): string | null {
  const url = streamerDiscord.value.trim()
  if (!url) return null
  if (!url.startsWith('https://') || url.length > 300 || /\s/.test(url)) return 'Le lien Discord doit commencer par https:// (sans espace).'
  return null
}

// Fiche du streamer relue juste avant l'enregistrement : on ne remplace que ce que le formulaire modifie
async function currentStreamer(login: string): Promise<Record<string, unknown> | null> {
  const key = login.trim().toLowerCase()
  const list = await invoke<{ login: string }[]>('get_streamers')
  return list.find(s => s.login === key) ?? null
}

// Fiche du streamer : Discord, notes, commandes ignorées et conditions habituelles
// Ancienneté du follow : vide (pas de minimum) ou durée lisible, 3650 jours au plus
const FOLLOW_MAX_SECS = 3650 * 86_400
const followDurationError = computed(() => {
  const f = form.value
  if (f.follow !== 'yes' || !f.followDuration.trim()) return null
  const secs = parseDuration(f.followDuration)
  if (secs === null) return 'Écris une durée comme 7j, 12h, 30min, 45s ou 2h 30min.'
  if (secs > FOLLOW_MAX_SECS) return '3650 jours au plus.'
  return null
})
async function saveStreamer() {
  const f = form.value
  if (followDurationError.value) {
    toast.add({ title: 'Ancienneté du follow invalide', description: followDurationError.value, color: 'error' })
    section.value = 'conditions'
    return
  }
  const invalid = discordError()
  if (invalid) {
    toast.add({ title: 'Lien Discord invalide', description: invalid, color: 'error' })
    section.value = 'channel'
    return
  }
  // Fiche pas encore chargée : l'enregistrer effacerait ce qu'elle contient
  if (!suggestion.value) {
    toast.add({ title: 'Fiche pas encore chargée', description: 'Attends la fin du chargement de la chaîne, puis réessaie.', color: 'warning' })
    return
  }
  saving.value = true
  try {
    await invoke('save_streamer', {
      streamer: {
        login: f.channel,
        discordUrl: streamerDiscord.value.trim() || null,
        notes: streamerNotes.value || null,
        ignoredCommands: streamerIgnored.value,
        requireFollow: fromTri(f.follow),
        followMinSecs: f.follow === 'yes' ? parseDuration(f.followDuration) : null,
        requirePresence: fromTri(f.presence),
        requireDiscord: fromTri(f.discord),
        requireGreeting: fromTri(f.greeting),
        minChatMinutes: f.chatTime === 'yes' ? (f.chatMinutes ?? null) : f.chatTime === 'no' ? 0 : null,
        winReply: streamerWinReply.value.trim() || null,
        winReplyDelay: streamerWinDelay.value,
        seName: streamerSeName.value.trim() || null
      }
    })
    toast.add({ title: `Fiche de ${f.channel} enregistrée`, color: 'success', icon: 'i-lucide-check' })
    open.value = false
  } catch (e) {
    toast.add({ title: 'Fiche non enregistrée', description: String(e), color: 'error' })
  } finally {
    saving.value = false
  }
}

async function reopenAndSave() {
  reopen.value = true
  await save()
  // Non enregistré (erreur) : la prochaine sauvegarde ne rouvre pas sans le demander
  if (open.value) reopen.value = false
}

async function save() {
  if (props.streamerOnly) return saveStreamer()
  const f = form.value
  const input: GiveawayInput = {
    channel: f.channel,
    kind: f.kind,
    command: f.command || null,
    url: f.url || null,
    ticketUnique: f.ticket === 'unique' ? true : f.ticket === 'multi' ? false : null,
    ticketMax: (f.kind === 'ticket' && f.ticket === 'multi') || f.kind === 'streamelements' ? f.ticketMax : null,
    rewardName: f.rewardName || null,
    rewardCost: f.rewardCost,
    drawAt: f.drawAt ? new Date(f.drawAt).toISOString() : null,
    requireFollow: fromTri(f.follow),
    requirePresence: fromTri(f.presence),
    requireDiscord: fromTri(f.discord),
    requireGreeting: fromTri(f.greeting),
    // Présence dans le chat : durée si requise, 0 si non requise, null si on ne sait pas
    minChatMinutes: f.chatTime === 'yes' ? (f.chatMinutes ?? null) : f.chatTime === 'no' ? 0 : null,
    followMinSecs: f.follow === 'yes' ? parseDuration(f.followDuration) : null,
    notes: f.notes || null,
    giftIds: f.giftIds
  }
  // Fiche du streamer à enregistrer si elle a changé (ou si le Discord vient d'être trouvé) ;
  // jamais si elle n'a pas été chargée (elle serait effacée)
  const loaded = streamerLoaded.value
  const streamerChanged = !!suggestion.value && (streamerDiscord.value !== loaded.discord || streamerNotes.value !== loaded.notes || streamerIgnored.value.join() !== loaded.ignored
    || streamerWinReply.value.trim() !== loaded.winReply || streamerWinDelay.value !== loaded.winDelay || streamerSeName.value.trim() !== loaded.seName)
  if (followDurationError.value) {
    toast.add({ title: 'Ancienneté du follow invalide', description: followDurationError.value, color: 'error' })
    section.value = 'conditions'
    return
  }
  // Chances vérifiées avant tout envoi : sinon le giveaway serait modifié, puis ses gagnants refusés
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
  if (pastMode.value && !pastAt.value) {
    toast.add({ title: 'Date du giveaway manquante', description: 'Indique quand ce giveaway a eu lieu.', color: 'error' })
    section.value = 'giveaway'
    return
  }
  const invalid = streamerChanged ? discordError() : null
  if (invalid) {
    toast.add({ title: 'Lien Discord invalide', description: invalid, color: 'error' })
    section.value = 'channel'
    return
  }
  const wasEdit = editId.value !== null
  saving.value = true
  // Fiche du giveaway enregistrée : une erreur ensuite (gagnants) ne porte que sur la suite
  let updated = false
  try {
    const id = editId.value
    if (id !== null) {
      await invoke('update_giveaway', { id, input })
      updated = true
      // Rouvert : ouvert si la chaîne est en live, sinon en pause ; gagnant retiré
      if (reopen.value && closed.value) await invoke('reopen_giveaway', { id })
      // Gagnant changé à la main : désigné (le giveaway est clos), ou retiré (il est rouvert)
      // Pseudo tapé sans cliquer sur « Ajouter » : ajouté aussi
      addWinner()
      const g = props.giveaway
      const before = winnersKey((g?.winners ?? []).map((w, i) => ({ login: w, chance: g?.winnerChances[w.toLowerCase()] ?? (i === 0 ? g?.winnerChance ?? '' : '') })))
      if (!reopen.value && g && g.id === id && winnersKey(f.winners) !== before) {
        await invoke('set_giveaway_winners', { id, winners: f.winners.map(w => ({ login: w.login, chance: w.chance.trim() || null })) })
      }
    }
    // Ancien giveaway : créé terminé à sa date, avec ses gagnants
    else if (pastMode.value) {
      addWinner()
      createdId.value = await invoke<number>('create_past_giveaway', {
        input,
        at: new Date(pastAt.value).toISOString(),
        winners: f.winners.map(w => ({ login: w.login, chance: w.chance.trim() || null }))
      })
    }
    // Créé une seule fois : un nouvel essai modifie ce giveaway
    else createdId.value = await invoke<number>('create_giveaway', { input })
  } catch (e) {
    toast.add({ title: updated ? 'Giveaway modifié, mais pas ses gagnants' : 'Giveaway non enregistré', description: String(e), color: 'error' })
    const target = sectionForError(String(e))
    if (target) section.value = target
    saving.value = false
    return
  }
  try {
    // Fiche du streamer modifiée (aussi depuis un ancien giveaway) : enregistrée
    if (streamerChanged) {
      // Conditions habituelles de la fiche (relue) reprises telles quelles, sinon elles seraient effacées
      const existing = await currentStreamer(f.channel)
      await invoke('save_streamer', {
        streamer: {
          ...(existing ?? {}), login: f.channel, discordUrl: streamerDiscord.value.trim() || null, notes: streamerNotes.value || null, ignoredCommands: streamerIgnored.value,
          winReply: streamerWinReply.value.trim() || null, winReplyDelay: streamerWinDelay.value, seName: streamerSeName.value.trim() || null
        }
      })
    }
    toast.add({ title: wasEdit ? 'Giveaway modifié' : 'Giveaway ajouté', color: 'success', icon: 'i-lucide-check' })
    open.value = false
  } catch (e) {
    toast.add({ title: 'Giveaway enregistré, mais pas la fiche du streamer', description: String(e), color: 'error' })
    section.value = 'channel'
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <UModal v-model:open="open" :title="streamerOnly ? `Fiche de ${channel}` : giveaway ? `Giveaway de ${giveaway.channel}` : pastMode ? 'Ancien giveaway' : 'Nouveau giveaway'" :ui="{ content: 'max-w-4xl' }">
    <template #body>
      <form id="giveaway-form" class="grid grid-cols-[200px_1fr] gap-6 min-h-[440px]" @submit.prevent="save">
        <!-- Menu des sections -->
        <nav class="flex flex-col gap-1 border-r border-default pr-4" aria-label="Sections du giveaway">
          <button
            v-for="item in sections"
            :key="item.key"
            type="button"
            class="flex items-start gap-2.5 rounded-md px-2.5 py-2 text-left transition-colors"
            :class="section === item.key ? 'bg-primary/10 text-primary' : 'hover:bg-elevated'"
            :aria-current="section === item.key ? 'step' : undefined"
            @click="section = item.key"
          >
            <UIcon :name="item.icon" class="size-4 mt-0.5 shrink-0" />
            <span class="flex flex-col min-w-0">
              <span class="text-sm font-medium">{{ item.label }}</span>
              <span class="text-xs truncate" :class="item.missing ? 'text-error' : 'text-muted'">{{ item.summary }}</span>
            </span>
          </button>
        </nav>

        <div class="min-w-0">
          <!-- Chaîne -->
          <div v-show="section === 'channel'" class="flex flex-col gap-4">
            <UFormField label="Chaîne">
              <!-- Champ texte simple (garde la valeur pré-remplie) ; les chaînes surveillées sont proposées -->
              <UInput
                id="gw-channel"
                v-model="form.channel"
                :disabled="streamerOnly"
                list="gw-channel-list"
                placeholder="lunarsea"
                autocomplete="off"
                class="w-full"
                :trailing-icon="inspecting && !profile ? 'i-lucide-loader-circle' : undefined"
                :ui="{ trailingIcon: 'animate-spin' }"
              />
              <datalist id="gw-channel-list">
                <option v-for="c in channelItems" :key="c" :value="c" />
              </datalist>
            </UFormField>
            <!-- Fiche de la chaîne -->
            <div v-if="profile" class="relative">
              <ChannelProfileCard :profile="profile" />
              <UIcon v-if="inspecting" name="i-lucide-loader-circle" class="absolute top-3 right-3 size-4 animate-spin text-muted" title="Mise à jour des informations de la chaîne…" />
            </div>
            <p v-else-if="suggestion && !inspecting && form.channel.trim()" class="text-sm text-error">Chaîne introuvable sur Twitch.</p>

            <UAlert
              v-if="suggestion?.streamelements && !giveaway && !streamerOnly && !pastMode"
              icon="i-lucide-sparkles"
              color="success"
              variant="subtle"
              :title="`Giveaway StreamElements actif : ${suggestion.streamelements.title || 'sans titre'}`"
              :description="`${suggestion.streamelements.cost} points le ticket${suggestion.streamelements.maxTickets ? `, ${suggestion.streamelements.maxTickets} au plus par personne` : ''}. Type, lien et commande remplis.`"
            />
            <UAlert
              v-else-if="suggestion?.detectedCommand && !giveaway && !streamerOnly && !pastMode"
              icon="i-lucide-sparkles"
              color="info"
              variant="subtle"
              :title="`Commande détectée dans le chat : ${suggestion.detectedCommand}`"
            />
            <div v-if="botReplies.length && !pastMode" class="flex flex-col gap-1.5">
              <div class="text-sm font-medium">Réponses des bots</div>
              <div v-for="r in botReplies" :key="r.command" class="border border-default rounded-md px-3 py-2 text-sm flex flex-col gap-0.5">
                <div class="text-xs text-muted"><span class="font-mono text-default">{{ r.command }}</span> → {{ r.bot }}</div>
                <div class="break-words">{{ r.message }}</div>
              </div>
            </div>
            <div class="flex flex-col gap-3 border-t border-default pt-4">
              <div class="text-xs uppercase tracking-wide text-muted">Sur ce streamer (gardé pour ses prochains giveaways)</div>
              <UFormField label="Discord du streamer" :help="profile?.discord && streamerDiscord === profile.discord && !streamerLoaded.discord ? 'Trouvé sur sa chaîne Twitch' : undefined">
                <div class="flex gap-2">
                  <UInput id="gw-streamer-discord" v-model="streamerDiscord" icon="i-simple-icons-discord" placeholder="https://discord.gg/…" class="flex-1" />
                  <UButton v-if="streamerDiscord.startsWith('https://')" icon="i-lucide-external-link" color="neutral" variant="soft" aria-label="Ouvrir le Discord" @click="openExternal(streamerDiscord)" />
                </div>
              </UFormField>
              <UFormField label="Pseudo StreamElements" help="Seulement si sa page StreamElements n'a pas son pseudo Twitch (streamelements.com/pseudo) : sert à suivre ses points, ses giveaways et ses liens. Le pseudo ou l'adresse de la page.">
                <UInput id="gw-streamer-se-name" v-model="streamerSeName" icon="i-lucide-ticket" :placeholder="form.channel ? form.channel.toLowerCase() : 'pseudo'" class="w-full" />
              </UFormField>
              <UFormField label="Commandes ignorées" help="Commandes de son chat qui ne déclenchent jamais d'alerte de giveaway (!points, !discord…)">
                <IgnoredCommandsEditor id="gw-streamer-ignored" v-model="streamerIgnored" />
              </UFormField>
              <UFormField label="Notes sur le streamer">
                <UTextarea id="gw-streamer-notes" v-model="streamerNotes" :rows="3" autoresize placeholder="Habitudes de tirage, règles, horaires…" class="w-full" />
              </UFormField>
            </div>
          </div>

          <!-- Giveaway : type, puis les champs de ce type -->
          <div v-show="section === 'giveaway'" class="flex flex-col gap-4">
            <UFormField v-if="pastMode" label="Date du giveaway" help="Quand il a eu lieu ; il est enregistré déjà terminé">
              <UInput id="gw-past-at" v-model="pastAt" type="datetime-local" :max="toLocalInput(new Date().toISOString())" class="w-full max-w-xs" />
            </UFormField>
            <UFormField label="Type de giveaway">
              <div class="grid grid-cols-4 gap-2">
                <button
                  v-for="k in GIVEAWAY_KIND_ITEMS"
                  :key="k.value"
                  type="button"
                  class="flex flex-col items-center gap-1.5 rounded-lg border p-3 text-sm transition-colors"
                  :class="form.kind === k.value ? 'border-primary bg-primary/10 text-primary' : 'border-default hover:border-primary/50'"
                  :aria-pressed="form.kind === k.value"
                  @click="form.kind = k.value"
                >
                  <UIcon :name="KIND_ICONS[k.value]" class="size-5" />
                  {{ k.label }}
                </button>
              </div>
            </UFormField>

            <!-- StreamElements : le lien d'abord, puis ce que StreamElements en dit, puis les réglages -->
            <template v-if="form.kind === 'streamelements'">
            <!-- Page StreamElements du streamer : ses giveaways, pour retrouver le lien du giveaway -->
            <div v-if="form.kind === 'streamelements' && form.channel.trim()" class="flex items-center gap-2 text-sm">
              <UIcon name="i-lucide-globe" class="size-4 text-muted shrink-0" />
              <span class="text-muted">Page StreamElements de {{ form.channel.trim() }} :</span>
              <button type="button" class="text-primary font-medium hover:underline inline-flex items-center gap-1 truncate" @click="openExternal(seStreamerUrl)">
                {{ seStreamerUrl.replace('https://', '') }}<UIcon name="i-lucide-external-link" class="size-3.5 shrink-0" />
              </button>
            </div>
              <UFormField
                label="Lien du giveaway StreamElements"
                :help="seGiveawayId ? `Giveaway StreamElements n° ${seGiveawayId} : clos dès que StreamElements annonce son gagnant` : 'Avec le lien, le giveaway est clos dès que StreamElements annonce son gagnant'"
              >
                <UInput id="gw-url" v-model="form.url" placeholder="https://streamelements.com/…/giveaway/…" class="w-full" />
              </UFormField>
              <div v-if="seGiveawayId" class="border border-default rounded-lg px-4 py-3 flex flex-col gap-2 text-sm">
                <div v-if="seInfoLoading && !seInfo" class="flex items-center gap-2 text-muted">
                  <UIcon name="i-lucide-loader" class="size-4 animate-spin" />Lecture du giveaway sur StreamElements…
                </div>
                <p v-else-if="seInfoError" class="text-error">{{ seInfoError }}</p>
                <p v-else-if="!seInfo" class="text-muted">Giveaway introuvable sur StreamElements : vérifie le lien.</p>
                <template v-else>
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
                </template>
              </div>
              <div class="grid grid-cols-2 gap-4">
                <UFormField label="Tickets maximum par personne">
                  <UInputNumber id="gw-se-max" v-model="form.ticketMax" :min="1" :max="100000" placeholder="Inconnu" class="w-full" />
                </UFormField>
              <UFormField label="Tirage" :help="pastMode ? 'Facultatif : heure du tirage, sinon la date du giveaway' : 'Un rappel est envoyé 5 min avant si la présence est requise'">
                <UInput id="gw-draw" v-model="form.drawAt" type="datetime-local" class="w-full" />
              </UFormField>
              </div>
            <UFormField
              v-if="form.kind === 'command' || form.kind === 'ticket' || form.kind === 'streamelements'"
              label="Commande à taper"
              :help="form.kind !== 'streamelements' && !form.command.trim() ? 'Pas encore connue ? Laisse vide : elle sera remplie dès que la détection la repère dans le chat.' : undefined"
            >
              <UInput id="gw-command" v-model="form.command" placeholder="!coffre" class="w-full font-mono" />
            </UFormField>
            </template>

            <template v-else>
            <UFormField
              v-if="form.kind === 'command' || form.kind === 'ticket' || form.kind === 'streamelements'"
              label="Commande à taper"
              :help="form.kind !== 'streamelements' && !form.command.trim() ? 'Pas encore connue ? Laisse vide : elle sera remplie dès que la détection la repère dans le chat.' : undefined"
            >
              <UInput id="gw-command" v-model="form.command" placeholder="!coffre" class="w-full font-mono" />
            </UFormField>
            <div v-if="form.kind === 'ticket'" class="grid grid-cols-2 gap-4">
              <UFormField label="Tickets">
                <USelect
                  id="gw-ticket"
                  v-model="form.ticket"
                  :items="[{ label: 'Un seul par personne', value: 'unique' }, { label: 'Plusieurs par personne', value: 'multi' }, { label: 'On ne sait pas', value: 'unknown' }]"
                  class="w-full"
                />
              </UFormField>
              <UFormField v-if="form.ticket === 'multi'" label="Tickets maximum par personne">
                <UInputNumber id="gw-ticket-max" v-model="form.ticketMax" :min="1" :max="100000" placeholder="Inconnu" class="w-full" />
              </UFormField>
            </div>

            <UFormField
              v-if="form.kind === 'channelpoints' && form.channel.trim()"
              label="Récompenses de la chaîne"
              :help="rewardsError ?? (rewardsLoading ? undefined : rewards.length ? 'Choisir la récompense du giveaway remplit son nom et son coût' : 'Cette chaîne ne propose aucune récompense en ce moment')"
            >
              <USelectMenu
                v-model="chosenReward"
                :items="rewardItems"
                value-key="value"
                :loading="rewardsLoading"
                :disabled="!rewards.length"
                :search-input="{ placeholder: 'Rechercher une récompense' }"
                placeholder="Choisir dans la liste de la chaîne"
                icon="i-lucide-coins"
                class="w-full"
              />
            </UFormField>
            <div v-if="form.kind === 'channelpoints'" class="grid grid-cols-2 gap-4">
              <UFormField label="Récompense à acheter">
                <UInput id="gw-reward" v-model="form.rewardName" placeholder="Ticket giveaway" class="w-full" />
              </UFormField>
              <UFormField label="Coût en points de chaîne">
                <UInputNumber id="gw-reward-cost" v-model="form.rewardCost" :min="0" placeholder="Inconnu" class="w-full" />
              </UFormField>
            </div>

              <div class="max-w-xs">
              <UFormField label="Tirage" :help="pastMode ? 'Facultatif : heure du tirage, sinon la date du giveaway' : 'Un rappel est envoyé 5 min avant si la présence est requise'">
                <UInput id="gw-draw" v-model="form.drawAt" type="datetime-local" class="w-full" />
              </UFormField>
              </div>
            </template>
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

            <!-- Communs à tous les types de giveaway -->
            <USeparator label="Pour tous les types de giveaway" :ui="{ label: 'text-xs text-muted' }" />
              <UFormField v-if="giveaway || pastMode" label="Gagnants" :help="pastMode ? 'Autant que nécessaire, avec leur chance de gagner si elle est connue. Facultatif : le giveaway est enregistré terminé' : 'Autant que nécessaire, avec leur chance de gagner si elle est connue. Tous retirés : le giveaway est rouvert'" class="max-w-sm">
                <div class="flex flex-col gap-2">
                  <!-- Un gagnant par ligne : pseudo, chance de gagner, retrait -->
                  <div v-for="w in form.winners" :key="w.login" class="flex items-center gap-2">
                    <UIcon name="i-lucide-trophy" class="size-4 text-success shrink-0" />
                    <span class="font-medium flex-1 truncate">{{ w.login }}</span>
                    <UInput v-model="w.chance" size="sm" placeholder="Chance %" class="w-28" :aria-label="`Chance de gagner de ${w.login}`">
                      <template #trailing><span class="text-xs text-muted">%</span></template>
                    </UInput>
                    <UButton icon="i-lucide-x" size="xs" color="neutral" variant="ghost" :aria-label="`Retirer ${w.login} des gagnants`" @click="removeWinner(w.login)" />
                  </div>
                  <div class="flex gap-2">
                    <UInput id="gw-winner" v-model="winnerInput" icon="i-lucide-trophy" :placeholder="form.winners.length ? 'Autre gagnant' : 'Pas encore tiré'" class="flex-1" @keydown.enter.prevent="addWinner" />
                    <UButton label="Ajouter" icon="i-lucide-plus" color="neutral" variant="soft" :disabled="!winnerInput.trim()" @click="addWinner" />
                  </div>
                </div>
              </UFormField>
            <UFormField label="Notes du giveaway">
              <UTextarea id="gw-notes" v-model="form.notes" :rows="3" autoresize placeholder="Lot, règles particulières de ce giveaway…" class="w-full" />
            </UFormField>
          </div>

          <!-- Cadeaux -->
          <div v-show="section === 'gifts'" class="flex flex-col gap-3">
            <div class="flex items-center gap-2">
              <span class="text-sm font-medium flex-1">Cadeaux <span v-if="form.giftIds.length" class="text-muted font-normal">({{ form.giftIds.length }})</span></span>
              <UInput id="gw-gift-search" v-model="giftSearch" icon="i-lucide-search" placeholder="Rechercher" size="sm" class="w-48" />
              <UButton label="Nouveau cadeau" icon="i-lucide-plus" size="sm" color="neutral" variant="soft" @click="giftFormOpen = true" />
            </div>
            <div v-if="gifts.length" class="flex flex-col gap-3 max-h-[380px] overflow-y-auto p-0.5">
              <section v-for="cat in giftSections" :key="cat.id ?? ''" class="flex flex-col gap-1.5">
                <div class="flex items-center gap-1.5 text-sm font-semibold sticky top-0 bg-default z-10 py-0.5">
                  <img v-if="cat.id" :src="boxArt(null, cat.id)" alt="" class="w-4 h-5 rounded-sm object-cover">
                  <UIcon v-else name="i-lucide-package" class="size-4 text-muted" />
                  {{ cat.name }}
                  <UBadge v-if="cat.live" label="en live" color="success" variant="subtle" size="sm" />
                </div>
                <div v-for="grp in cat.groups" :key="grp.id ?? 'none'" class="flex flex-col gap-1 pl-2">
                  <div class="flex items-center gap-1 text-xs text-muted">
                    <UIcon :name="grp.id === null ? 'i-lucide-folder-x' : 'i-lucide-folder'" class="size-3.5" />{{ grp.name }}
                  </div>
                  <div class="grid grid-cols-[repeat(auto-fill,minmax(170px,1fr))] gap-2">
                    <button
                      v-for="g in grp.gifts"
                      :key="g.id"
                      type="button"
                      class="relative flex items-center gap-2 border rounded-md p-1.5 text-left text-sm transition-colors"
                      :class="form.giftIds.includes(g.id) ? 'border-primary bg-primary/10' : 'border-default hover:border-primary/50'"
                      :aria-pressed="form.giftIds.includes(g.id)"
                      @click="toggleGift(g.id)"
                    >
                      <img v-if="g.image" :src="g.image" referrerpolicy="no-referrer" alt="" class="size-10 rounded object-contain bg-elevated shrink-0">
                      <span v-else class="size-10 rounded bg-elevated shrink-0 flex items-center justify-center"><UIcon name="i-lucide-gift" class="size-4 text-muted" /></span>
                      <span class="flex flex-col min-w-0">
                        <span class="leading-tight line-clamp-2">{{ g.title }}</span>
                        <span v-if="g.gameName" class="text-xs text-muted truncate">{{ g.gameName }}</span>
                      </span>
                      <UIcon v-if="suggested.has(g.id)" name="i-lucide-sparkles" class="absolute top-1 right-1 size-3.5 text-primary" title="Suggéré : le titre du cadeau ou un mot-clé apparaît dans le titre, la description, le message épinglé, le giveaway StreamElements ou la réponse d'un bot à !giveaway" />
                    </button>
                  </div>
                </div>
              </section>
              <p v-if="giftSections.length === 0" class="text-sm text-muted">Aucun cadeau ne correspond.</p>
            </div>
            <p v-else class="text-sm text-muted">Catalogue vide : ajoute un cadeau avec « Nouveau cadeau ».</p>
          </div>

          <!-- Conditions : une par ligne -->
          <div v-show="section === 'conditions'" class="flex flex-col gap-3">
            <p class="text-sm text-muted">
              <template v-if="streamerOnly">Ses conditions habituelles : elles pré-remplissent ses nouveaux giveaways (chacun garde ensuite les siennes). « ? » : on ne sait pas.</template>
              <template v-else>Ce qu'il faut pour participer. « ? » : on ne sait pas.</template>
            </p>
            <div v-for="c in CONDITIONS" :key="c.key" class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border border-default rounded-lg px-4 py-3">
              <span class="inline-flex items-center gap-2.5">
                <UIcon :name="c.icon" class="size-5 text-muted" />{{ c.label }}
              </span>
              <UFieldGroup>
                <UButton
                  v-for="o in TRI_OPTIONS"
                  :key="o.value"
                  :label="o.label"
                  :title="o.title"
                  size="sm"
                  class="w-12 justify-center"
                  :color="form[c.key] === o.value ? 'primary' : 'neutral'"
                  :variant="form[c.key] === o.value ? 'solid' : 'outline'"
                  :aria-pressed="form[c.key] === o.value"
                  @click="form[c.key] = o.value"
                />
              </UFieldGroup>
              <!-- Précisions : ancienneté du follow, durée de présence -->
              <span v-if="c.key === 'follow' && form.follow === 'yes'" class="basis-full flex flex-wrap items-center justify-end gap-2 text-sm text-muted">
                depuis au moins
                <UInput
                  id="gw-follow-duration"
                  v-model="form.followDuration"
                  placeholder="ex. 7j, 2h 30min"
                  size="sm"
                  class="w-36"
                  :color="followDurationError ? 'error' : undefined"
                  :highlight="!!followDurationError"
                  :title="followDurationError ?? 'j : jours, h : heures, min : minutes, s : secondes, combinables (2h 30min). Vide : pas de minimum'"
                />
                <span v-if="followDurationError" class="basis-full text-right text-xs text-error">{{ followDurationError }}</span>
              </span>
              <!-- Présence requise : réponse écrite dans le chat si un de nos comptes gagne (fiche du streamer) -->
              <div v-if="c.key === 'presence' && form.presence === 'yes'" class="basis-full flex flex-col gap-2 pt-1">
                <UFormField
                  label="Réponse automatique si tu gagnes"
                  :help="`Écrite dans le chat par le compte gagnant, ${streamerWinDelay ?? WIN_REPLY_DELAY} s après l'annonce (annulable). Vide : pas de réponse. Gardée dans la fiche de ${form.channel || 'ce streamer'}, sur ce PC seulement.`"
                >
                  <UInput id="gw-win-reply" v-model="streamerWinReply" :maxlength="400" placeholder="Merci beaucoup, je suis là !" class="w-full" />
                </UFormField>
                <span v-if="streamerWinReply.trim()" class="flex items-center justify-end gap-2 text-sm text-muted">
                  après
                  <UInputNumber id="gw-win-delay" v-model="streamerWinDelay" :min="0" :max="600" :placeholder="String(WIN_REPLY_DELAY)" size="sm" class="w-28" />
                  seconde(s)
                </span>
              </div>
              <span v-if="c.key === 'chatTime' && form.chatTime === 'yes'" class="basis-full flex items-center justify-end gap-2 text-sm text-muted">
                au moins
                <UInputNumber id="gw-chat-minutes" v-model="form.chatMinutes" :min="1" :max="1440" placeholder="—" size="sm" class="w-28" />
                minute(s)
              </span>
            </div>
          </div>
        </div>
      </form>
    </template>
    <template #footer>
      <div class="flex justify-end items-center gap-2 w-full">
        <!-- Giveaway terminé : enregistré et rouvert (ouvert si la chaîne est en live, sinon en
             pause jusqu'à son prochain live ; gagnant retiré) -->
        <UTooltip
          v-if="closed"
          :text="`Enregistre et rouvre ce giveaway${giveaway?.winners.length ? ` (${giveaway.winners.length > 1 ? 'les gagnants sont retirés' : `le gagnant ${giveaway.winners[0]} est retiré`})` : ''} : ouvert si la chaîne est en live, sinon en pause jusqu'à son prochain live`"
        >
          <UButton label="Rouvrir" icon="i-lucide-rotate-ccw" color="neutral" variant="soft" class="mr-auto" :loading="saving && reopen" @click="reopenAndSave" />
        </UTooltip>
        <UButton label="Annuler" color="neutral" variant="ghost" @click="open = false" />
        <UButton type="submit" form="giveaway-form" :label="editId !== null || streamerOnly ? 'Enregistrer' : 'Ajouter'" :loading="saving" />
      </div>
    </template>
  </UModal>
  <GiftFormModal v-model:open="giftFormOpen" :gift="null" @saved="onGiftCreated" />
</template>
