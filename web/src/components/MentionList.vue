<script setup lang="ts">
import { computed, ref } from 'vue'
import type { Mention } from '../types'
import { formatWhen } from '../format'
import { chatUrl } from '../giveaways'
import ChannelFlag from './ChannelFlag.vue'

// Mentions de nos pseudos dans les chats surveillés (accueil), les plus récentes d'abord ;
// les 5 dernières, puis toutes sur demande
// `highlight` : chaîne d'une notification touchée, sa mention la plus récente mise en évidence
const props = defineProps<{ mentions: Mention[], langs?: Record<string, string> | null, avatars?: Record<string, string> | null, highlight?: string | null }>()
const all = ref(false)
const shown = computed(() => all.value ? props.mentions : props.mentions.slice(0, 5))
const highlightedId = computed(() => props.highlight ? shown.value.find(m => m.channel.toLowerCase() === props.highlight)?.id ?? null : null)
</script>

<template>
  <section class="flex flex-col gap-3" aria-labelledby="mentions-title">
    <h1 id="mentions-title" class="text-sm font-medium text-muted uppercase">Mentions · {{ mentions.length }}</h1>
    <ul class="flex flex-col gap-2">
      <li
        v-for="m in shown"
        :id="highlightedId === m.id ? `mention-${m.channel.toLowerCase()}` : undefined"
        :key="m.id"
        class="rounded-xl border bg-elevated/40 p-3 flex flex-col gap-1.5 transition-colors"
        :class="highlightedId === m.id ? 'border-primary ring-2 ring-primary/40' : 'border-default'"
      >
        <div class="flex items-start gap-2">
          <div class="min-w-0 flex-1 text-sm">
            <b class="break-words">{{ m.author }}</b> mentionne <b class="break-words">{{ m.account }}</b>
            <span class="text-muted"> sur </span><img v-if="avatars?.[m.channel.toLowerCase()]" :src="avatars[m.channel.toLowerCase()]" alt="" class="inline-block align-middle size-5 rounded-full" loading="lazy"> <ChannelFlag :login="m.channel" :langs="langs" class="inline-block align-middle" /> <b class="break-words">{{ m.channel }}</b>
          </div>
          <span class="text-xs text-muted whitespace-nowrap shrink-0">{{ formatWhen(m.at) }}</span>
        </div>
        <p class="text-sm break-words">{{ m.message }}</p>
        <div>
          <UButton label="Ouvrir le chat" icon="i-lucide-message-square" size="xs" color="neutral" variant="soft" :to="chatUrl(m.channel)" target="_blank" />
        </div>
      </li>
    </ul>
    <UButton v-if="mentions.length > 5" :label="all ? 'Voir moins' : `Voir les ${mentions.length} mentions`" color="neutral" variant="ghost" size="sm" class="self-center" @click="all = !all" />
  </section>
</template>
