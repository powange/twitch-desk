<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ChannelLive, Giveaway, GiveawayConditions, InstanceState, PhoneAction } from '../types'
import { ApiError, giveawayChatLog, giveawayConditions, hideGiveaway } from '../api'
import { pendingFor, usePhone } from '../phone'
import { chatUrl, conditions, copyText, GIVEAWAY_KIND, playerUrl, streamUrl } from '../giveaways'
import { formatDraw, formatNumber, formatWhen } from '../format'
import GiftImage from './GiftImage.vue'

const props = defineProps<{ giveaway: Giveaway, state: InstanceState | null }>()
const toast = useToast()

const g = computed(() => props.giveaway)
const kind = computed(() => GIVEAWAY_KIND[g.value.kind] ?? { label: g.value.kind, color: 'neutral' as const })
const live = computed<ChannelLive | null>(() => props.state?.live[g.value.channel] ?? null)
const mine = computed(() => props.state?.giveaways[g.value.id] ?? null)
const conds = computed(() => conditions(g.value))
const watching = ref(false)

// Demandes au PC : masquer, journal du chat, conditions (exécutées par le PC, résultat annoncé)
const phone = usePhone()
const hidden = computed(() => !!mine.value?.hidden)
const busy = (kind: string) => pendingFor(phone.actions.value, kind, g.value.id)
async function ask(run: (token: string) => Promise<PhoneAction>) {
  const token = phone.token.value
  if (!token) return
  try {
    phone.requested(await run(token))
  } catch (e) {
    toast.add({ title: 'Demande non envoyée', description: e instanceof ApiError && e.status === 400 ? 'Mets à jour le serveur de synchronisation (version 0.8.10 ou plus).' : String(e), color: 'error' })
  }
}
const toggleHidden = () => ask(t => hideGiveaway<PhoneAction>(t, g.value.id, !hidden.value))
const openChatLog = () => ask(t => giveawayChatLog<PhoneAction>(t, g.value.id))

// Conditions : Oui / Non / ? ; temps de présence en minutes (0 : non requis)
type Tri = 'yes' | 'no' | 'unknown'
const toTri = (v: boolean | null | undefined): Tri => v === true ? 'yes' : v === false ? 'no' : 'unknown'
const fromTri = (v: Tri) => v === 'yes' ? true : v === 'no' ? false : null
const COND_ROWS = [
  { key: 'requirePresence', label: 'Présence au tirage', icon: 'i-lucide-user-check' },
  { key: 'requireDiscord', label: 'Discord du streamer', icon: 'i-simple-icons-discord' },
  { key: 'requireFollow', label: 'Follow requis', icon: 'i-lucide-heart' },
  { key: 'requireGreeting', label: 'Avoir dit bonjour', icon: 'i-lucide-hand' },
  { key: 'canGift', label: 'Gain offrable', icon: 'i-lucide-hand-heart' }
] as const
const condOpen = ref(false)
const form = ref<Record<string, Tri>>({})
const chatTime = ref<Tri>('unknown')
const chatMinutes = ref<number | null>(null)
function openConditions() {
  const x = g.value
  form.value = Object.fromEntries(COND_ROWS.map(r => [r.key, toTri(x[r.key])]))
  chatTime.value = x.minChatMinutes == null ? 'unknown' : x.minChatMinutes > 0 ? 'yes' : 'no'
  chatMinutes.value = x.minChatMinutes && x.minChatMinutes > 0 ? x.minChatMinutes : null
  condOpen.value = true
}
async function saveConditions() {
  const conditions: Partial<GiveawayConditions> = Object.fromEntries(COND_ROWS.map(r => [r.key, fromTri(form.value[r.key] ?? 'unknown')]))
  conditions.minChatMinutes = chatTime.value === 'yes' ? (chatMinutes.value ?? null) : chatTime.value === 'no' ? 0 : null
  await ask(t => giveawayConditions<PhoneAction>(t, g.value.id, conditions))
  condOpen.value = false
}

async function copy() {
  if (!g.value.command) return
  const ok = await copyText(g.value.command)
  toast.add(ok
    ? { title: 'Commande copiée', description: g.value.command, color: 'success', icon: 'i-lucide-check', duration: 2000 }
    : { title: 'Copie impossible', description: 'Sélectionne la commande à la main.', color: 'error' })
}
</script>

