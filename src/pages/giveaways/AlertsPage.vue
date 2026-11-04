<script setup lang="ts">
import RefreshButton from '../../components/RefreshButton.vue'
import ChannelCell from '../../components/ChannelCell.vue'
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { formatDuration } from '../../utils/duration'
import { invoke } from '@tauri-apps/api/core'
import { useRouter } from 'vue-router'
import { useSnapshot } from '../../composables/useSnapshot'
import { useOpenExternal } from '../../utils/external'
import { useWatchStream } from '../../utils/viewer'
import { channelUrl, formatNumber, timeAgo } from '../../utils/format'
import type { GiveawayKind, GiveawayView, ParticipationResult } from '../../types'
import ChannelFlag from '../../components/ChannelFlag.vue'
import GiveawayConditions from '../../components/GiveawayConditions.vue'
import GiveawayFormModal from '../../components/GiveawayFormModal.vue'
import ImportPastModal from '../../components/ImportPastModal.vue'
import ChatLogModal from '../../components/ChatLogModal.vue'
import { useGiftCatalog } from '../../composables/useGiftCatalog'
import { GIVEAWAY_KIND, GIVEAWAY_KIND_ITEMS, formatDrawAt } from '../../utils/giveaways'
import SolidIcon from '../../components/SolidIcon.vue'
import { useGiftPreview } from '../../composables/useGiftPreview'
import ConditionHeaders from '../../components/conditions/ConditionHeaders.vue'
import ConditionCells from '../../components/conditions/ConditionCells.vue'
import { useConditionActions } from '../../composables/useConditionActions'
import { useAction } from '../../composables/useAction'

const { snapshot, accounts } = useSnapshot()
const toast = useToast()
const router = useRouter()
const openExternal = useOpenExternal()
const watchStream = useWatchStream()

const view = computed(() => snapshot.value?.giveaways)
const giveaways = computed(() => view.value?.giveaways ?? [])
// Deux tableaux : en cours (ouverts, puis en pause) et passés (terminés)
const tables = computed(() => [
  {
    key: 'current',
    title: 'Giveaways en cours',
    list: [...giveaways.value.filter(g => g.status === 'open'), ...giveaways.value.filter(g => g.status === 'paused')],
    empty: 'Aucun giveaway en cours. Ils apparaissent à la détection, ou avec « Nouveau giveaway ».'
  },
  {
    key: 'past',
    title: 'Giveaways passés',
    list: past.value?.giveaways ?? [],
    empty: pastFilterCount.value ? 'Aucun giveaway passé ne correspond aux filtres.' : 'Aucun giveaway terminé pour l\'instant.'
  }
])

// Giveaways passés : lus page par page (PAST_PER_PAGE), relus quand un giveaway change
const PAST_PER_PAGE = 25
const pastPage = ref(1)
const past = ref<{ giveaways: GiveawayView[], total: number } | null>(null)
let pastTimer: ReturnType<typeof setTimeout> | null = null
let pastSeq = 0
// Filtres des giveaways passés (fenêtre « Filtres ») : période (date du tirage, sinon de la
// détection), chaîne, types, gagnant, cadeaux ; appliqués par la base, sur tous les terminés
const pastFilterOpen = ref(false)
const emptyPastFilter = () => ({ from: '', to: '', channel: '', kinds: [] as GiveawayKind[], winner: '', giftIds: [] as number[] })
const pastFilter = ref(emptyPastFilter())
const pastFilterCount = computed(() => {
  const f = pastFilter.value
  return [f.from || f.to, f.channel.trim(), f.kinds.length, f.winner.trim(), f.giftIds.length].filter(Boolean).length
})
const giftFilterItems = computed(() => catalogGifts.value.map(g => ({ label: g.title, value: g.id })))
// Streamers des giveaways passés (autocomplétion du filtre), relus à l'ouverture des filtres
const pastChannels = ref<string[]>([])
const channelFilterItems = computed(() => [{ label: 'Tous les streamers', value: '' }, ...pastChannels.value.map(c => ({ label: c, value: c }))])
watch(pastFilterOpen, (isOpen) => {
  if (!isOpen) return
  invoke<string[]>('past_giveaway_channels').then((list) => { pastChannels.value = list }).catch(() => {})
})
// Dates du formulaire (jour local) → bornes RFC 3339, journées entières comprises
function pastFilterArgs() {
  const f = pastFilter.value
  return {
    from: f.from ? new Date(`${f.from}T00:00:00`).toISOString() : null,
    to: f.to ? new Date(`${f.to}T23:59:59.999`).toISOString() : null,
    channel: f.channel.trim() || null,
    kinds: f.kinds,
    winner: f.winner.trim().replace(/^@/, '') || null,
    giftIds: f.giftIds
  }
}
let pastFilterTimer: ReturnType<typeof setTimeout> | null = null
watch(pastFilter, () => {
  if (pastFilterTimer) clearTimeout(pastFilterTimer)
  pastFilterTimer = setTimeout(() => {
    if (pastPage.value !== 1) pastPage.value = 1
    else void loadPast()
  }, 300)
}, { deep: true })

