<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../composables/useSnapshot'
import { suggestGifts, useGiftCatalog } from '../composables/useGiftCatalog'
import GiftFormModal from './GiftFormModal.vue'
import ChannelProfileCard, { type ChannelProfile } from './ChannelProfileCard.vue'
import { useOpenExternal } from '../utils/external'
import { boxArt, channelUrl, formatNumber } from '../utils/format'
import { GIVEAWAY_KIND_ITEMS } from '../utils/giveaways'
import type { GiveawayInput, GiveawayView } from '../types'

// Création (giveaway = null) ou modification d'une fiche de giveaway
const props = defineProps<{ giveaway: GiveawayView | null }>()
const open = defineModel<boolean>('open', { required: true })

const { snapshot } = useSnapshot()
const toast = useToast()
const { gifts } = useGiftCatalog()
const openExternal = useOpenExternal()

type TriState = 'yes' | 'no' | 'unknown'
const TRI_OPTIONS: { label: string, value: TriState, title: string }[] = [
  { label: 'Oui', value: 'yes', title: 'Requis' },
  { label: 'Non', value: 'no', title: 'Non requis' },
  { label: '?', value: 'unknown', title: 'On ne sait pas' }
]
// Une condition par ligne
const CONDITIONS = [
  { key: 'follow', label: 'Follow requis', icon: 'i-lucide-heart' },
  { key: 'presence', label: 'Présence au tirage', icon: 'i-lucide-user-check' },
  { key: 'discord', label: 'Discord du streamer', icon: 'i-simple-icons-discord' }
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
  notes: '',
  giftIds: [] as number[]
})

// Ce que le cœur sait de la chaîne saisie (fiche Twitch, StreamElements, streamer)
const suggestion = ref<ChannelSuggestion | null>(null)

// --- Cadeaux ---
const giftSearch = ref('')
// Cadeaux dont un mot-clé apparaît dans le titre du live, la description de la chaîne
// ou son message épinglé (comme sur le site Giveaway Twitch)
const suggestionText = computed(() => {
  const login = form.value.channel.trim().toLowerCase()
  const p = suggestion.value?.profile
  return [
    snapshot.value?.giveaways.channels.find(c => c.login === login)?.live?.title,
    p?.title,
    p?.description,
    p?.pinnedMessage,
    // Giveaway StreamElements actif : son titre et sa description annoncent souvent les lots
    suggestion.value?.streamelements?.title,
    suggestion.value?.streamelements?.description
  ].filter(Boolean).join('\n')
})
const suggested = computed(() => new Set(suggestGifts(gifts.value, suggestionText.value).map(g => g.id)))
// Jeu streamé en ce moment par la chaîne : ses cadeaux passent en tête
const liveGameId = computed(() => {
  const login = form.value.channel.trim().toLowerCase()
  return snapshot.value?.giveaways.channels.find(c => c.login === login)?.live?.gameId ?? null
})
const giftList = computed(() => {
  const q = giftSearch.value.trim().toLowerCase()
  const rank = (id: number, gameId: string | null) =>
    (form.value.giftIds.includes(id) ? 4 : 0) + (suggested.value.has(id) ? 2 : 0) + (liveGameId.value && gameId === liveGameId.value ? 1 : 0)
  return gifts.value
    .filter(g => !q || g.title.toLowerCase().includes(q) || g.keywords.some(k => k.includes(q)) || (g.gameName ?? '').toLowerCase().includes(q))
    .sort((a, b) => rank(b.id, b.gameId) - rank(a.id, a.gameId) || a.title.localeCompare(b.title))
})
function toggleGift(id: number) {
  const ids = form.value.giftIds
  form.value.giftIds = ids.includes(id) ? ids.filter(x => x !== id) : [...ids, id]
}
// À la création, les cadeaux suggérés sont cochés tant qu'on n'a rien choisi
watch(suggested, (ids) => {
  if (!props.giveaway && open.value && form.value.giftIds.length === 0 && ids.size) form.value.giftIds = [...ids]
})
const giftFormOpen = ref(false)
function onGiftCreated(id: number) {
  if (!form.value.giftIds.includes(id)) form.value.giftIds = [...form.value.giftIds, id]
}

watch(open, (isOpen) => {
  if (!isOpen) return
  suggestion.value = null
  streamerDiscord.value = ''
  streamerNotes.value = ''
  streamerLoaded.value = { discord: '', notes: '' }
  // Modification : fiche de la chaîne chargée tout de suite
  if (props.giveaway) void inspect(props.giveaway.channel)
  const g = props.giveaway
  form.value = {
    channel: g?.channel ?? '',
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
    notes: g?.notes ?? '',
    giftIds: [...(g?.giftIds ?? [])]
  }
  giftSearch.value = ''
})

const channelItems = computed(() => (snapshot.value?.giveaways.channels ?? []).map(c => c.login))

// --- Pré-remplissage à la création : giveaway StreamElements actif, commande détectée dans le chat ---
interface ChannelSuggestion {
  streamelements: { url: string, title: string, description: string, command: string | null, cost: number, maxTickets: number | null } | null
  detectedCommand: string | null
  profile: ChannelProfile | null
  streamer: { login: string, discordUrl: string | null, notes: string | null } | null
}

