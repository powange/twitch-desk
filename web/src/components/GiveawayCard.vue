<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ChannelLive, Giveaway, InstanceState } from '../types'
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

async function copy() {
  if (!g.value.command) return
  const ok = await copyText(g.value.command)
  toast.add(ok
    ? { title: 'Commande copiée', description: g.value.command, color: 'success', icon: 'i-lucide-check', duration: 2000 }
    : { title: 'Copie impossible', description: 'Sélectionne la commande à la main.', color: 'error' })
}
</script>

<template>
  <article class="rounded-xl border border-default bg-elevated/40 overflow-hidden" :class="g.status === 'paused' ? 'opacity-80' : ''">
    <div class="flex flex-col gap-3 p-4">
      <!-- Chaîne, type et statut -->
      <header class="flex flex-col gap-1.5">
        <div class="flex items-center gap-2 flex-wrap">
          <h2 class="text-lg font-semibold break-all">{{ g.channel }}</h2>
          <UBadge :label="kind.label" :color="kind.color" variant="subtle" size="sm" />
          <UBadge v-if="g.status === 'paused'" label="En pause" color="warning" variant="subtle" size="sm" icon="i-lucide-pause" />
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
    <div v-if="watching" class="aspect-video bg-black">
      <iframe :src="playerUrl(g.channel)" :title="`Stream de ${g.channel}`" class="w-full h-full" allow="autoplay; fullscreen; picture-in-picture" allowfullscreen />
    </div>
  </article>
</template>
