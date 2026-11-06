<script setup lang="ts">
import type { ChatLogEntry } from '../types'

// Journal du chat d'un giveaway, lu par le PC : commandes des spectateurs, annonces des bots,
// messages du streamer et de mes comptes, du plus ancien au plus récent
defineProps<{ log: { channel: string, entries: ChatLogEntry[] } | null }>()
const open = defineModel<boolean>('open', { default: false })

const KIND: Record<string, { label: string, color: 'primary' | 'warning' | 'success' | 'info' | 'neutral' }> = {
  command: { label: 'Commande', color: 'neutral' },
  own: { label: 'Moi', color: 'primary' },
  streamer: { label: 'Streamer', color: 'info' },
  winner: { label: 'Gagnant', color: 'success' },
  reply: { label: 'Réponse', color: 'warning' },
  bot: { label: 'Bot', color: 'warning' }
}
const time = (at: string) => new Date(at).toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' })
</script>

<template>
  <UModal v-model:open="open" :title="log ? `Journal du chat de ${log.channel}` : 'Journal du chat'" description="Pendant ce giveaway, du plus ancien au plus récent">
    <template #body>
      <p v-if="!log?.entries.length" class="text-sm text-muted">Rien d'enregistré pendant ce giveaway.</p>
      <ul v-else class="flex flex-col gap-2">
        <li v-for="(e, i) in log.entries" :key="i" class="flex flex-col gap-0.5 min-w-0">
          <div class="flex items-center gap-1.5 flex-wrap text-xs">
            <span class="text-muted tabular-nums">{{ time(e.at) }}</span>
            <span class="font-medium break-all">{{ e.author }}</span>
            <UBadge :label="KIND[e.kind]?.label ?? e.kind" :color="KIND[e.kind]?.color ?? 'neutral'" variant="subtle" size="xs" />
          </div>
          <p class="text-sm break-words">{{ e.message }}</p>
        </li>
      </ul>
    </template>
  </UModal>
</template>