// --- Informations propres au streamer (mémorisées, retrouvées à chaque giveaway) ---
const streamerDiscord = ref('')
const streamerNotes = ref('')
// Valeurs chargées, pour n'enregistrer que si elles changent
const streamerLoaded = ref({ discord: '', notes: '' })
const inspecting = ref(false)
let inspectTimer: ReturnType<typeof setTimeout> | null = null
let inspectSeq = 0

watch(() => form.value.channel, (login) => {
  if (!open.value) return
  if (inspectTimer) clearTimeout(inspectTimer)
  suggestion.value = null
  if (!login.trim()) return
  inspectTimer = setTimeout(() => void inspect(login), 400)
})

async function inspect(login: string) {
  const seq = ++inspectSeq
  inspecting.value = true
  try {
    const found = await invoke<ChannelSuggestion>('inspect_giveaway_channel', { login })
    // Une saisie plus récente a pris le relais
    if (seq !== inspectSeq) return
    suggestion.value = found
    // Fiche du streamer : ce qu'on a noté, sinon le lien Discord trouvé sur sa chaîne
    streamerLoaded.value = { discord: found.streamer?.discordUrl ?? '', notes: found.streamer?.notes ?? '' }
    streamerDiscord.value = found.streamer?.discordUrl ?? found.profile?.discord ?? ''
    streamerNotes.value = found.streamer?.notes ?? ''
    // En modification, la fiche de la chaîne suffit : on ne remplace pas ce qui a été saisi
    if (props.giveaway) return
    const f = form.value
    if (found.streamelements) {
      f.kind = 'streamelements'
      f.url = found.streamelements.url
      if (found.streamelements.command) f.command = found.streamelements.command
      if (found.streamelements.maxTickets) f.ticketMax = found.streamelements.maxTickets
    } else if (found.detectedCommand && !f.command) {
      f.command = found.detectedCommand
    }
  } catch {
    // Meilleur effort : rien à suggérer
  } finally {
    if (seq === inspectSeq) inspecting.value = false
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
  const conditions = [f.follow, f.presence, f.discord].filter(v => v !== 'unknown').length
  return [
    { key: 'channel' as const, label: 'Chaîne', icon: 'i-lucide-tv', summary: f.channel.trim() || 'à choisir', missing: !f.channel.trim() },
    { key: 'giveaway' as const, label: 'Giveaway', icon: 'i-lucide-gift', summary: GIVEAWAY_KIND_ITEMS.find(k => k.value === f.kind)?.label ?? '', missing: (f.kind === 'command' || f.kind === 'ticket') && !f.command.trim() },
    { key: 'gifts' as const, label: 'Cadeaux', icon: 'i-lucide-package', summary: f.giftIds.length ? `${f.giftIds.length} choisi(s)` : 'aucun', missing: false },
    { key: 'conditions' as const, label: 'Conditions', icon: 'i-lucide-list-checks', summary: conditions ? `${conditions} renseignée(s)` : 'inconnues', missing: false }
  ]
})

// Erreur du cœur : ouvrir la section concernée
function sectionForError(message: string): Section | null {
  const m = message.toLowerCase()
  if (m.includes('chaîne') || m.includes('discord')) return 'channel'
  if (['commande', 'ticket', 'lien', 'coût', 'tirage', 'type'].some(w => m.includes(w))) return 'giveaway'
  return null
}

