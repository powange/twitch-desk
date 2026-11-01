<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../../composables/useSnapshot'
import { useOpenExternal } from '../../utils/external'
import { useWatchStream } from '../../utils/viewer'
import { channelUrl, timeAgo } from '../../utils/format'
import type { GiveawayView, ParticipationResult } from '../../types'
import ChannelFlag from '../../components/ChannelFlag.vue'
import GiveawayConditions from '../../components/GiveawayConditions.vue'
import GiveawayFormModal from '../../components/GiveawayFormModal.vue'
import ChatLogModal from '../../components/ChatLogModal.vue'
import { useGiftCatalog } from '../../composables/useGiftCatalog'
import { GIVEAWAY_KIND, formatDrawAt } from '../../utils/giveaways'

const { snapshot, accounts } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()
const watchStream = useWatchStream()

const view = computed(() => snapshot.value?.giveaways)
const giveaways = computed(() => view.value?.giveaways ?? [])

// --- Follow requis : nos comptes suivent-ils la chaîne ? (relu à l'ouverture de la page) ---
interface FollowState { accountId: number, login: string, follows: boolean | null, followedAt: string | null }
const follows = reactive<Record<string, FollowState[] | 'loading'>>({})
const following = ref<string | null>(null)
async function loadFollows(channel: string) {
  follows[channel] = 'loading'
  try {
    follows[channel] = await invoke<FollowState[]>('follow_status', { channel })
  } catch {
    delete follows[channel]
  }
}
// Giveaways ouverts avec follow requis : état lu une fois par chaîne
watch(() => giveaways.value.filter(g => g.open && g.requireFollow === true).map(g => g.channel), (channels) => {
  for (const c of new Set(channels)) if (!(c in follows)) void loadFollows(c)
}, { immediate: true })
function followList(channel: string): FollowState[] | null {
  const f = follows[channel]
  return Array.isArray(f) ? f : null
}
const notFollowing = (channel: string) => (followList(channel) ?? []).filter(f => f.follows === false)
// Jours depuis le follow d'un compte
const followDays = (f: FollowState) => f.followedAt ? Math.floor((Date.now() - new Date(f.followedAt).getTime()) / 86_400_000) : null
// Comptes qui suivent, mais depuis moins longtemps que demandé
function tooRecent(g: GiveawayView) {
  const min = g.followMinDays
  if (!min) return []
  return (followList(g.channel) ?? []).filter(f => f.follows && (followDays(f) ?? 0) < min)
}
const followOk = (g: GiveawayView) => {
  const list = followList(g.channel)
  return !!list && list.every(f => f.follows) && tooRecent(g).length === 0
}
function followTooltip(g: GiveawayView) {
  const list = followList(g.channel)
  const head = g.followMinDays ? `Follow requis depuis au moins ${g.followMinDays} jour(s)` : 'Follow requis'
  if (!list) return `${head} : vérification…`
  const missing = list.filter(f => f.follows === false).map(f => f.login)
  const unknown = list.filter(f => f.follows === null).map(f => f.login)
  const recent = tooRecent(g).map(f => `${f.login} (${followDays(f)} j)`)
  const parts = [head]
  if (missing.length) parts.push(`pas suivie par ${missing.join(', ')} : clique pour suivre`)
  if (recent.length) parts.push(`follow trop récent : ${recent.join(', ')}`)
  if (!missing.length && !recent.length && !unknown.length) parts.push('suivie par tous tes comptes')
  if (unknown.length) parts.push(`état inconnu pour ${unknown.join(', ')}`)
  return parts.join(' · ')
}
// Journal du chat d'une chaîne
const logChannel = ref<string | null>(null)
const logOpen = ref(false)
function openLog(login: string) {
  logChannel.value = login
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

// Discord requis : invitation de la fiche du streamer, sinon trouvée sur sa chaîne Twitch
const openingDiscord = ref<string | null>(null)
async function openDiscord(channel: string) {
  if (openingDiscord.value) return
  openingDiscord.value = channel
  try {
    const url = await invoke<string | null>('streamer_discord', { login: channel })
    if (url) openExternal(url)
    else toast.add({ title: 'Aucune invitation Discord connue', description: `Ajoute-la sur la fiche de ${channel} (Modifier → Chaîne).`, color: 'warning' })
  } catch (e) {
    toast.add({ title: 'Invitation Discord introuvable', description: String(e), color: 'error' })
  } finally {
    openingDiscord.value = null
  }
}

async function followChannel(channel: string) {
  const ids = notFollowing(channel).map(f => f.accountId)
  if (!ids.length || following.value) return
  following.value = channel
  try {
    follows[channel] = await invoke<FollowState[]>('follow_channel', { channel, accountIds: ids })
    toast.add({ title: `${channel} suivie`, color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Follow impossible', description: String(e), color: 'error' })
    void loadFollows(channel)
  } finally {
    following.value = null
  }
}
const mentions = computed(() => view.value?.mentions ?? [])
// File d'alertes : giveaways ouverts pas encore traités, et mentions
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

async function participate(g: GiveawayView, ids = chosenFor(g)) {
  const todo = ids.filter(id => !entered(g, id))
  if (todo.length === 0) return
  participating.value = g.id
  try {
    const results = await invoke<ParticipationResult[]>('participate_giveaway', { giveawayId: g.id, accountIds: todo })
    const failed = results.filter(r => !r.ok)
    if (failed.length === 0) {
      toast.add({ title: `${g.command} envoyé sur ${g.channel}`, description: results.map(r => r.login).join(', '), color: 'success', icon: 'i-lucide-check' })
    } else {
      toast.add({ title: 'Participation incomplète', description: failed.map(r => `${r.login} : ${r.message}`).join(' · '), color: 'error' })
    }
  } catch (e) {
    toast.add({ title: 'Participation impossible', description: String(e), color: 'error' })
  } finally {
    participating.value = null
  }
}

function evidenceText(g: GiveawayView) {
  const e = g.evidence
  if (!e) return ''
  return `${e.count} messages sur les ${e.window} derniers, par ${e.users.length} pseudos différents : ${e.users.slice(0, 8).join(', ')}${e.users.length > 8 ? '…' : ''}`
}

const { byId: giftById } = useGiftCatalog()

// Formulaire : null = nouveau giveaway
const formOpen = ref(false)
const editing = ref<GiveawayView | null>(null)
function openForm(g: GiveawayView | null) {
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
      <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">
        File d'alertes <span v-if="alerts.length + mentions.length" class="text-primary">({{ alerts.length + mentions.length }})</span>
      </h2>
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
          <UBadge :label="GIVEAWAY_KIND[g.kind].label" :color="GIVEAWAY_KIND[g.kind].color" variant="subtle" size="sm" />
          <UTooltip v-if="g.command" text="Copier la commande">
            <button type="button" class="font-mono bg-elevated px-1.5 py-0.5 rounded hover:text-primary" @click="copyCommand(g.command)">{{ g.command }}</button>
          </UTooltip>
          <GiveawayConditions :details="g" :kind="g.kind" />
          <span v-if="g.drawAt" class="text-sm text-muted inline-flex items-center gap-1"><UIcon name="i-lucide-clock" class="size-4" />Tirage {{ formatDrawAt(g.drawAt) }}</span>
          <UTooltip v-if="g.evidence" :text="evidenceText(g)">
            <span class="text-sm text-muted underline decoration-dotted cursor-help">{{ share(g) }} % du chat</span>
          </UTooltip>
          <span class="text-xs text-muted ml-auto">{{ timeAgo(g.detectedAt, now) }}</span>
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
          <template v-else>
            <UButton v-if="g.url" label="Voir le giveaway" icon="i-lucide-external-link" size="sm" @click="openExternal(g.url)" />
            <UButton label="Acheter des tickets" icon="i-lucide-ticket" size="sm" color="neutral" variant="soft" to="/streamelements" />
          </template>
          <UButton label="Regarder" icon="i-lucide-tv" size="sm" color="neutral" variant="soft" title="Ajouter le stream à la fenêtre multi-stream" @click="watchStream(g.channel)" />
          <UButton label="Modifier" icon="i-lucide-pencil" size="sm" color="neutral" variant="ghost" @click="openForm(g)" />
          <UButton label="Ignorer" size="sm" color="neutral" variant="ghost" @click="invoke('dismiss_giveaway', { id: g.id })" />
        </div>
      </div>

      <div v-for="m in mentions" :key="`m${m.id}`" class="border border-warning/40 bg-warning/10 rounded-lg px-4 py-3 flex items-start gap-3">
        <UIcon name="i-lucide-at-sign" class="size-5 text-warning mt-0.5" />
        <div class="flex-1 min-w-0">
          <div class="font-medium">{{ m.author }} mentionne {{ m.account }} sur <button class="hover:underline" @click="openExternal(channelUrl(m.channel))">{{ m.channel }}</button></div>
          <div class="text-sm text-muted break-words">« {{ m.message }} »</div>
        </div>
        <span class="text-xs text-muted">{{ timeAgo(m.at, now) }}</span>
        <UButton label="Vu" size="xs" color="neutral" variant="soft" @click="invoke('dismiss_mention', { id: m.id })" />
      </div>
    </section>

    <!-- Giveaways récents -->
    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">Giveaways récents</h2>
      <div class="border border-default rounded-lg overflow-x-auto">
        <table class="w-full text-sm">
          <thead class="bg-elevated/60 text-xs uppercase text-muted">
            <tr>
              <th class="text-left px-3 py-2">Détecté</th>
              <th class="text-left px-3 py-2">Chaîne</th>
              <th class="text-left px-3 py-2">Type</th>
              <th class="text-left px-3 py-2">Commande</th>
              <th class="text-left px-3 py-2">Cadeaux</th>
              <th class="px-2 py-2">
                <div class="flex justify-center">
                  <UTooltip text="Présence requise dans le chat au moment du tirage">
                  <UIcon name="i-lucide-user-check" class="size-4 block" aria-label="Présence au tirage" />
                  </UTooltip>
                </div>
              </th>
              <th class="px-2 py-2">
                <div class="flex justify-center">
                  <UTooltip text="Être sur le Discord du streamer">
                  <UIcon name="i-simple-icons-discord" class="size-4 block" aria-label="Discord requis" />
                  </UTooltip>
                </div>
              </th>
              <th class="px-2 py-2">
                <div class="flex justify-center">
                  <UTooltip text="Follow de la chaîne requis (vert : suivie par tous tes comptes ; clique pour suivre sinon)">
                    <UIcon name="i-lucide-heart" class="size-4 block" aria-label="Follow requis" />
                  </UTooltip>
                </div>
              </th>
              <th class="px-2 py-2">
                <div class="flex justify-center">
                  <UTooltip text="Avoir dit bonjour dans le chat">
                    <UIcon name="i-lucide-hand" class="size-4 block" aria-label="Avoir dit bonjour" />
                  </UTooltip>
                </div>
              </th>
              <th class="px-2 py-2">
                <div class="flex justify-center">
                  <UTooltip text="Temps de présence minimum dans le chat">
                    <UIcon name="i-lucide-timer" class="size-4 block" aria-label="Temps de présence dans le chat" />
                  </UTooltip>
                </div>
              </th>
              <th class="text-left px-3 py-2">Tirage</th>
              <th class="text-left px-3 py-2">Participations</th>
              <th class="text-left px-3 py-2">Gagnant</th>
              <th class="text-left px-3 py-2">État</th>
              <th class="px-3 py-2" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="g in giveaways" :key="g.id" class="border-t border-default" :class="g.winnerOurs ? 'bg-success/10' : ''">
              <td class="px-3 py-2 whitespace-nowrap text-muted">{{ timeAgo(g.detectedAt, now) }}</td>
              <td class="px-3 py-2">
                <div class="flex items-center gap-1.5">
                  <ChannelFlag :login="g.channel" />
                  <button class="font-medium hover:underline" @click="openExternal(channelUrl(g.channel))">{{ g.channel }}</button>
                </div>
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
              </td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap items-center gap-1 min-w-24 max-w-64">
                  <UTooltip v-for="id in g.giftIds" :key="id" :text="giftById.get(id)?.title ?? ''">
                    <img v-if="giftById.get(id)?.image" :src="giftById.get(id)?.image ?? ''" referrerpolicy="no-referrer" alt="" class="size-8 rounded object-contain bg-elevated">
                    <span v-else class="size-8 rounded bg-elevated flex items-center justify-center"><UIcon name="i-lucide-gift" class="size-4 text-muted" /></span>
                  </UTooltip>
                  <span v-if="g.giftIds.length === 0" class="text-muted">—</span>
                </div>
              </td>
              <!-- Une colonne par condition : ? inconnu, picto si requis, rien sinon -->
              <td class="px-2 py-2">
                <div class="flex justify-center items-center">
                  <span v-if="g.requirePresence === null" class="text-muted" title="On ne sait pas">?</span>
                  <UTooltip v-else-if="g.requirePresence" text="Présence requise au moment du tirage">
                    <UIcon name="i-lucide-user-check" class="size-4 text-emerald-600 dark:text-emerald-400 block" />
                  </UTooltip>
                </div>
              </td>
              <td class="px-2 py-2">
                <div class="flex justify-center items-center">
                  <span v-if="g.requireDiscord === null" class="text-muted" title="On ne sait pas">?</span>
                  <UTooltip v-else-if="g.requireDiscord" text="Être sur le Discord du streamer : clique pour ouvrir son invitation">
                    <button
                      type="button"
                      class="inline-flex text-indigo-600 dark:text-indigo-400 hover:text-indigo-400 disabled:opacity-50"
                      :disabled="openingDiscord === g.channel"
                      :aria-label="`Ouvrir le Discord de ${g.channel}`"
                      @click="openDiscord(g.channel)"
                    >
                      <UIcon :name="openingDiscord === g.channel ? 'i-lucide-loader-circle' : 'i-simple-icons-discord'" class="size-4" :class="openingDiscord === g.channel ? 'animate-spin' : ''" />
                    </button>
                  </UTooltip>
                </div>
              </td>
              <td class="px-2 py-2">
                <div class="flex justify-center items-center">
                  <span v-if="g.requireFollow === null" class="text-muted" title="On ne sait pas">?</span>
                  <UTooltip v-else-if="g.requireFollow" :text="g.open ? followTooltip(g) : g.followMinDays ? `Follow requis depuis au moins ${g.followMinDays} jour(s)` : 'Follow requis'">
                    <button
                      v-if="g.open && notFollowing(g.channel).length"
                      type="button"
                      class="inline-flex text-warning hover:text-pink-500 disabled:opacity-50"
                      :disabled="following === g.channel"
                      :aria-label="`Suivre ${g.channel}`"
                      @click="followChannel(g.channel)"
                    >
                      <UIcon :name="following === g.channel ? 'i-lucide-loader-circle' : 'i-lucide-heart'" class="size-4" :class="following === g.channel ? 'animate-spin' : ''" />
                    </button>
                    <UIcon
                      v-else
                      name="i-lucide-heart"
                      class="size-4 block"
                      :class="g.open && followOk(g) ? 'text-success fill-current' : g.open && tooRecent(g).length ? 'text-warning' : 'text-pink-600 dark:text-pink-400'"
                    />
                  </UTooltip>
                  <span v-if="g.requireFollow && g.followMinDays" class="text-xs text-muted ml-1 whitespace-nowrap">{{ g.followMinDays }} j</span>
                </div>
              </td>
              <td class="px-2 py-2">
                <div class="flex justify-center items-center">
                  <span v-if="g.requireGreeting === null" class="text-muted" title="On ne sait pas">?</span>
                  <UTooltip v-else-if="g.requireGreeting" text="Avoir dit bonjour dans le chat">
                    <UIcon name="i-lucide-hand" class="size-4 text-amber-600 dark:text-amber-400 block" />
                  </UTooltip>
                </div>
              </td>
              <td class="px-2 py-2">
                <div class="flex justify-center items-center">
                  <span v-if="g.minChatMinutes === null" class="text-muted" title="On ne sait pas">?</span>
                  <UTooltip v-else-if="g.minChatMinutes > 0" :text="`Présent dans le chat depuis au moins ${g.minChatMinutes} min`">
                    <span class="inline-flex items-center gap-1 text-sky-600 dark:text-sky-400 whitespace-nowrap">
                      <UIcon name="i-lucide-timer" class="size-4" />{{ g.minChatMinutes }} min
                    </span>
                  </UTooltip>
                </div>
              </td>
              <td class="px-3 py-2 whitespace-nowrap" :class="g.drawAt ? '' : 'text-muted'">{{ formatDrawAt(g.drawAt) }}</td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap gap-1">
                  <UBadge
                    v-for="e in g.entriesView"
                    :key="e.accountId"
                    :label="e.tickets !== null ? `${e.login} · ${e.tickets} ticket${e.tickets > 1 ? 's' : ''}` : e.login"
                    :icon="e.tickets !== null ? 'i-lucide-ticket' : e.auto ? 'i-lucide-bot' : 'i-lucide-check'"
                    color="success"
                    variant="subtle"
                    size="sm"
                    class="whitespace-nowrap"
                    :title="e.tickets !== null ? 'Tickets StreamElements relevés sur StreamElements' : e.auto ? 'Participation automatique' : 'Participation envoyée par l\'application'"
                  />
                  <span v-if="g.entriesView.length === 0" class="text-muted">—</span>
                </div>
              </td>
              <td class="px-3 py-2">
                <template v-if="g.winner">
                  <span :class="g.winnerOurs ? 'text-success font-semibold' : ''">{{ g.winner }}</span>
                  <span v-if="g.winnerChance" class="text-xs text-muted"> ({{ g.winnerChance }})</span>
                  <UBadge v-if="g.winnerOurs" label="Gagné !" color="success" size="sm" class="ml-1" />
                </template>
                <span v-else class="text-muted">—</span>
              </td>
              <td class="px-3 py-2">
                <UBadge :label="g.open ? 'Ouvert' : 'Terminé'" :color="g.open ? 'primary' : 'neutral'" variant="subtle" />
              </td>
              <td class="px-3 py-2">
                <div class="flex justify-end gap-1">
                  <UButton
                    v-if="g.open && (g.kind === 'command' || g.kind === 'ticket') && g.command && accounts.some(a => !entered(g, a.id))"
                    label="Participer"
                    size="xs"
                    :loading="participating === g.id"
                    @click="participate(g)"
                  />
                  <UButton icon="i-lucide-scroll-text" size="xs" color="neutral" variant="ghost" :aria-label="`Journal du chat de ${g.channel}`" title="Journal du chat" @click="openLog(g.channel)" />
                  <UButton v-if="g.open" icon="i-lucide-tv" size="xs" color="neutral" variant="ghost" :aria-label="`Regarder ${g.channel}`" title="Regarder (fenêtre multi-stream)" @click="watchStream(g.channel)" />
                  <UButton icon="i-lucide-pencil" size="xs" color="neutral" variant="ghost" aria-label="Modifier" @click="openForm(g)" />
                  <UButton v-if="g.open" label="Clore" size="xs" color="neutral" variant="ghost" @click="invoke('close_giveaway', { id: g.id })" />
                  <UButton icon="i-lucide-trash-2" size="xs" color="error" variant="ghost" aria-label="Supprimer" @click="invoke('delete_giveaway', { id: g.id })" />
                </div>
              </td>
            </tr>
            <tr v-if="giveaways.length === 0">
              <td colspan="15" class="px-3 py-8 text-center text-muted">Aucun giveaway pour l'instant. Ils apparaissent à la détection, ou avec « Nouveau giveaway ».</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <GiveawayFormModal v-model:open="formOpen" :giveaway="editing" />
    <ChatLogModal v-model:open="logOpen" :channel="logChannel" />
  </div>
</template>