async function loadPast() {
  const seq = ++pastSeq
  try {
    const result = await invoke<{ giveaways: GiveawayView[], total: number }>('past_giveaways', { page: pastPage.value, perPage: PAST_PER_PAGE, filter: pastFilterArgs() })
    if (seq !== pastSeq) return
    past.value = result
    // Page devenue vide (giveaways supprimés ou rouverts) : dernière page existante
    const last = Math.max(1, Math.ceil(result.total / PAST_PER_PAGE))
    if (pastPage.value > last) pastPage.value = last
  } catch (e) {
    if (seq === pastSeq) toast.add({ title: 'Giveaways passés non lus', description: String(e), color: 'error' })
  }
}
watch(pastPage, () => { void loadPast() })
function reloadPastSoon() {
  if (pastTimer) clearTimeout(pastTimer)
  pastTimer = setTimeout(() => { void loadPast() }, 300)
}
// Relu quand un giveaway récent change d'état ou qu'un giveaway clos change (pas à chaque
// participation ou preuve du chat des giveaways en cours)
const pastKey = computed(() => JSON.stringify(giveaways.value.map(g => g.status === 'closed' ? g : [g.id, g.status])))
watch(pastKey, reloadPastSoon, { immediate: true })
onBeforeUnmount(() => {
  if (pastTimer) clearTimeout(pastTimer)
  if (pastFilterTimer) clearTimeout(pastFilterTimer)
})
// Live en cours d'une chaîne surveillée (catégorie, spectateurs), indexé une fois par mise à jour
const liveByLogin = computed(() => new Map((view.value?.channels ?? []).map(c => [c.login, c.live])))
const liveOf = (channel: string) => liveByLogin.value.get(channel) ?? null
const avatarByLogin = computed(() => new Map((view.value?.channels ?? []).map(c => [c.login, c.profileImage])))
const avatarOf = (channel: string) => avatarByLogin.value.get(channel) ?? null

// Conditions : follow, bonjour, Discord des giveaways ouverts ou en pause, et follow des chaînes
// de la page affichée des giveaways passés (cœur : suivie ou non aujourd'hui)
const cond = useConditionActions(() => [
  ...giveaways.value.filter(g => g.status !== 'closed'),
  ...(past.value?.giveaways ?? []).map(g => ({ ...g, requireGreeting: null }))
])

// Ignorer, Vu, Clore, Supprimer : erreur affichée, bouton occupé pendant l'action
const { run, busy } = useAction()
const dismiss = (g: GiveawayView) => run(`dismiss-${g.id}`, 'Alerte non ignorée', () => invoke('dismiss_giveaway', { id: g.id }))
const dismissMention = (id: number) => run(`mention-${id}`, 'Mention non marquée comme vue', () => invoke('dismiss_mention', { id }))
const closeGiveaway = (g: GiveawayView) => run(`close-${g.id}`, 'Giveaway non clos', () => invoke('close_giveaway', { id: g.id }))
const toDelete = ref<GiveawayView | null>(null)
async function confirmDelete() {
  const g = toDelete.value
  if (!g) return
  if (await run('delete', 'Giveaway non supprimé', () => invoke('delete_giveaway', { id: g.id }))) {
    toDelete.value = null
    reloadPastSoon()
  }
}

// Journal du chat d'une chaîne, ouvert sur la section du giveaway
const logChannel = ref<string | null>(null)
const logGiveaway = ref<number | null>(null)
const logOpen = ref(false)
function openLog(login: string, giveawayId: number) {
  logChannel.value = login
  logGiveaway.value = giveawayId
  logOpen.value = true
}

// Commande : copiée d'un clic, modifiable dans une mini-fenêtre
async function copyCommand(command: string) {
  try {
    await navigator.clipboard.writeText(command)
    toast.add({ title: `${command} copiée`, color: 'success', icon: 'i-lucide-clipboard-check' })
  } catch {
    toast.add({ title: 'Copie impossible', color: 'error' })
  }
}
const editingCommand = ref<number | null>(null)
const commandDraft = ref('')
function startEditCommand(g: GiveawayView) {
  editingCommand.value = g.id
  commandDraft.value = g.command ?? ''
}
async function saveCommand(g: GiveawayView) {
  try {
    await invoke('set_giveaway_command', { id: g.id, command: commandDraft.value.trim() || null })
    editingCommand.value = null
  } catch (e) {
    toast.add({ title: 'Commande non enregistrée', description: String(e), color: 'error' })
  }
}

const mentions = computed(() => view.value?.mentions ?? [])
// File d'alertes : giveaways ouverts (pas en pause) pas encore traités, et mentions
const alerts = computed(() => giveaways.value.filter(g => g.open && !g.handled))

// Rafraîchit les « il y a … »
const now = ref(Date.now())
let clock: ReturnType<typeof setInterval> | null = null
onMounted(() => { clock = setInterval(() => { now.value = Date.now() }, 30_000) })
onBeforeUnmount(() => { if (clock) clearInterval(clock) })

// --- Participation ---
// Comptes cochés par giveaway (tous par défaut)
const chosen = reactive<Record<number, number[]>>({})
function chosenFor(g: GiveawayView): number[] {
  return chosen[g.id] ?? accounts.value.map(a => a.id)
}
function toggleAccount(g: GiveawayView, id: number) {
  const list = chosenFor(g)
  chosen[g.id] = list.includes(id) ? list.filter(x => x !== id) : [...list, id]
}
function entered(g: GiveawayView, id: number) {
  return g.entriesView.some(e => e.accountId === id)
}
const participating = ref<number | null>(null)
// Gagnant qui est un de nos comptes
const ourLogins = computed(() => new Set(accounts.value.map(a => a.login.toLowerCase())))
const isOurs = (login: string) => ourLogins.value.has(login.toLowerCase())
// Chance de gagner d'un gagnant (celle du premier aussi gardée à part)
const chanceOf = (g: GiveawayView, login: string, i: number) => g.winnerChances[login.toLowerCase()] ?? (i === 0 ? g.winnerChance : null)
// Participations ; StreamElements : chacun de nos comptes, à 0 ticket s'il n'en a pas
function participationsOf(g: GiveawayView) {
  if (g.kind !== 'streamelements') return g.entriesView
  return accounts.value.map(a => g.entriesView.find(e => e.accountId === a.id)
    ?? { accountId: a.id, login: a.login, at: '', auto: false, tickets: 0 })
}
// Chaîne dans la liste StreamElements d'un compte
const inSeList = (accountId: number, channel: string) =>
  !!accounts.value.find(a => a.id === accountId)?.channels.some(c => c.inSe && c.login.toLowerCase() === channel.toLowerCase())
