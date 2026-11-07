<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ChannelLive, Giveaway, InstanceState, PhoneAction } from '../types'
import { ApiError, giveawayChatLog, hideGiveaway } from '../api'
import { pendingFor, sendAction, usePhone } from '../phone'
import { chatUrl, conditions, copyText, GIVEAWAY_KIND, playerUrl, streamUrl, type Condition } from '../giveaways'
import { formatDraw, formatNumber, formatWhen } from '../format'
import GiftImage from './GiftImage.vue'
import ChannelFlag from './ChannelFlag.vue'

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

// Participer (commande envoyée, ou récompense achetée) avec les comptes qui n'y sont pas encore
const missing = computed(() => {
  const done = (mine.value?.participated ?? []).map(l => l.toLowerCase())
  return (props.state?.accounts ?? []).filter(a => !done.includes(a.toLowerCase()))
})
const canJoin = computed(() => g.value.status === 'open' && missing.value.length > 0
  && (((g.value.kind === 'command' || g.value.kind === 'ticket') && !!g.value.command)
    || (g.value.kind === 'channelpoints' && !!g.value.rewardName && !!props.state?.actionsAllowed)))
const act = (kind: string) => sendAction(phone, toast, { kind, item: g.value.id })
// Suppression (comme sur le PC), après confirmation
const deleteOpen = ref(false)
function confirmDelete() {
  deleteOpen.value = false
  void act('giveaway_delete')
}
const openChatLog = () => ask(t => giveawayChatLog<PhoneAction>(t, g.value.id))

// Conditions cliquables, comme sur le PC (giveaway pas encore clos) : follow (fait suivre la
// chaîne aux comptes qui ne la suivent pas), bonjour (fenêtre de confirmation des phrases),
// Discord (invitation du streamer) ; état de mes comptes publié par le PC
const discordUrl = computed(() => props.state?.discord?.[g.value.channel.toLowerCase()] ?? null)
const channelState = computed(() => props.state?.conditions?.[g.value.channel.toLowerCase()] ?? null)
const winReply = computed(() => channelState.value?.winReply ?? null)
const followAge = (at: string | null) => at ? (Date.now() - new Date(at).getTime()) / 1000 : null
const followInfo = computed(() => {
  const list = channelState.value?.follows
  if (!list) return null
  const min = g.value.followMinSecs ?? 0
  const missing = list.filter(f => f.follows === false).map(f => f.login)
  const recent = list.filter(f => f.follows && min > 0 && (followAge(f.followedAt) ?? 0) < min).map(f => f.login)
  const unknown = list.filter(f => f.follows === null).map(f => f.login)
  return { missing, recent, ok: list.length > 0 && !missing.length && !recent.length && !unknown.length }
})
const greetInfo = computed(() => {
  const list = channelState.value?.greetings
  if (!list) return null
  const missing = list.filter(x => !x.greeted).map(x => x.login)
  return { missing, ok: list.length > 0 && !missing.length }
})
interface Chip extends Condition { done?: boolean, loading?: boolean, href?: string, action?: () => void }
const info = (title: string, description: string, icon: string) => toast.add({ title, description, color: 'info', icon })
const chips = computed<Chip[]>(() => conds.value.map((c): Chip => {
  if (!c.required || g.value.status === 'closed') return c
  const channel = g.value.channel
  switch (c.key) {
    case 'follow': {
      const f = followInfo.value
      if (f?.ok) return { ...c, done: true, colorClass: 'text-success', action: () => info('Follow', `Tous tes comptes suivent ${channel}${g.value.followMinSecs ? ' depuis assez longtemps' : ''}.`, 'i-lucide-heart') }
      if (f && !f.missing.length && f.recent.length) {
        return { ...c, label: `${c.label} : trop récent`, colorClass: 'text-warning', action: () => info('Follow trop récent', `${f.recent.join(', ')} suit ${channel} depuis moins longtemps que demandé.`, 'i-lucide-heart') }
      }
      return { ...c, label: f?.missing.length ? `${c.label} : ${f.missing.join(', ')}` : c.label, loading: busy('giveaway_follow'), action: () => act('giveaway_follow') }
    }
    case 'greeting': {
      const h = greetInfo.value
      if (h?.ok) return { ...c, done: true, colorClass: 'text-success', action: () => info('Bonjour', `Déjà dit sur ${channel} par tous tes comptes.`, 'i-lucide-hand') }
      return { ...c, label: h?.missing.length ? `${c.label} : ${h.missing.join(', ')}` : c.label, loading: busy('giveaway_hello_draft') || busy('giveaway_hello'), action: () => act('giveaway_hello_draft') }
    }
    case 'discord':
      return discordUrl.value ? { ...c, href: discordUrl.value } : { ...c, loading: busy('giveaway_discord'), action: () => act('giveaway_discord') }
    default:
      return c
  }
}))

