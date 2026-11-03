<script setup lang="ts">
import ChannelFlag from './ChannelFlag.vue'
import { useOpenExternal } from '../utils/external'
import { channelUrl, formatNumber } from '../utils/format'

// Cellule « Chaîne » des tableaux : photo de profil, drapeau de la langue, nom (ouvre la chaîne
// dans le navigateur) et, en live, catégorie et spectateurs dessous (`showLive` à false : Scanner,
// qui a ses propres colonnes). Emplacements : `badges` après le nom, défaut sous la chaîne
withDefaults(defineProps<{
  login: string
  name?: string | null
  avatar?: string | null
  lang?: string | null
  live?: { gameName: string | null, viewers: number } | null
  showLive?: boolean
}>(), { name: null, avatar: null, lang: undefined, live: null, showLive: true })
const openExternal = useOpenExternal()
</script>

<template>
  <div class="flex items-center gap-2">
    <img v-if="avatar" :src="avatar" alt="" class="size-7 rounded-full shrink-0">
    <div class="min-w-0">
      <div class="flex items-center gap-1.5 whitespace-nowrap">
        <ChannelFlag :login="login" :lang="lang" />
        <button class="font-medium hover:underline" @click="openExternal(channelUrl(login))">{{ name || login }}</button>
        <slot name="badges" />
      </div>
      <div v-if="showLive && live" class="flex items-center gap-1.5 text-xs text-muted min-w-0">
        <span class="truncate max-w-[220px]" :title="live.gameName ?? 'Sans catégorie'">{{ live.gameName ?? 'Sans catégorie' }}</span>
        <span class="inline-flex items-center gap-0.5 whitespace-nowrap text-error" :title="`${formatNumber(live.viewers)} spectateurs`">
          <UIcon name="i-lucide-eye" class="size-3.5" />{{ formatNumber(live.viewers) }}
        </span>
      </div>
      <slot />
    </div>
  </div>
</template>