async function save() {
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
    notes: f.notes || null,
    giftIds: f.giftIds
  }
  saving.value = true
  try {
    if (props.giveaway) await invoke('update_giveaway', { id: props.giveaway.id, input })
    else await invoke('create_giveaway', { input })
    // Fiche du streamer : enregistrée si elle a changé (ou si le Discord vient d'être trouvé)
    if (streamerDiscord.value !== streamerLoaded.value.discord || streamerNotes.value !== streamerLoaded.value.notes) {
      await invoke('save_streamer', { streamer: { login: f.channel, discordUrl: streamerDiscord.value || null, notes: streamerNotes.value || null } })
    }
    toast.add({ title: props.giveaway ? 'Giveaway modifié' : 'Giveaway ajouté', color: 'success', icon: 'i-lucide-check' })
    open.value = false
  } catch (e) {
    toast.add({ title: 'Giveaway non enregistré', description: String(e), color: 'error' })
    const target = sectionForError(String(e))
    if (target) section.value = target
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <UModal v-model:open="open" :title="giveaway ? `Giveaway de ${giveaway.channel}` : 'Nouveau giveaway'" :ui="{ content: 'max-w-4xl' }">
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
                list="gw-channel-list"
                placeholder="lunarsea"
                autocomplete="off"
                class="w-full"
                :trailing-icon="inspecting ? 'i-lucide-loader-circle' : undefined"
                :ui="{ trailingIcon: 'animate-spin' }"
              />
              <datalist id="gw-channel-list">
                <option v-for="c in channelItems" :key="c" :value="c" />
              </datalist>
            </UFormField>
            <!-- Fiche de la chaîne -->
            <ChannelProfileCard v-if="profile" :profile="profile" />
            <p v-else-if="suggestion && !inspecting && form.channel.trim()" class="text-sm text-error">Chaîne introuvable sur Twitch.</p>

            <UAlert
              v-if="suggestion?.streamelements && !giveaway"
              icon="i-lucide-sparkles"
              color="success"
              variant="subtle"
              :title="`Giveaway StreamElements actif : ${suggestion.streamelements.title || 'sans titre'}`"
              :description="`${suggestion.streamelements.cost} points le ticket${suggestion.streamelements.maxTickets ? `, ${suggestion.streamelements.maxTickets} au plus par personne` : ''}. Type, lien et commande remplis.`"
            />
            <UAlert
              v-else-if="suggestion?.detectedCommand && !giveaway"
              icon="i-lucide-sparkles"
              color="info"
              variant="subtle"
              :title="`Commande détectée dans le chat : ${suggestion.detectedCommand}`"
            />
            <div class="flex flex-col gap-3 border-t border-default pt-4">
              <div class="text-xs uppercase tracking-wide text-muted">Sur ce streamer (gardé pour ses prochains giveaways)</div>
              <UFormField label="Discord du streamer" :help="profile?.discord && streamerDiscord === profile.discord && !streamerLoaded.discord ? 'Trouvé sur sa chaîne Twitch' : undefined">
                <div class="flex gap-2">
                  <UInput id="gw-streamer-discord" v-model="streamerDiscord" icon="i-simple-icons-discord" placeholder="https://discord.gg/…" class="flex-1" />
                  <UButton v-if="streamerDiscord.startsWith('https://')" icon="i-lucide-external-link" color="neutral" variant="soft" aria-label="Ouvrir le Discord" @click="openExternal(streamerDiscord)" />
                </div>
              </UFormField>
              <UFormField label="Notes sur le streamer">
                <UTextarea id="gw-streamer-notes" v-model="streamerNotes" :rows="3" autoresize placeholder="Habitudes de tirage, règles, horaires…" class="w-full" />
              </UFormField>
            </div>
          </div>

          <!-- Giveaway : type, puis les champs de ce type -->
          <div v-show="section === 'giveaway'" class="flex flex-col gap-4">
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

            <UFormField v-if="form.kind === 'command' || form.kind === 'ticket' || form.kind === 'streamelements'" label="Commande à taper">
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

            <div v-if="form.kind === 'streamelements'" class="grid grid-cols-2 gap-4">
              <UFormField label="Lien du giveaway StreamElements">
                <UInput id="gw-url" v-model="form.url" placeholder="https://streamelements.com/…/giveaway/…" class="w-full" />
              </UFormField>
              <UFormField label="Tickets maximum par personne">
                <UInputNumber id="gw-se-max" v-model="form.ticketMax" :min="1" :max="100000" placeholder="Inconnu" class="w-full" />
              </UFormField>
            </div>

            <div v-if="form.kind === 'channelpoints'" class="grid grid-cols-2 gap-4">
              <UFormField label="Récompense à acheter">
                <UInput id="gw-reward" v-model="form.rewardName" placeholder="Ticket giveaway" class="w-full" />
              </UFormField>
              <UFormField label="Coût en points de chaîne">
                <UInputNumber id="gw-reward-cost" v-model="form.rewardCost" :min="0" placeholder="Inconnu" class="w-full" />
              </UFormField>
            </div>

            <UFormField label="Tirage" help="Un rappel est envoyé 5 min avant si la présence est requise" class="max-w-xs">
              <UInput id="gw-draw" v-model="form.drawAt" type="datetime-local" class="w-full" />
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
            <div v-if="gifts.length" class="grid grid-cols-[repeat(auto-fill,minmax(170px,1fr))] gap-2 max-h-[380px] overflow-y-auto p-0.5">
              <button
                v-for="g in giftList"
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
                <UIcon v-if="suggested.has(g.id)" name="i-lucide-sparkles" class="absolute top-1 right-1 size-3.5 text-primary" title="Suggéré : un mot-clé apparaît dans le titre, la description, le message épinglé ou le giveaway StreamElements de la chaîne" />
              </button>
            </div>
            <p v-else class="text-sm text-muted">Catalogue vide : ajoute un cadeau avec « Nouveau cadeau ».</p>
          </div>

          <!-- Conditions : une par ligne -->
          <div v-show="section === 'conditions'" class="flex flex-col gap-3">
            <p class="text-sm text-muted">Ce qu'il faut pour participer. « ? » : on ne sait pas.</p>
            <div v-for="c in CONDITIONS" :key="c.key" class="flex items-center justify-between gap-4 border border-default rounded-lg px-4 py-3">
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
            </div>
          </div>
        </div>
      </form>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton label="Annuler" color="neutral" variant="ghost" @click="open = false" />
        <UButton type="submit" form="giveaway-form" :label="giveaway ? 'Enregistrer' : 'Ajouter'" :loading="saving" />
      </div>
    </template>
  </UModal>
  <GiftFormModal v-model:open="giftFormOpen" :gift="null" @saved="onGiftCreated" />
</template>