async function copy() {
  if (!g.value.command) return
  const ok = await copyText(g.value.command)
  toast.add(ok
    ? { title: 'Commande copiée', description: g.value.command, color: 'success', icon: 'i-lucide-check', duration: 2000 }
    : { title: 'Copie impossible', description: 'Sélectionne la commande à la main.', color: 'error' })
}
</script>

<template>
  <article class="rounded-xl border border-default bg-elevated/40 overflow-hidden" :class="g.status !== 'open' || hidden ? 'opacity-80' : ''">
    <div class="flex flex-col gap-3 p-4">
      <!-- Chaîne, type et statut -->
      <header class="flex flex-col gap-1.5">
        <div class="flex items-center gap-2 flex-wrap">
          <img v-if="state?.avatars?.[g.channel.toLowerCase()]" :src="state.avatars[g.channel.toLowerCase()]" alt="" class="size-8 rounded-full shrink-0" loading="lazy">
          <ChannelFlag :login="g.channel" :langs="state?.languages" />
          <h2 class="text-lg font-semibold break-all">{{ g.channel }}</h2>
          <!-- StreamElements : page StreamElements de la web app, filtrée sur cette chaîne -->
          <button v-if="g.kind === 'streamelements' && phone.openSe" type="button" class="inline-flex rounded focus-visible:outline-2 focus-visible:outline-primary" :aria-label="`Page StreamElements de ${g.channel}`" @click="phone.openSe(g.channel)">
            <UBadge :label="kind.label" :color="kind.color" variant="subtle" size="sm" trailing-icon="i-lucide-chevron-right" class="cursor-pointer" />
          </button>
          <UBadge v-else :label="kind.label" :color="kind.color" variant="subtle" size="sm" />
          <UBadge v-if="g.status === 'paused'" label="En pause" color="warning" variant="subtle" size="sm" icon="i-lucide-pause" />
          <UBadge v-if="g.status === 'closed'" :label="g.closedAt ? `Clos ${formatWhen(g.closedAt)}` : 'Clos'" color="neutral" variant="subtle" size="sm" icon="i-lucide-lock" />
          <UBadge v-if="hidden && g.status !== 'closed'" label="Masqué" color="neutral" variant="subtle" size="sm" icon="i-lucide-eye-off" />
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

      <!-- Participation de mes comptes (pas connue du PC pour un giveaway clos) -->
      <div v-if="state && (g.status !== 'closed' || mine)" class="flex items-center gap-2 text-sm">
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
      <ul v-if="chips.length" class="flex flex-wrap gap-1.5" aria-label="Conditions">
        <li v-for="c in chips" :key="c.key" class="inline-flex items-center gap-1 min-w-0">
          <component
            :is="c.href ? 'a' : c.action ? 'button' : 'span'"
            :href="c.href"
            :target="c.href ? '_blank' : undefined"
            :rel="c.href ? 'noopener' : undefined"
            :type="c.action ? 'button' : undefined"
            :disabled="c.loading || undefined"
            class="inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs min-w-0"
            :class="[c.colorClass, c.href || c.action ? 'border-current/40 active:bg-elevated focus-visible:outline-2 focus-visible:outline-primary' : 'border-default']"
            @click="c.action?.()"
          >
            <span class="relative inline-flex shrink-0">
              <UIcon :name="c.loading ? 'i-lucide-loader-circle' : c.icon" class="size-3.5" :class="c.loading ? 'animate-spin' : ''" />
              <UIcon v-if="c.done" name="i-lucide-check" class="absolute -bottom-1 -right-1.5 size-2.5 bg-default rounded-full" />
            </span>
            <span class="truncate" :class="c.required ? '' : 'line-through decoration-1'">{{ c.label }}</span>
            <UIcon v-if="c.href" name="i-lucide-external-link" class="size-3 shrink-0 opacity-70" />
          </component>
          <!-- Présence requise : réponse écrite automatiquement si un de mes comptes gagne -->
          <UPopover v-if="c.key === 'presence' && c.required && winReply">
            <button type="button" class="inline-flex rounded-full p-0.5 text-primary focus-visible:outline-2 focus-visible:outline-primary" :aria-label="`Réponse automatique si tu gagnes : ${winReply.text}`">
              <UIcon name="i-lucide-message-circle-reply" class="size-4" />
            </button>
            <template #content>
              <div class="flex flex-col gap-1.5 p-3 max-w-xs text-sm">
                <div class="flex items-center gap-1.5 font-medium">
                  <UIcon name="i-lucide-message-circle-reply" class="size-4 text-primary" />
                  Réponse automatique si tu gagnes
                </div>
                <p class="whitespace-pre-wrap break-words">« {{ winReply.text }} »</p>
                <p class="text-xs text-muted">Écrite dans le chat par le compte gagnant, {{ winReply.delay }} s après l'annonce (annulable){{ winReply.text.includes('\n') ? ' ; une phrase par ligne, l\'une après l\'autre' : '' }}.</p>
              </div>
            </template>
          </UPopover>
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

      <!-- Participer, raccourcis des conditions -->
      <UButton
        v-if="canJoin"
        :label="g.kind === 'channelpoints' ? `Acheter avec ${missing.join(', ')}` : `Participer avec ${missing.join(', ')}`"
        :icon="g.kind === 'channelpoints' ? 'i-lucide-coins' : 'i-lucide-send'"
        size="lg"
        block
        :loading="busy('giveaway_participate')"
        @click="act('giveaway_participate')"
      />
      <!-- Demandes au PC -->
      <div class="grid gap-2" :class="g.status === 'closed' ? 'grid-cols-2' : 'grid-cols-3'">
        <UButton
          v-if="g.status !== 'closed'"
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
        <UButton label="Modifier" icon="i-lucide-pencil" color="neutral" variant="soft" size="sm" block :loading="busy('giveaway_form') || busy('giveaway_save')" @click="act('giveaway_form')" />
      </div>
      <UButton label="Supprimer" icon="i-lucide-trash-2" color="error" variant="soft" size="sm" block :loading="busy('giveaway_delete')" @click="deleteOpen = true" />

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
    <UModal v-model:open="deleteOpen" :title="`Supprimer le giveaway de ${g.channel} ?`" description="Il est supprimé sur le PC et sur tous les appareils synchronisés. Pour seulement ne plus le voir, choisis plutôt « Masquer ».">
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="deleteOpen = false" />
          <UButton label="Supprimer" color="error" icon="i-lucide-trash-2" @click="confirmDelete" />
        </div>
      </template>
    </UModal>
    <div v-if="watching" class="aspect-video bg-black">
      <iframe :src="playerUrl(g.channel)" :title="`Stream de ${g.channel}`" class="w-full h-full" allow="autoplay; fullscreen; picture-in-picture" allowfullscreen />
    </div>
  </article>
</template>