// Participation à un giveaway StreamElements : clos → sa page sur StreamElements ; en cours → page
// StreamElements de l'application, la chaîne ajoutée d'abord à la liste du compte au besoin
async function openSeParticipation(g: GiveawayView, accountId: number) {
  const closedUrl = seClosedUrl(g)
  if (closedUrl) return openExternal(closedUrl)
  if (!inSeList(accountId, g.channel)) {
    try {
      await invoke('add_channel', { accountId, login: g.channel, list: 'se' })
      const login = accounts.value.find(a => a.id === accountId)?.login ?? ''
      toast.add({ title: `${g.channel} ajoutée à la page StreamElements`, description: login, color: 'success', icon: 'i-lucide-check' })
    } catch (e) {
      toast.add({ title: 'Chaîne non ajoutée', description: String(e), color: 'error' })
      return
    }
  }
  void router.push({ path: '/streamelements', query: { channels: g.channel } })
}
// Chaîne ajoutée à la page StreamElements d'un compte (ses points relevés), sans quitter la page
function addToSe(g: GiveawayView, accountId: number) {
  const login = accounts.value.find(a => a.id === accountId)?.login ?? ''
  void run(`se-add-${g.id}-${accountId}`, 'Chaîne non ajoutée', async () => {
    await invoke('add_channel', { accountId, login: g.channel, list: 'se' })
    toast.add({ title: `${g.channel} ajoutée à la page StreamElements`, description: `Points relevés avec ${login}`, color: 'success', icon: 'i-lucide-check' })
  })
}
// Giveaway StreamElements terminé : adresse de sa page sur StreamElements
const seClosedUrl = (g: GiveawayView) => g.kind === 'streamelements' && g.status === 'closed' && g.url?.toLowerCase().startsWith('https://streamelements.com/') ? g.url : null

async function participate(g: GiveawayView, ids = chosenFor(g)) {
  const todo = ids.filter(id => !entered(g, id))
  if (todo.length === 0) return
  participating.value = g.id
  try {
    const results = await invoke<ParticipationResult[]>('participate_giveaway', { giveawayId: g.id, accountIds: todo })
    const failed = results.filter(r => !r.ok)
    const done = g.kind === 'channelpoints' ? `« ${g.rewardName} » acheté sur ${g.channel}` : `${g.command} envoyé sur ${g.channel}`
    if (failed.length === 0) {
      toast.add({ title: done, description: results.map(r => r.login).join(', '), color: 'success', icon: 'i-lucide-check' })
    } else {
      toast.add({ title: g.kind === 'channelpoints' ? 'Achat incomplet' : 'Participation incomplète', description: failed.map(r => `${r.login} : ${r.message}`).join(' · '), color: 'error' })
    }
  } catch (e) {
    toast.add({ title: 'Participation impossible', description: String(e), color: 'error' })
  } finally {
    participating.value = null
  }
}

// Points de chaîne : achat de la récompense, confirmé d'abord (des points sont dépensés)
const toBuy = ref<{ giveaway: GiveawayView, ids: number[] } | null>(null)
function askBuy(g: GiveawayView, ids = chosenFor(g)) {
  const todo = ids.filter(id => !entered(g, id))
  if (todo.length) toBuy.value = { giveaway: g, ids: todo }
}
const buyLogins = computed(() => (toBuy.value?.ids ?? []).map(id => accounts.value.find(a => a.id === id)?.login ?? '?').join(', '))
async function confirmBuy() {
  const b = toBuy.value
  if (!b) return
  toBuy.value = null
  await participate(b.giveaway, b.ids)
}
const canBuy = (g: GiveawayView) => g.kind === 'channelpoints' && g.status !== 'closed' && !!g.rewardName && accounts.value.some(a => !entered(g, a.id))

function evidenceText(g: GiveawayView) {
  const e = g.evidence
  if (!e) return ''
  return `${e.count} messages sur les ${e.window} derniers, par ${e.users.length} pseudos différents : ${e.users.slice(0, 8).join(', ')}${e.users.length > 8 ? '…' : ''}`
}

const { byId: giftById, gifts: catalogGifts } = useGiftCatalog()
const { openGift } = useGiftPreview()

// Formulaire : null = nouveau giveaway
const formOpen = ref(false)
const editing = ref<GiveawayView | null>(null)
// Ancien giveaway ajouté après coup (sous les giveaways passés), un à un ou par lot (JSON)
const formPast = ref(false)
const importOpen = ref(false)
// Ajout, modification ou import fermé : giveaways passés relus (un ancien giveaway n'est pas
// forcément parmi les plus récents)
watch([formOpen, importOpen], ([form, imp], [wasForm, wasImp]) => {
  if ((wasForm && !form) || (wasImp && !imp)) reloadPastSoon()
})
function openPastForm() {
  editing.value = null
  formPast.value = true
  formOpen.value = true
}
function openForm(g: GiveawayView | null) {
  formPast.value = false
  editing.value = g
  formOpen.value = true
}

function share(g: GiveawayView) {
  return g.evidence ? Math.round(g.evidence.count / Math.max(1, g.evidence.window) * 100) : null
}
</script>

