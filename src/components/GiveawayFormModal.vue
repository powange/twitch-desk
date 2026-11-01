<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../composables/useSnapshot'
import { GIVEAWAY_KIND_ITEMS } from '../utils/giveaways'
import type { GiveawayInput, GiveawayView } from '../types'

// Création (giveaway = null) ou modification d'une fiche de giveaway
const props = defineProps<{ giveaway: GiveawayView | null }>()
const open = defineModel<boolean>('open', { required: true })

const { snapshot } = useSnapshot()
const toast = useToast()

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
  notes: ''
})

watch(open, (isOpen) => {
  if (!isOpen) return
  suggestion.value = null
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
    notes: g?.notes ?? ''
  }
})

const channelItems = computed(() => (snapshot.value?.giveaways.channels ?? []).map(c => c.login))

// --- Pré-remplissage à la création : giveaway StreamElements actif, commande détectée dans le chat ---
interface ChannelSuggestion {
  streamelements: { url: string, title: string, command: string | null, cost: number, maxTickets: number | null } | null
  detectedCommand: string | null
}
const suggestion = ref<ChannelSuggestion | null>(null)
const inspecting = ref(false)
let inspectTimer: ReturnType<typeof setTimeout> | null = null
let inspectSeq = 0

watch(() => form.value.channel, (login) => {
  if (props.giveaway || !open.value) return
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
    notes: f.notes || null
  }
  saving.value = true
  try {
    if (props.giveaway) await invoke('update_giveaway', { id: props.giveaway.id, input })
    else await invoke('create_giveaway', { input })
    toast.add({ title: props.giveaway ? 'Giveaway modifié' : 'Giveaway ajouté', color: 'success', icon: 'i-lucide-check' })
    open.value = false
  } catch (e) {
    toast.add({ title: 'Giveaway non enregistré', description: String(e), color: 'error' })
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <UModal v-model:open="open" :title="giveaway ? `Giveaway de ${giveaway.channel}` : 'Nouveau giveaway'" :ui="{ content: 'max-w-2xl' }">
    <template #body>
      <form id="giveaway-form" class="grid grid-cols-2 gap-4" @submit.prevent="save">
        <UFormField label="Chaîne" class="col-span-1">
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
        <UFormField label="Type">
          <USelect id="gw-kind" v-model="form.kind" :items="GIVEAWAY_KIND_ITEMS" class="w-full" />
        </UFormField>

        <UAlert
          v-if="suggestion?.streamelements"
          class="col-span-2"
          icon="i-lucide-sparkles"
          color="success"
          variant="subtle"
          :title="`Giveaway StreamElements actif : ${suggestion.streamelements.title || 'sans titre'}`"
          :description="`${suggestion.streamelements.cost} points le ticket${suggestion.streamelements.maxTickets ? `, ${suggestion.streamelements.maxTickets} au plus par personne` : ''}. Type, lien et commande remplis.`"
        />
        <UAlert
          v-else-if="suggestion?.detectedCommand"
          class="col-span-2"
          icon="i-lucide-sparkles"
          color="info"
          variant="subtle"
          :title="`Commande détectée dans le chat : ${suggestion.detectedCommand}`"
        />

        <UFormField v-if="form.kind === 'command' || form.kind === 'ticket' || form.kind === 'streamelements'" label="Commande à taper" class="col-span-2">
          <UInput id="gw-command" v-model="form.command" placeholder="!coffre" class="w-full font-mono" />
        </UFormField>

        <template v-if="form.kind === 'ticket'">
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
        </template>

        <template v-if="form.kind === 'streamelements'">
          <UFormField label="Lien du giveaway StreamElements">
            <UInput id="gw-url" v-model="form.url" placeholder="https://streamelements.com/…/giveaway/…" class="w-full" />
          </UFormField>
          <UFormField label="Tickets maximum par personne">
            <UInputNumber id="gw-se-max" v-model="form.ticketMax" :min="1" :max="100000" placeholder="Inconnu" class="w-full" />
          </UFormField>
        </template>

        <template v-if="form.kind === 'channelpoints'">
          <UFormField label="Récompense à acheter">
            <UInput id="gw-reward" v-model="form.rewardName" placeholder="Ticket giveaway" class="w-full" />
          </UFormField>
          <UFormField label="Coût en points de chaîne">
            <UInputNumber id="gw-reward-cost" v-model="form.rewardCost" :min="0" placeholder="Inconnu" class="w-full" />
          </UFormField>
        </template>

        <UFormField label="Tirage" help="Un rappel est envoyé 5 min avant si la présence est requise">
          <UInput id="gw-draw" v-model="form.drawAt" type="datetime-local" class="w-full" />
        </UFormField>
        <div />

        <div class="col-span-2 flex flex-col gap-2">
          <div class="text-sm font-medium">Conditions</div>
          <div v-for="c in CONDITIONS" :key="c.key" class="flex items-center justify-between gap-4">
            <span class="inline-flex items-center gap-2 text-sm">
              <UIcon :name="c.icon" class="size-4 text-muted" />{{ c.label }}
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

        <UFormField label="Notes" class="col-span-2">
          <UTextarea id="gw-notes" v-model="form.notes" :rows="2" autoresize placeholder="Lot, règles particulières…" class="w-full" />
        </UFormField>
      </form>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton label="Annuler" color="neutral" variant="ghost" @click="open = false" />
        <UButton type="submit" form="giveaway-form" :label="giveaway ? 'Enregistrer' : 'Ajouter'" :loading="saving" />
      </div>
    </template>
  </UModal>
</template>