<template>
  <article class="rounded-xl border border-default bg-elevated/40 overflow-hidden" :class="g.status === 'paused' || hidden ? 'opacity-80' : ''">
    <div class="flex flex-col gap-3 p-4">
      <!-- Chaîne, type et statut -->
      <header class="flex flex-col gap-1.5">
        <div class="flex items-center gap-2 flex-wrap">
          <h2 class="text-lg font-semibold break-all">{{ g.channel }}</h2>
          <UBadge :label="kind.label" :color="kind.color" variant="subtle" size="sm" />
          <UBadge v-if="g.status === 'paused'" label="En pause" color="warning" variant="subtle" size="sm" icon="i-lucide-pause" />
          <UBadge v-if="hidden" label="Masqué" color="neutral" variant="subtle" size="sm" icon="i-lucide-eye-off" />
        </div>
        <div v-if="live" class="flex items-center gap-2 flex-wrap text-sm">
          <UBadge v-if="live.live" label="En live" color="error" variant="solid" size="sm" icon="i-lucide-radio" />
          <UBadge v-else label="Hors ligne" color="neutral" variant="subtle" size="sm" icon="i-lucide-moon" />
          <span v-if="live.live && live.viewers !== undefined" class="inline-flex items-center gap-1 text-muted whitespace-nowrap" :aria-label="`${formatNumber(live.viewers)} spectateurs`">
            <UIcon name="i-lucide-users" class="size-3.5" />{{ formatNumber(live.viewers) }}
          </span>
          <span v-if="live.live && live.category" class="text-muted truncate">{{ live.category }}</span>
        </div>
        <p v-if="live?.live && live.title" class="text-xs text-muted line-clamp-2">{{ live.title }}</p>
      </header>

      <!-- Commande à copier -->
      <div v-if="g.command" class="flex items-stretch gap-2">
        <code class="flex-1 min-w-0 rounded-lg bg-muted px-3 py-2.5 font-mono text-lg break-all select-all">{{ g.command }}</code>
        <UButton label="Copier" icon="i-lucide-copy" size="xl" @click="copy" />
      </div>
      <div v-if="g.kind === 'channelpoints' && g.rewardName" class="text-sm">
        <span class="text-muted">Récompense :</span> {{ g.rewardName }}<template v-if="g.rewardCost"> · {{ formatNumber(g.rewardCost) }} points</template>
      </div>
      <UButton
        v-if="g.kind === 'streamelements' && g.url"
        :to="g.url"
        target="_blank"
        label="Ouvrir le giveaway StreamElements"
        icon="i-lucide-external-link"
        color="warning"
        variant="soft"
        block
      />

      <!-- Participation de mes comptes -->
      <div v-if="state" class="flex items-center gap-2 text-sm">
        <template v-if="mine && mine.participated.length">
          <UIcon name="i-lucide-circle-check" class="size-4 text-success shrink-0" />
          <span><span class="text-muted">Participé :</span> {{ mine.participated.join(', ') }}</span>
        </template>
        <template v-else>
          <UIcon name="i-lucide-circle-dashed" class="size-4 text-warning shrink-0" />
          <span class="text-warning">Pas encore</span>
        </template>
      </div>

      <!-- Lots -->
      <ul v-if="g.gifts.length" class="flex flex-col gap-2">
        <li v-for="gift in g.gifts" :key="gift.id" class="flex items-center gap-3">
          <GiftImage :src="gift.image ?? null" />
          <div class="min-w-0">
            <div class="text-sm font-medium truncate">{{ gift.title ?? 'Cadeau' }}</div>
            <div v-if="gift.gameName" class="text-xs text-muted truncate">{{ gift.gameName }}</div>
            <div v-if="gift.group" class="text-xs text-muted truncate">{{ gift.group }}</div>
          </div>
        </li>
      </ul>

      <!-- Conditions -->
      <ul v-if="conds.length" class="flex flex-wrap gap-1.5" aria-label="Conditions">
        <li v-for="c in conds" :key="c.key" class="inline-flex items-center gap-1 rounded-full border border-default px-2 py-0.5 text-xs" :class="c.colorClass">
          <UIcon :name="c.icon" class="size-3.5" />
          <span :class="c.required ? '' : 'line-through decoration-1'">{{ c.label }}</span>
        </li>
      </ul>

      <!-- Tirage et gagnant -->
      <div v-if="g.drawAt" class="flex items-center gap-2 text-sm">
        <UIcon name="i-lucide-alarm-clock" class="size-4 text-muted shrink-0" />
        <span><span class="text-muted">Tirage :</span> {{ formatDraw(g.drawAt) }}</span>
      </div>
      <div v-if="g.winner" class="flex items-center gap-2 text-sm">
        <UIcon name="i-lucide-trophy" class="size-4 text-amber-500 shrink-0" />
        <span>
          <span class="text-muted">Gagnant :</span> <span class="font-medium">{{ g.winner }}</span>
          <template v-if="g.winnerChance"> · {{ g.winnerChance }}</template>
          <template v-if="g.drawnAt"> · {{ formatWhen(g.drawnAt) }}</template>
        </span>
      </div>
      <p v-if="g.notes" class="text-sm text-muted whitespace-pre-line">{{ g.notes }}</p>

      <!-- Demandes au PC -->
      <div class="grid grid-cols-3 gap-2">
        <UButton
          :label="hidden ? 'Afficher' : 'Masquer'"
          :icon="hidden ? 'i-lucide-eye' : 'i-lucide-eye-off'"
          color="neutral"
          variant="soft"
          size="sm"
          block
          :loading="busy('giveaway_hide')"
          @click="toggleHidden"
        />
        <UButton label="Journal" icon="i-lucide-scroll-text" color="neutral" variant="soft" size="sm" block :loading="busy('giveaway_chat_log')" @click="openChatLog" />
        <UButton label="Conditions" icon="i-lucide-list-checks" color="neutral" variant="soft" size="sm" block :loading="busy('giveaway_conditions')" @click="openConditions" />
      </div>

      <!-- Twitch -->
      <div class="grid grid-cols-2 gap-2">
        <UButton :to="streamUrl(g.channel)" target="_blank" label="Ouvrir le stream" icon="i-lucide-tv" color="neutral" variant="soft" block class="col-span-2" />
        <UButton :to="chatUrl(g.channel)" target="_blank" label="Chat" icon="i-lucide-message-square" color="neutral" variant="soft" block />
        <UButton
          :label="watching ? 'Fermer' : 'Regarder ici'"
          :icon="watching ? 'i-lucide-chevron-up' : 'i-lucide-play'"
          color="neutral"
          variant="soft"
          block
          :aria-expanded="watching"
          @click="watching = !watching"
        />
      </div>
    </div>
    <UModal v-model:open="condOpen" :title="`Conditions · ${g.channel}`" description="Enregistrées par le PC ; un giveaway en cours met aussi à jour la fiche du streamer. « ? » : on ne sait pas.">
      <template #body>
        <div class="flex flex-col gap-2">
          <div v-for="r in COND_ROWS" :key="r.key" class="flex items-center justify-between gap-3">
            <span class="inline-flex items-center gap-2 text-sm min-w-0"><UIcon :name="r.icon" class="size-4 text-muted shrink-0" /><span class="truncate">{{ r.label }}</span></span>
            <UFieldGroup size="sm" class="shrink-0">
              <UButton v-for="o in (['yes', 'no', 'unknown'] as const)" :key="o" :label="o === 'yes' ? 'Oui' : o === 'no' ? 'Non' : '?'" :color="form[r.key] === o ? 'primary' : 'neutral'" :variant="form[r.key] === o ? 'solid' : 'outline'" class="w-11 justify-center" @click="form[r.key] = o" />
            </UFieldGroup>
          </div>
          <div class="flex items-center justify-between gap-3">
            <span class="inline-flex items-center gap-2 text-sm min-w-0"><UIcon name="i-lucide-timer" class="size-4 text-muted shrink-0" /><span class="truncate">Temps dans le chat</span></span>
            <UFieldGroup size="sm" class="shrink-0">
              <UButton v-for="o in (['yes', 'no', 'unknown'] as const)" :key="o" :label="o === 'yes' ? 'Oui' : o === 'no' ? 'Non' : '?'" :color="chatTime === o ? 'primary' : 'neutral'" :variant="chatTime === o ? 'solid' : 'outline'" class="w-11 justify-center" @click="chatTime = o" />
            </UFieldGroup>
          </div>
          <div v-if="chatTime === 'yes'" class="flex items-center justify-end gap-2 text-sm text-muted">
            au moins <UInputNumber v-model="chatMinutes" :min="1" :max="1440" size="sm" class="w-28" /> min
          </div>
        </div>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="condOpen = false" />
          <UButton label="Enregistrer" @click="saveConditions" />
        </div>
      </template>
    </UModal>
    <div v-if="watching" class="aspect-video bg-black">
      <iframe :src="playerUrl(g.channel)" :title="`Stream de ${g.channel}`" class="w-full h-full" allow="autoplay; fullscreen; picture-in-picture" allowfullscreen />
    </div>
  </article>
</template>