<template>
  <div class="p-6 flex flex-col gap-6">
    <div class="flex items-center gap-3">
      <h1 class="text-xl font-semibold flex-1">Alertes</h1>
      <UButton label="Nouveau giveaway" icon="i-lucide-plus" @click="openForm(null)" />
    </div>

    <UAlert
      v-if="accounts.length === 0"
      title="Connecte d'abord un compte Twitch"
      :actions="[{ label: 'Comptes', to: '/accounts' }]"
      color="neutral"
      variant="subtle"
    />

    <!-- File d'alertes -->
    <section class="flex flex-col gap-2">
      <div class="flex items-center gap-2">
        <h2 class="text-sm font-semibold uppercase text-muted tracking-wide flex-1">
          File d'alertes <span v-if="alerts.length + mentions.length" class="text-primary">({{ alerts.length + mentions.length }})</span>
        </h2>
        <RefreshButton :action="() => invoke('refresh_giveaways')" />
      </div>
      <p v-if="alerts.length === 0 && mentions.length === 0" class="text-sm text-muted">
        Rien en attente. Les giveaways détectés dans les chaînes surveillées et les mentions de tes pseudos apparaîtront ici, avec une notification Windows.
      </p>

      <div v-for="g in alerts" :key="g.id" class="border border-primary/40 bg-primary/5 rounded-lg px-4 py-3 flex flex-col gap-2">
        <div class="flex items-center gap-3 flex-wrap">
          <UIcon name="i-lucide-gift" class="size-5 text-primary" />
          <span class="font-medium">
            {{ g.kind === 'streamelements' ? 'Giveaway StreamElements' : 'Giveaway' }} sur
            <ChannelFlag :login="g.channel" class="inline-block align-middle mx-0.5" />
            <button class="hover:underline" @click="openExternal(channelUrl(g.channel))">{{ g.channel }}</button>
          </span>
          <span v-if="liveOf(g.channel)" class="inline-flex items-center gap-1.5 text-xs text-muted">
            {{ liveOf(g.channel)?.gameName ?? 'Sans catégorie' }}
            <span class="inline-flex items-center gap-0.5 text-error"><UIcon name="i-lucide-eye" class="size-3.5" />{{ formatNumber(liveOf(g.channel)?.viewers ?? 0) }}</span>
          </span>
          <UBadge :label="GIVEAWAY_KIND[g.kind].label" :color="GIVEAWAY_KIND[g.kind].color" variant="subtle" size="sm" />
          <UTooltip v-if="g.command" text="Copier la commande">
            <button type="button" class="font-mono bg-elevated px-1.5 py-0.5 rounded hover:text-primary" @click="copyCommand(g.command)">{{ g.command }}</button>
          </UTooltip>
          <!-- Follow requis : cœur rempli si suivie par tous tes comptes, sinon cliquable pour suivre -->
          <UTooltip v-if="g.requireFollow" :text="cond.followTooltip(g)">
            <button
              v-if="cond.notFollowing(g.channel).length"
              type="button"
              class="inline-flex items-center gap-0.5 text-warning hover:text-pink-500 disabled:opacity-50"
              :disabled="cond.following === g.channel"
              :aria-label="`${cond.followLabel(g)}. Suivre ${g.channel}`"
              @click="cond.followChannel(g.channel)"
            >
              <UIcon :name="cond.following === g.channel ? 'i-lucide-loader-circle' : 'i-lucide-heart'" class="size-4" :class="cond.following === g.channel ? 'animate-spin' : ''" />
              <span v-if="g.followMinSecs" class="text-xs font-medium">{{ formatDuration(g.followMinSecs) }}</span>
            </button>
            <span
              v-else
              class="inline-flex items-center gap-0.5 rounded focus-visible:outline-2 focus-visible:outline-primary"
              :class="cond.followOk(g) ? 'text-success' : cond.tooRecent(g).length ? 'text-warning' : 'text-pink-600 dark:text-pink-400'"
              tabindex="0"
              role="img"
              :aria-label="cond.followLabel(g)"
            >
              <SolidIcon v-if="cond.followOk(g)" name="heart" class="size-4" />
              <UIcon v-else name="i-lucide-heart" class="size-4" />
              <span v-if="g.followMinSecs" class="text-xs font-medium">{{ formatDuration(g.followMinSecs) }}</span>
            </span>
          </UTooltip>
          <GiveawayConditions :details="{ ...g, requireFollow: g.requireFollow ? null : g.requireFollow }" :kind="g.kind" />
          <span v-if="g.drawAt" class="text-sm text-muted inline-flex items-center gap-1"><UIcon name="i-lucide-clock" class="size-4" />Tirage {{ formatDrawAt(g.drawAt) }}</span>
          <UTooltip v-if="g.evidence" :text="evidenceText(g)">
            <span class="text-sm text-muted underline decoration-dotted cursor-help rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" :aria-label="evidenceText(g)">{{ share(g) }} % du chat</span>
          </UTooltip>
          <span class="text-xs text-muted ml-auto">{{ timeAgo(g.detectedAt, now) }}</span>
          <!-- Comme les mentions : retire la carte de la file, le giveaway reste en cours -->
          <UButton
            label="Vu"
            size="xs"
            color="neutral"
            variant="soft"
            title="Retire cette carte de la file d'alertes. Le giveaway reste dans les giveaways en cours (rien n'est supprimé ni clos)"
            :loading="busy(`dismiss-${g.id}`)"
            @click="dismiss(g)"
          />
        </div>
        <!-- Le giveaway est déjà enregistré : l'alerte ne fait que le signaler -->
        <div class="text-sm text-muted flex items-center gap-2">
          <UIcon name="i-lucide-circle-check" class="size-4 text-success shrink-0" />
          <span>{{ g.kind === 'streamelements' ? 'Giveaway StreamElements ajouté' : 'Giveaway ajouté' }} aux giveaways en cours{{ g.status === 'paused' ? ' (en pause)' : '' }}.</span>
        </div>
        <!-- Réponse d'un bot à !giveaway… sur la chaîne -->
        <div v-for="r in view?.botReplies[g.channel] ?? []" :key="r.command" class="text-sm flex items-start gap-2">
          <UIcon name="i-lucide-message-square-text" class="size-4 text-muted mt-0.5 shrink-0" />
          <span><span class="text-muted">Réponse de {{ r.bot }} à <span class="font-mono">{{ r.command }}</span> :</span> {{ r.message }}</span>
        </div>
        <div class="flex items-center gap-2 flex-wrap">
          <span v-if="(g.kind === 'command' || g.kind === 'ticket') && !g.command" class="text-sm text-muted">
            <UIcon name="i-lucide-hourglass" class="size-3.5 align-middle" /> Commande pas encore connue : elle sera remplie dès que la détection la repère dans le chat.
          </span>
          <template v-else-if="g.kind === 'command' || g.kind === 'ticket'">
            <span class="text-sm text-muted">Participer avec</span>
            <UButton
              v-for="a in accounts"
              :key="a.id"
              :label="a.login"
              size="xs"
              :icon="entered(g, a.id) ? 'i-lucide-check' : chosenFor(g).includes(a.id) ? 'i-lucide-square-check' : 'i-lucide-square'"
              :color="entered(g, a.id) ? 'success' : 'neutral'"
              :variant="chosenFor(g).includes(a.id) || entered(g, a.id) ? 'soft' : 'ghost'"
              :disabled="entered(g, a.id)"
              @click="toggleAccount(g, a.id)"
            />
            <UButton label="Participer" icon="i-lucide-send" size="sm" :loading="participating === g.id" :disabled="!chosenFor(g).some(id => !entered(g, id))" @click="participate(g)" />
          </template>
          <span v-else-if="g.kind === 'channelpoints' && !g.rewardName" class="text-sm text-muted">
            <UIcon name="i-lucide-coins" class="size-3.5 align-middle" /> Récompense à acheter pas encore renseignée : choisis-la dans la fiche du giveaway.
          </span>
          <template v-else-if="g.kind === 'channelpoints'">
            <span class="text-sm text-muted">Acheter avec</span>
            <UButton
              v-for="a in accounts"
              :key="a.id"
              :label="a.login"
              size="xs"
              :icon="entered(g, a.id) ? 'i-lucide-check' : chosenFor(g).includes(a.id) ? 'i-lucide-square-check' : 'i-lucide-square'"
              :color="entered(g, a.id) ? 'success' : 'neutral'"
              :variant="chosenFor(g).includes(a.id) || entered(g, a.id) ? 'soft' : 'ghost'"
              :disabled="entered(g, a.id)"
              @click="toggleAccount(g, a.id)"
            />
            <UButton
              :label="`Acheter « ${g.rewardName} »${g.rewardCost !== null ? ` (${formatNumber(g.rewardCost)} pts)` : ''}`"
              icon="i-lucide-coins"
              size="sm"
              :loading="participating === g.id"
              :disabled="!chosenFor(g).some(id => !entered(g, id))"
              @click="askBuy(g)"
            />
          </template>
          <template v-else>
            <UButton v-if="g.url" label="Voir le giveaway" icon="i-lucide-external-link" size="sm" @click="openExternal(g.url)" />
            <UButton label="Acheter des tickets" icon="i-lucide-ticket" size="sm" color="neutral" variant="soft" :to="{ path: '/streamelements', query: { channels: g.channel } }" />
            <!-- Chaque compte : ses tickets s'il relève les points StreamElements de la chaîne,
                 sinon un bouton pour l'ajouter à sa page StreamElements -->
            <template v-for="e in participationsOf(g)" :key="e.accountId">
              <UButton
                v-if="!inSeList(e.accountId, g.channel)"
                :label="`${e.login} · ajouter à StreamElements`"
                icon="i-lucide-plus"
                size="sm"
                color="neutral"
                variant="outline"
                :loading="busy(`se-add-${g.id}-${e.accountId}`)"
                :title="`Relever les points StreamElements de ${g.channel} avec ${e.login}, pour acheter des tickets`"
                @click="addToSe(g, e.accountId)"
              />
              <UBadge
                v-else
                :label="`${e.login} · ${e.tickets ?? 0} ticket${(e.tickets ?? 0) > 1 ? 's' : ''}`"
                icon="i-lucide-ticket"
                :color="e.tickets ? 'success' : 'neutral'"
                variant="subtle"
                title="Points StreamElements de la chaîne relevés pour ce compte"
              />
            </template>
          </template>
          <UButton label="Regarder" icon="i-lucide-tv" size="sm" color="neutral" variant="soft" title="Ajouter le stream à la fenêtre multi-stream" @click="watchStream(g.channel)" />
          <UButton label="Modifier" icon="i-lucide-pencil" size="sm" color="neutral" variant="ghost" @click="openForm(g)" />
        </div>
      </div>

      <div v-for="m in mentions" :key="`m${m.id}`" class="border border-warning/40 bg-warning/10 rounded-lg px-4 py-3 flex items-start gap-3">
        <UIcon name="i-lucide-at-sign" class="size-5 text-warning mt-0.5" />
        <div class="flex-1 min-w-0">
          <div class="font-medium">{{ m.author }} mentionne {{ m.account }} sur <button class="hover:underline" @click="openExternal(channelUrl(m.channel))">{{ m.channel }}</button></div>
          <div class="text-sm text-muted break-words">« {{ m.message }} »</div>
        </div>
        <span class="text-xs text-muted">{{ timeAgo(m.at, now) }}</span>
        <UButton label="Vu" size="xs" color="neutral" variant="soft" :loading="busy(`mention-${m.id}`)" @click="dismissMention(m.id)" />
      </div>
    </section>

    <!-- Giveaways en cours (ouverts et en pause), puis passés : mêmes colonnes -->
    <section v-for="group in tables" :key="group.key" class="flex flex-col gap-2">
      <div class="flex items-center gap-3 flex-wrap">
        <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">
          {{ group.title }}
          <span v-if="group.key === 'past' ? past?.total : group.list.length" class="text-primary">({{ group.key === 'past' ? past?.total : group.list.length }})</span>
        </h2>
        <UPagination
          v-if="group.key === 'past' && (past?.total ?? 0) > PAST_PER_PAGE"
          v-model:page="pastPage"
          :total="past?.total ?? 0"
          :items-per-page="PAST_PER_PAGE"
          size="xs"
          :sibling-count="1"
          show-edges
        />
        <span class="flex-1" />
        <!-- Giveaways en cours : statut live et giveaways StreamElements relus ; passés : relus -->
        <RefreshButton :action="() => group.key === 'past' ? loadPast() : invoke('refresh_giveaways')" />
        <template v-if="group.key === 'past'">
          <UButton v-if="pastFilterCount" label="Effacer les filtres" icon="i-lucide-x" size="xs" color="neutral" variant="ghost" @click="pastFilter = emptyPastFilter()" />
          <UButton
            :label="pastFilterCount ? `Filtres (${pastFilterCount})` : 'Filtres'"
            icon="i-lucide-list-filter"
            size="xs"
            :color="pastFilterCount ? 'primary' : 'neutral'"
            variant="soft"
            @click="pastFilterOpen = true"
          />
        </template>
      </div>
      <div class="border border-default rounded-lg overflow-x-auto">
        <table class="w-full text-sm">
          <thead class="bg-elevated/60 text-xs uppercase text-muted">
            <tr>
              <th class="text-left px-3 py-2">Détecté</th>
              <th class="px-2 py-2"><span class="sr-only">Regarder</span></th>
              <th class="text-left px-3 py-2">Chaîne</th>
              <th class="text-left px-3 py-2">Type</th>
              <th class="text-left px-3 py-2">Commande</th>
              <th class="text-left px-3 py-2">Cadeaux</th>
              <ConditionHeaders />
              <th class="text-left px-3 py-2">Tirage</th>
              <th class="text-left px-3 py-2">Participations</th>
              <th class="text-left px-3 py-2">Gagnant</th>
              <th class="text-left px-3 py-2">État</th>
              <th class="px-3 py-2" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="g in group.list" :key="g.id" class="border-t border-default" :class="g.winnerOurs ? 'bg-success/10' : ''">
              <td class="px-3 py-2 whitespace-nowrap text-muted">{{ timeAgo(g.detectedAt, now) }}</td>
              <td class="px-2 py-2">
                <UButton v-if="liveOf(g.channel)" icon="i-lucide-tv" size="xs" color="neutral" variant="ghost" :aria-label="`Regarder ${g.channel}`" title="Regarder (fenêtre multi-stream)" @click="watchStream(g.channel)" />
              </td>
              <td class="px-3 py-2">
                <ChannelCell :login="g.channel" :avatar="avatarOf(g.channel)" :live="liveOf(g.channel)" />
              </td>
              <td class="px-3 py-2">
                <div class="flex items-center gap-1.5">
                  <UBadge :label="GIVEAWAY_KIND[g.kind].label" :color="GIVEAWAY_KIND[g.kind].color" variant="subtle" class="whitespace-nowrap" />
                  <!-- Ticket : usage unique ou multi-tickets -->
                  <GiveawayConditions v-if="g.kind === 'ticket' && g.ticketUnique !== null" :details="{ ...g, requireFollow: null, requirePresence: null, requireDiscord: null }" :kind="g.kind" />
                </div>
              </td>
              <td class="px-3 py-2 whitespace-nowrap group/cmd">
                <span class="inline-flex items-center gap-1">
                  <UTooltip v-if="g.command" text="Copier la commande">
                    <button type="button" class="font-mono hover:text-primary" @click="copyCommand(g.command)">{{ g.command }}</button>
                  </UTooltip>
                  <button v-else-if="g.url" class="text-primary hover:underline" @click="openExternal(g.url)">Lien du giveaway</button>
                  <span v-else-if="g.rewardName">{{ g.rewardName }}<template v-if="g.rewardCost !== null"> ({{ g.rewardCost }} pts)</template></span>
                  <span v-else class="text-muted">—</span>
                  <UPopover :open="editingCommand === g.id" @update:open="(v: boolean) => { if (!v) editingCommand = null }">
                    <UButton
                      icon="i-lucide-pencil"
                      size="xs"
                      color="neutral"
                      variant="ghost"
                      class="opacity-0 group-hover/cmd:opacity-100 focus-visible:opacity-100"
                      :class="editingCommand === g.id ? 'opacity-100' : ''"
                      :aria-label="`Modifier la commande du giveaway de ${g.channel}`"
                      title="Modifier la commande"
                      @click="startEditCommand(g)"
                    />
                    <template #content>
                      <form class="flex items-center gap-2 p-2" @submit.prevent="saveCommand(g)">
                        <UInput :id="`cmd-${g.id}`" v-model="commandDraft" placeholder="!join ou mot-clé" size="sm" class="w-44 font-mono" autofocus />
                        <UButton type="submit" icon="i-lucide-check" size="sm" aria-label="Enregistrer la commande" />
                      </form>
                    </template>
                  </UPopover>
                </span>
                <!-- Commande connue : le lien du giveaway en dessous (les deux informations) -->
                <button v-if="g.command && g.url" type="button" class="block text-xs text-primary hover:underline" @click="openExternal(g.url)">Lien du giveaway</button>
              </td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap items-center gap-1 min-w-24 max-w-64">
                  <UTooltip v-for="id in g.giftIds" :key="id" :text="giftById.get(id)?.title ?? ''">
                    <button type="button" class="relative cursor-zoom-in rounded focus-visible:outline-2 focus-visible:outline-primary" :aria-label="`Voir ${giftById.get(id)?.title ?? 'le cadeau'} en grand`" @click="openGift(id)">
                      <img v-if="giftById.get(id)?.image" :src="giftById.get(id)?.image ?? ''" referrerpolicy="no-referrer" alt="" class="size-8 rounded object-contain bg-elevated">
                      <span v-else class="size-8 rounded bg-elevated flex items-center justify-center"><UIcon name="i-lucide-gift" class="size-4 text-muted" /></span>
                    </button>
                  </UTooltip>
                  <span v-if="g.giftIds.length === 0" class="text-muted">—</span>
                </div>
              </td>
              <ConditionCells :target="g" :actions="cond" />
              <td class="px-3 py-2 whitespace-nowrap" :class="g.drawAt ? '' : 'text-muted'">{{ formatDrawAt(g.drawAt) }}</td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap gap-1">
                  <!-- StreamElements : giveaway clos → sa page sur StreamElements (navigateur) ; en cours
                       ou en pause → page StreamElements de l'application, filtrée sur la chaîne (la
                       chaîne y est d'abord ajoutée pour ce compte si elle n'y est pas) -->
                  <template v-for="e in participationsOf(g)" :key="e.accountId">
                  <!-- Giveaway StreamElements en cours, compte qui ne relève pas les points de la
                       chaîne : l'ajouter à sa page StreamElements -->
                  <UButton
                    v-if="g.kind === 'streamelements' && !seClosedUrl(g) && !inSeList(e.accountId, g.channel)"
                    :label="`${e.login} · ajouter à StreamElements`"
                    icon="i-lucide-plus"
                    size="xs"
                    color="neutral"
                    variant="outline"
                    class="whitespace-nowrap"
                    :loading="busy(`se-add-${g.id}-${e.accountId}`)"
                    :title="`Relever les points StreamElements de ${g.channel} avec ${e.login} (page StreamElements), pour acheter des tickets`"
                    @click="addToSe(g, e.accountId)"
                  />
                  <component
                    :is="g.kind === 'streamelements' ? 'button' : 'span'"
                    v-else
                    v-bind="g.kind === 'streamelements' ? { type: 'button' } : {}"
                    :title="seClosedUrl(g) ? 'Voir ce giveaway sur StreamElements (navigateur)' : g.kind === 'streamelements' ? (inSeList(e.accountId, g.channel) ? 'Voir sur la page StreamElements' : 'Ajouter la chaîne à la page StreamElements de ce compte') : undefined"
                    @click="g.kind === 'streamelements' && openSeParticipation(g, e.accountId)"
                  >
                    <UBadge
                      :label="e.tickets !== null ? `${e.login} · ${e.tickets} ticket${e.tickets > 1 ? 's' : ''}` : e.login"
                      :icon="e.tickets !== null ? 'i-lucide-ticket' : e.auto ? 'i-lucide-bot' : 'i-lucide-check'"
                      :color="e.tickets === 0 ? 'neutral' : 'success'"
                      variant="subtle"
                      size="sm"
                      class="whitespace-nowrap"
                      :trailing-icon="seClosedUrl(g) ? 'i-lucide-external-link' : undefined"
                      :class="g.kind === 'streamelements' ? (e.tickets === 0 ? 'hover:bg-elevated cursor-pointer' : 'hover:bg-success/20 cursor-pointer') : ''"
                      :title="e.tickets === 0 ? 'Aucun ticket pour ce compte' : e.tickets !== null ? 'Tickets StreamElements relevés sur StreamElements' : e.auto ? 'Participation automatique' : 'Participation envoyée par l\'application'"
                    />
                  </component>
                  </template>
                  <span v-if="participationsOf(g).length === 0" class="text-muted">—</span>
                </div>
              </td>
              <td class="px-3 py-2">
                <!-- Tous les gagnants (plusieurs possibles), les nôtres en vert -->
                <div v-if="g.winners.length" class="flex flex-col gap-0.5">
                  <span v-for="(w, i) in g.winners" :key="w" class="whitespace-nowrap">
                    <span :class="isOurs(w) ? 'text-success font-semibold' : ''">{{ w }}</span>
                    <span v-if="chanceOf(g, w, i)" class="text-xs text-muted"> ({{ chanceOf(g, w, i) }})</span>
                    <UBadge v-if="isOurs(w)" label="Gagné !" color="success" size="sm" class="ml-1" />
                  </span>
                </div>
                <span v-else class="text-muted">—</span>
              </td>
              <td class="px-3 py-2">
                <UBadge v-if="g.status === 'open'" label="Ouvert" color="primary" variant="subtle" />
                <UTooltip v-else-if="g.status === 'paused'" text="Live terminé, tirage pas encore fait : il reprendra au prochain live">
                  <UBadge label="En pause" color="warning" variant="subtle" />
                </UTooltip>
                <UBadge v-else label="Terminé" color="neutral" variant="subtle" />
              </td>
              <td class="px-3 py-2">
                <div class="flex justify-end gap-1">
                  <UButton
                    v-if="g.status !== 'closed' && (g.kind === 'command' || g.kind === 'ticket') && g.command && accounts.some(a => !entered(g, a.id))"
                    label="Participer"
                    size="xs"
                    :loading="participating === g.id"
                    @click="participate(g)"
                  />
                  <UButton
                    v-else-if="canBuy(g)"
                    label="Acheter"
                    icon="i-lucide-coins"
                    size="xs"
                    :title="`Acheter « ${g.rewardName} » avec tes comptes`"
                    :loading="participating === g.id"
                    @click="askBuy(g, accounts.map(a => a.id))"
                  />
                  <UButton icon="i-lucide-scroll-text" size="xs" color="neutral" variant="ghost" :aria-label="`Journal du chat de ${g.channel}`" title="Journal du chat (pendant ce giveaway)" @click="openLog(g.channel, g.id)" />
                  <UButton icon="i-lucide-pencil" size="xs" color="neutral" variant="ghost" aria-label="Modifier" @click="openForm(g)" />
                  <UButton v-if="g.status !== 'closed'" label="Clore" size="xs" color="neutral" variant="ghost" :loading="busy(`close-${g.id}`)" @click="closeGiveaway(g)" />
                  <UButton icon="i-lucide-trash-2" size="xs" color="error" variant="ghost" aria-label="Supprimer" @click="toDelete = g" />
                </div>
              </td>
            </tr>
            <tr v-if="group.list.length === 0">
              <td colspan="16" class="px-3 py-8 text-center text-muted">{{ group.empty }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <!-- Giveaways passés : en ajouter un ancien, déjà terminé, pour garder l'historique -->
      <div v-if="group.key === 'past'" class="flex items-center gap-1">
        <UButton label="Ajouter un ancien giveaway" icon="i-lucide-history" size="xs" color="neutral" variant="link" @click="openPastForm" />
        <UButton label="Importer par lot (JSON)" icon="i-lucide-file-json" size="xs" color="neutral" variant="link" @click="importOpen = true" />
      </div>
    </section>

    <GiveawayFormModal v-model:open="formOpen" :giveaway="editing" :past="formPast" />
    <ImportPastModal v-model:open="importOpen" />
    <ChatLogModal v-model:open="logOpen" :channel="logChannel" :giveaway-id="logGiveaway" />

    <UModal v-model:open="pastFilterOpen" title="Filtrer les giveaways passés" description="Appliqués tout de suite, sur tous les giveaways terminés">
      <template #body>
        <div class="flex flex-col gap-4">
          <div class="grid grid-cols-2 gap-4">
            <UFormField label="Du" help="Date du tirage, sinon de la détection">
              <UInput id="past-from" v-model="pastFilter.from" type="date" class="w-full" />
            </UFormField>
            <UFormField label="Au">
              <UInput id="past-to" v-model="pastFilter.to" type="date" class="w-full" />
            </UFormField>
          </div>
          <UFormField label="Streamer">
            <USelectMenu
              id="past-channel"
              v-model="pastFilter.channel"
              :items="channelFilterItems"
              value-key="value"
              icon="i-lucide-tv"
              :search-input="{ placeholder: 'Rechercher un streamer' }"
              placeholder="Tous les streamers"
              class="w-full"
            />
          </UFormField>
          <UFormField label="Type de giveaway">
            <USelect v-model="pastFilter.kinds" :items="GIVEAWAY_KIND_ITEMS" multiple placeholder="Tous les types" class="w-full" />
          </UFormField>
          <UFormField label="Gagnant">
            <UInput id="past-winner" v-model="pastFilter.winner" icon="i-lucide-trophy" placeholder="Pseudo du gagnant (ou une partie)" class="w-full" />
          </UFormField>
          <UFormField label="Cadeaux" help="Giveaways avec au moins un de ces cadeaux">
            <USelectMenu
              v-model="pastFilter.giftIds"
              :items="giftFilterItems"
              value-key="value"
              multiple
              :search-input="{ placeholder: 'Rechercher un cadeau' }"
              placeholder="Tous les cadeaux"
              class="w-full"
            />
          </UFormField>
        </div>
      </template>
      <template #footer>
        <div class="flex justify-between gap-2 w-full">
          <UButton label="Effacer les filtres" color="neutral" variant="ghost" :disabled="!pastFilterCount" @click="pastFilter = emptyPastFilter()" />
          <UButton :label="`Voir ${past?.total ?? 0} giveaway(s)`" @click="pastFilterOpen = false" />
        </div>
      </template>
    </UModal>

    <UModal :open="toBuy !== null" title="Acheter la récompense ?" @update:open="v => { if (!v) toBuy = null }">
      <template #body>
        <p class="text-sm">
          « {{ toBuy?.giveaway.rewardName }} » sera achetée sur {{ toBuy?.giveaway.channel }}<template v-if="toBuy?.giveaway.rewardCost !== null"> pour {{ formatNumber(toBuy?.giveaway.rewardCost ?? 0) }} points par compte</template>, avec : {{ buyLogins }}.
        </p>
        <p class="text-sm text-muted mt-2">Les points sont dépensés aussitôt. Si la récompense a changé de prix sur la chaîne, l'achat est refusé.</p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="toBuy = null" />
          <UButton label="Acheter" icon="i-lucide-coins" @click="confirmBuy" />
        </div>
      </template>
    </UModal>

    <UModal :open="toDelete !== null" title="Supprimer le giveaway ?" @update:open="v => { if (!v) toDelete = null }">
      <template #body>
        <p>Le giveaway de {{ toDelete?.channel }} sera retiré de l'historique. Cette action est définitive.</p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="toDelete = null" />
          <UButton label="Supprimer" color="error" :loading="busy('delete')" @click="confirmDelete" />
        </div>
      </template>
    </UModal>
  </div>
</template>
