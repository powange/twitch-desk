<script lang="ts">
// Fiche publique d'une chaîne Twitch (renvoyée par le cœur : inspect_giveaway_channel)
export interface ChannelProfile {
  login: string
  displayName: string
  description: string | null
  profileImage: string | null
  language: string | null
  followers: number | null
  live: boolean
  viewers: number | null
  gameId: string | null
  gameName: string | null
  title: string | null
  startedAt: string | null
  pinnedMessage: string | null
  pinnedBy: string | null
  discord: string | null
}
</script>

<script setup lang="ts">
import { computed } from 'vue'
import ChannelFlag from './ChannelFlag.vue'
import { useOpenExternal } from '../utils/external'
import { boxArt, channelUrl, formatNumber } from '../utils/format'

const props = defineProps<{ profile: ChannelProfile }>()
const discord = computed(() => props.profile.discord)
const openExternal = useOpenExternal()

// Durée du live : « 2 h 05 »
function liveFor(iso: string | null) {
  if (!iso) return ''
  const min = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 60000))
  const h = Math.floor(min / 60)
  return h ? `${h} h ${String(min % 60).padStart(2, '0')}` : `${min} min`
}
</script>

<template>
  <div class="border border-default rounded-lg p-4 flex flex-col gap-3">
    <div class="flex items-start gap-3">
      <img v-if="profile.profileImage" :src="profile.profileImage" alt="" class="size-14 rounded-full object-cover shrink-0" :class="profile.live ? 'ring-2 ring-error ring-offset-2 ring-offset-default' : ''">
      <div class="flex-1 min-w-0 flex flex-col gap-1">
        <div class="flex items-center gap-2 flex-wrap">
          <ChannelFlag :login="profile.login" :lang="profile.language" />
          <button type="button" class="font-semibold text-base hover:underline" @click="openExternal(channelUrl(profile.login))">{{ profile.displayName || profile.login }}</button>
          <UBadge v-if="profile.live" label="En live" color="error" variant="solid" size="sm" />
          <UBadge v-else label="Hors ligne" color="neutral" variant="subtle" size="sm" />
          <span v-if="profile.followers !== null" class="text-xs text-muted">{{ formatNumber(profile.followers) }} followers</span>
        </div>
        <div v-if="profile.live" class="flex items-center gap-3 text-sm flex-wrap">
          <span class="inline-flex items-center gap-1 text-error font-medium" :title="`${formatNumber(profile.viewers ?? 0)} spectateurs`"><UIcon name="i-lucide-eye" class="size-4" />{{ formatNumber(profile.viewers ?? 0) }}</span>
          <span v-if="profile.gameName" class="inline-flex items-center gap-1.5">
            <img v-if="profile.gameId" :src="boxArt(null, profile.gameId)" alt="" class="w-4 h-5 rounded-sm object-cover">{{ profile.gameName }}
          </span>
          <span class="text-muted">depuis {{ liveFor(profile.startedAt) }}</span>
        </div>
        <div v-else-if="profile.gameName" class="text-sm text-muted">Dernière catégorie : {{ profile.gameName }}</div>
        <div v-if="profile.title" class="text-sm">{{ profile.title }}</div>
      </div>
    </div>
    <p v-if="profile.description" class="text-sm text-muted whitespace-pre-line">{{ profile.description }}</p>
    <UButton v-if="discord" :label="(discord ?? '').replace('https://', '')" icon="i-simple-icons-discord" size="xs" color="neutral" variant="soft" class="self-start" @click="openExternal(discord ?? '')" />
    <div v-if="profile.pinnedMessage" class="flex items-start gap-2 rounded-md bg-elevated px-3 py-2 text-sm">
      <UIcon name="i-lucide-pin" class="size-4 text-muted mt-0.5 shrink-0" />
      <div class="min-w-0">
        <div class="text-xs text-muted">Message épinglé<template v-if="profile.pinnedBy"> par {{ profile.pinnedBy }}</template></div>
        <div class="break-words">{{ profile.pinnedMessage }}</div>
      </div>
    </div>
  </div>
</template>
