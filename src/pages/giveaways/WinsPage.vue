<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../../composables/useSnapshot'
import { useGiftCatalog } from '../../composables/useGiftCatalog'
import { useOpenExternal } from '../../utils/external'
import { channelUrl } from '../../utils/format'
import ChannelFlag from '../../components/ChannelFlag.vue'
import type { GiveawayView } from '../../types'

const { snapshot } = useSnapshot()
const { byId } = useGiftCatalog()
const openExternal = useOpenExternal()
const toast = useToast()

const wins = ref<GiveawayView[]>([])
async function load() {
  try {
    wins.value = await invoke<GiveawayView[]>('get_wins')
  } catch (e) {
    toast.add({ title: 'Gains non chargés', description: String(e), color: 'error' })
  }
}
onMounted(load)
// Un tirage gagné ou un lot modifié arrive par l'état : on relit la liste
watch(() => snapshot.value?.giveaways.giveaways.filter(g => g.winnerOurs).map(g => `${g.id}:${g.prizeReceived}`).join(), load)

const pending = computed(() => wins.value.filter(w => !w.prizeReceived).length)
const notes = reactive<Record<number, string>>({})
function noteOf(w: GiveawayView) {
  return notes[w.id] ?? w.prizeNote ?? ''
}

async function setPrize(w: GiveawayView, received: boolean, note = noteOf(w)): Promise<boolean> {
  try {
    await invoke('set_prize', { id: w.id, received, note: note || null })
  } catch (e) {
    toast.add({ title: 'Lot non enregistré', description: String(e), color: 'error' })
    return false
  }
  await load()
  return true
}
// Note du lot : enregistrée une seule fois (Entrée puis sortie du champ) et seulement si elle a changé
async function saveNote(w: GiveawayView) {
  const note = noteOf(w).trim()
  if (note === (w.prizeNote ?? '').trim()) return
  // Note gardée comme « enregistrée » tout de suite : la sortie du champ qui suit Entrée ne renvoie rien
  const previous = w.prizeNote
  w.prizeNote = note || null
  if (!(await setPrize(w, w.prizeReceived, note))) w.prizeNote = previous
}

function formatDate(iso: string | null) {
  return iso ? new Date(iso).toLocaleDateString('fr-FR', { day: 'numeric', month: 'short', year: 'numeric' }) : '—'
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <div class="flex items-baseline gap-3">
      <h1 class="text-xl font-semibold">Mes gains</h1>
      <span v-if="wins.length" class="text-sm text-muted">{{ wins.length }} giveaway(s) gagné(s) · {{ pending }} lot(s) en attente</span>
    </div>

    <div class="border border-default rounded-lg overflow-x-auto">
      <table class="w-full text-sm">
        <thead class="bg-elevated/60 text-xs uppercase text-muted">
          <tr>
            <th class="text-left px-3 py-2">Tirage</th>
            <th class="text-left px-3 py-2">Chaîne</th>
            <th class="text-left px-3 py-2">Compte</th>
            <th class="text-left px-3 py-2">Cadeaux</th>
            <th class="text-left px-3 py-2">Lot reçu</th>
            <th class="text-left px-3 py-2">Note (clé, code…)</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="w in wins" :key="w.id" class="border-t border-default" :class="w.prizeReceived ? '' : 'bg-warning/5'">
            <td class="px-3 py-2 whitespace-nowrap">{{ formatDate(w.drawnAt ?? w.detectedAt) }}</td>
            <td class="px-3 py-2">
              <div class="flex items-center gap-1.5">
                <ChannelFlag :login="w.channel" />
                <button class="font-medium hover:underline" @click="openExternal(channelUrl(w.channel))">{{ w.channel }}</button>
              </div>
            </td>
            <td class="px-3 py-2 font-medium text-success">{{ w.winnerOurs }}</td>
            <td class="px-3 py-2">
              <div class="flex flex-wrap gap-2">
                <div v-for="id in w.giftIds" :key="id" class="flex items-center gap-1.5">
                  <img v-if="byId.get(id)?.image" :src="byId.get(id)?.image ?? ''" referrerpolicy="no-referrer" alt="" class="size-7 rounded object-contain bg-elevated">
                  <span>{{ byId.get(id)?.title }}</span>
                </div>
                <span v-if="w.giftIds.length === 0" class="text-muted">—</span>
              </div>
            </td>
            <td class="px-3 py-2">
              <UFieldGroup>
                <UButton label="Reçu" size="xs" :color="w.prizeReceived ? 'success' : 'neutral'" :variant="w.prizeReceived ? 'solid' : 'outline'" @click="setPrize(w, true)" />
                <UButton label="En attente" size="xs" :color="!w.prizeReceived ? 'warning' : 'neutral'" :variant="!w.prizeReceived ? 'solid' : 'outline'" @click="setPrize(w, false)" />
              </UFieldGroup>
            </td>
            <td class="px-3 py-2">
              <UInput
                :id="`prize-note-${w.id}`"
                :model-value="noteOf(w)"
                size="sm"
                class="w-64 font-mono"
                placeholder="—"
                @update:model-value="v => notes[w.id] = String(v)"
                @blur="saveNote(w)"
                @keydown.enter="saveNote(w)"
              />
            </td>
          </tr>
          <tr v-if="wins.length === 0">
            <td colspan="6" class="px-3 py-8 text-center text-muted">Aucun gain pour l'instant. Quand un bot annonce l'un de tes comptes comme gagnant, le giveaway apparaît ici.</td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>
