<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'

// Journal du chat d'une chaîne : commandes des spectateurs, nos messages, annonces de gagnant
// et réponses des bots aux commandes d'information (!giveaway…), gardés 14 jours
interface ChatLogEntry { id: number, channel: string, at: string, author: string, kind: Kind, command: string | null, message: string }
interface BotReply { channel: string, command: string, bot: string, message: string, askedBy: string, updatedAt: string, seenAt: string }
type Kind = 'command' | 'own' | 'winner' | 'reply'

const props = defineProps<{ channel: string | null }>()
const open = defineModel<boolean>('open', { required: true })
const toast = useToast()

const KINDS: Record<Kind, { label: string, color: 'neutral' | 'primary' | 'success' | 'info' }> = {
  command: { label: 'Commande', color: 'neutral' },
  own: { label: 'Mon message', color: 'primary' },
  winner: { label: 'Gagnant', color: 'success' },
  reply: { label: 'Réponse de bot', color: 'info' }
}
const ALL = 'all'
const kindItems = [
  { label: 'Tous les messages', value: ALL },
  { label: 'Commandes', value: 'command' },
  { label: 'Mes messages', value: 'own' },
  { label: 'Gagnants', value: 'winner' },
  { label: 'Réponses de bots', value: 'reply' }
]

const entries = ref<ChatLogEntry[]>([])
const commands = ref<[string, number][]>([])
const replies = ref<BotReply[]>([])
const kind = ref<string>(ALL)
const command = ref<string | undefined>(undefined)
const loading = ref(false)
const more = ref(false)
const PAGE = 200

const commandItems = computed(() => commands.value.map(([c, n]) => ({ label: `${c} (${n})`, value: c })))

async function load(append = false) {
  if (!props.channel) return
  loading.value = true
  try {
    const before = append ? entries.value.at(-1)?.id ?? null : null
    const page = await invoke<ChatLogEntry[]>('get_chat_log', {
      channel: props.channel,
      kinds: kind.value === ALL ? [] : [kind.value],
      command: command.value ?? null,
      beforeId: before
    })
    entries.value = append ? [...entries.value, ...page] : page
    more.value = page.length === PAGE
  } catch (e) {
    toast.add({ title: 'Journal illisible', description: String(e), color: 'error' })
  } finally {
    loading.value = false
  }
}

async function refresh() {
  if (!props.channel) return
  try {
    const meta = await invoke<{ commands: [string, number][], replies: BotReply[] }>('get_chat_log_meta', { channel: props.channel })
    commands.value = meta.commands
    replies.value = meta.replies
  } catch {
    // Les filtres restent ceux d'avant
  }
  await load()
}

watch(open, (isOpen) => {
  if (!isOpen) return
  kind.value = ALL
  command.value = undefined
  entries.value = []
  void refresh()
})
watch([kind, command], () => { if (open.value) void load() })

function time(iso: string) {
  const d = new Date(iso)
  const today = new Date().toDateString() === d.toDateString()
  const t = d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit', second: '2-digit' })
  return today ? t : `${d.toLocaleDateString('fr-FR', { day: 'numeric', month: 'short' })} ${t}`
}
</script>

<template>
  <UModal v-model:open="open" :title="`Journal du chat de ${channel ?? ''}`" description="Commandes des spectateurs, tes messages, annonces de gagnant et réponses des bots, gardés 14 jours." :ui="{ content: 'max-w-5xl' }">
    <template #body>
      <div class="flex flex-col gap-3">
        <!-- Réponses des bots aux commandes d'information -->
        <div v-if="replies.length" class="flex flex-col gap-1.5">
          <div class="text-sm font-medium">Réponses des bots</div>
          <div v-for="r in replies" :key="r.command" class="border border-default rounded-md px-3 py-2 text-sm flex flex-col gap-0.5">
            <div class="flex items-center gap-2 flex-wrap text-xs text-muted">
              <span class="font-mono text-default">{{ r.command }}</span>
              <span>→ {{ r.bot }}</span>
              <span>· demandé par {{ r.askedBy }}</span>
              <span>· modifiée {{ time(r.updatedAt) }}</span>
              <span v-if="r.seenAt !== r.updatedAt">· revue {{ time(r.seenAt) }}</span>
            </div>
            <div class="break-words">{{ r.message }}</div>
          </div>
        </div>

        <div class="flex items-center gap-2 flex-wrap">
          <USelect v-model="kind" :items="kindItems" class="w-52" aria-label="Type de message" />
          <USelectMenu
            v-model="command"
            :items="commandItems"
            value-key="value"
            placeholder="Toutes les commandes"
            :search-input="{ placeholder: 'Rechercher une commande' }"
            class="w-60"
            aria-label="Commande"
          />
          <UButton v-if="command" label="Toutes les commandes" size="xs" color="neutral" variant="ghost" icon="i-lucide-x" @click="command = undefined" />
          <span class="flex-1" />
          <UButton icon="i-lucide-refresh-cw" color="neutral" variant="soft" :loading="loading" aria-label="Actualiser" title="Actualiser" @click="refresh" />
        </div>

        <div class="border border-default rounded-lg overflow-auto max-h-[55vh]">
          <table class="w-full text-sm">
            <thead class="bg-elevated/60 text-xs uppercase text-muted sticky top-0">
              <tr>
                <th class="text-left px-3 py-2">Heure</th>
                <th class="text-left px-3 py-2">Auteur</th>
                <th class="text-left px-3 py-2">Type</th>
                <th class="text-left px-3 py-2">Commande</th>
                <th class="text-left px-3 py-2">Message</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="e in entries" :key="e.id" class="border-t border-default align-top">
                <td class="px-3 py-1.5 whitespace-nowrap text-muted tabular-nums">{{ time(e.at) }}</td>
                <td class="px-3 py-1.5 whitespace-nowrap font-medium">{{ e.author }}</td>
                <td class="px-3 py-1.5"><UBadge :label="KINDS[e.kind]?.label ?? e.kind" :color="KINDS[e.kind]?.color ?? 'neutral'" variant="subtle" size="sm" class="whitespace-nowrap" /></td>
                <td class="px-3 py-1.5 whitespace-nowrap font-mono">
                  <button v-if="e.command" type="button" class="hover:underline" title="Filtrer sur cette commande" @click="command = e.command">{{ e.command }}</button>
                  <span v-else class="text-muted">—</span>
                </td>
                <td class="px-3 py-1.5 break-words min-w-64">{{ e.message }}</td>
              </tr>
              <tr v-if="!loading && entries.length === 0">
                <td colspan="5" class="px-3 py-8 text-center text-muted">Rien d'enregistré pour l'instant sur cette chaîne avec ces filtres.</td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-if="more" class="flex justify-center">
          <UButton label="Messages plus anciens" color="neutral" variant="soft" :loading="loading" @click="load(true)" />
        </div>
      </div>
    </template>
  </UModal>
</template>
