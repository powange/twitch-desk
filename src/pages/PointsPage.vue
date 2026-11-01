<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useChannelList } from '../composables/useChannelList'
import { useOpenExternal } from '../utils/external'
import { CHANNEL_STATE, formatNumber } from '../utils/format'
import type { ChannelView } from '../types'
import ChannelFlag from '../components/ChannelFlag.vue'
import AccountPicker from '../components/AccountPicker.vue'

const openExternal = useOpenExternal()
const { accounts, account, channels, newLogin, adding, addChannel, setOptions, toRemove, confirmRemove } = useChannelList('points')

// L'ordre ne concerne que cette liste : c'est lui qui décide des 2 chaînes regardées.
// Glisser-déposer par la poignée : la ligne n'est déplaçable que pendant qu'on la tient,
// pour ne pas gêner les interrupteurs et la sélection de texte.
const grabbed = ref<number | null>(null)
const dragging = ref<number | null>(null)
// Ordre affiché pendant le glisser (aperçu), null hors glisser
const preview = ref<number[] | null>(null)
// Ordre déposé, affiché tel quel jusqu'à ce que le cœur renvoie le même : sans ça,
// la ligne revenait à sa place le temps que l'état soit rafraîchi
const pending = ref<number[] | null>(null)
let pendingTimer: ReturnType<typeof setTimeout> | null = null

const ordered = computed(() => {
  const order = preview.value ?? pending.value
  if (!order) return channels.value
  const byId = new Map(channels.value.map(c => [c.id, c]))
  const list = order.map(id => byId.get(id)).filter((c): c is ChannelView => !!c)
  // Chaîne ajoutée entre-temps : à la fin
  return [...list, ...channels.value.filter(c => !order.includes(c.id))]
})

watch(channels, (list) => {
  if (pending.value && list.map(c => c.id).join() === pending.value.join()) clearPending()
})

function clearPending() {
  pending.value = null
  if (pendingTimer) clearTimeout(pendingTimer)
  pendingTimer = null
}

function onDragStart(event: DragEvent, channel: ChannelView) {
  dragging.value = channel.id
  preview.value = channels.value.map(c => c.id)
  event.dataTransfer?.setData('text/plain', channel.login)
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
}

function onDragOver(event: DragEvent, target: ChannelView) {
  if (dragging.value === null || !preview.value) return
  event.preventDefault()
  if (target.id === dragging.value) return
  const ids = [...preview.value]
  const from = ids.indexOf(dragging.value)
  const to = ids.indexOf(target.id)
  ids.splice(from, 1)
  ids.splice(to, 0, dragging.value)
  preview.value = ids
}

async function onDrop(event: DragEvent) {
  event.preventDefault()
  const ids = preview.value
  const changed = ids && ids.some((id, i) => id !== channels.value[i]?.id)
  if (changed) {
    pending.value = ids
    // Filet de sécurité : si le cœur ne confirme pas, on revient à son état
    pendingTimer = setTimeout(clearPending, 5000)
  }
  endDrag()
  if (changed && account.value) {
    try {
      await invoke('reorder_channels', { accountId: account.value.id, ids })
    } catch {
      clearPending()
    }
  }
}

function endDrag() {
  dragging.value = null
  grabbed.value = null
  preview.value = null
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <div class="flex items-center gap-3">
      <h1 class="text-xl font-semibold flex-1">Points de chaîne</h1>
      <AccountPicker />
    </div>

    <UAlert
      v-if="accounts.length === 0"
      title="Connecte d'abord un compte Twitch"
      :actions="[{ label: 'Comptes', to: '/accounts' }]"
      color="neutral"
      variant="subtle"
    />

    <template v-else-if="account">
      <UAlert
        icon="i-lucide-info"
        color="info"
        variant="subtle"
        title="Twitch ne compte le temps de visionnage que sur 2 chaînes à la fois par compte."
        description="L'application regarde les 2 premières chaînes en live de la liste. Change l'ordre en faisant glisser une ligne par sa poignée."
      />

      <UAlert
        v-if="!account.pointsEnabled"
        icon="i-lucide-pause"
        color="warning"
        variant="subtle"
        title="Récolte désactivée pour ce compte"
        description="Active-la dans Comptes."
      />

      <form class="flex gap-2" @submit.prevent="addChannel">
        <UInput id="new-points-channel" v-model="newLogin" placeholder="Nom ou adresse de la chaîne (ex. lunarsea)" class="flex-1" icon="i-lucide-plus" />
        <UButton type="submit" label="Ajouter" :loading="adding" />
      </form>

      <div class="border border-default rounded-lg overflow-x-auto">
        <table class="w-full text-sm">
          <thead class="bg-elevated/60 text-xs uppercase text-muted">
            <tr>
              <th class="text-left px-3 py-2 w-16">Ordre</th>
              <th class="text-left px-3 py-2">Chaîne</th>
              <th class="text-left px-3 py-2">État</th>
              <th class="text-right px-3 py-2">Solde</th>
              <th class="text-right px-3 py-2">Aujourd'hui</th>
              <th class="text-center px-3 py-2">Récolte</th>
              <th class="text-center px-3 py-2 whitespace-nowrap">Bonus auto</th>
              <th class="px-3 py-2" />
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="(channel, i) in ordered"
              :key="channel.id"
              class="border-t border-default transition-colors"
              :class="dragging === channel.id ? 'bg-primary/10 opacity-70' : ''"
              :draggable="grabbed === channel.id"
              @dragstart="onDragStart($event, channel)"
              @dragover="onDragOver($event, channel)"
              @drop="onDrop"
              @dragend="endDrag"
            >
              <td class="px-3 py-2">
                <div
                  class="flex items-center gap-1.5 cursor-grab active:cursor-grabbing select-none"
                  title="Glisser pour changer l'ordre"
                  @mousedown="grabbed = channel.id"
                  @mouseup="grabbed = null"
                >
                  <UIcon name="i-lucide-grip-vertical" class="size-4 text-muted" />
                  <span class="w-5 text-muted">{{ i + 1 }}</span>
                </div>
              </td>
              <td class="px-3 py-2 min-w-0">
                <div class="flex items-center gap-1.5">
                  <ChannelFlag :login="channel.login" />
                  <button class="font-medium hover:underline" @click="openExternal(`https://www.twitch.tv/${channel.login}`)">{{ channel.login }}</button>
                  <UTooltip v-if="channel.priority" text="Giveaway par points de chaîne en cours : regardée en priorité">
                    <UBadge label="Prioritaire" icon="i-lucide-gift" color="primary" variant="subtle" size="sm" />
                  </UTooltip>
                </div>
                <div v-if="channel.live" class="flex items-center gap-1.5 text-xs text-muted min-w-0">
                  <span class="truncate max-w-[320px]" :title="channel.live.gameName ?? 'Sans catégorie'">{{ channel.live.gameName ?? 'Sans catégorie' }}</span>
                  <span class="inline-flex items-center gap-0.5 whitespace-nowrap text-error" :title="`${formatNumber(channel.live.viewers)} spectateurs`">
                    <UIcon name="i-lucide-eye" class="size-3.5" />{{ formatNumber(channel.live.viewers) }}
                  </span>
                </div>
                <div v-if="channel.error" class="text-xs text-error truncate max-w-[320px]" :title="channel.error">{{ channel.error }}</div>
              </td>
              <td class="px-3 py-2">
                <UTooltip :text="CHANNEL_STATE[channel.state].hint" :disabled="!CHANNEL_STATE[channel.state].hint">
                  <UBadge :color="CHANNEL_STATE[channel.state].color" variant="subtle" :label="CHANNEL_STATE[channel.state].label" class="whitespace-nowrap" />
                </UTooltip>
              </td>
              <td class="px-3 py-2 text-right">{{ formatNumber(channel.balance) }}</td>
              <td class="px-3 py-2 text-right" :class="channel.earnedToday > 0 ? 'text-success' : 'text-muted'">
                {{ channel.earnedToday > 0 ? '+' : '' }}{{ formatNumber(channel.earnedToday) }}
              </td>
              <td class="px-3 py-2 text-center">
                <div class="flex justify-center">
                  <USwitch :model-value="channel.enabled" :aria-label="`Récolter sur ${channel.login}`" @update:model-value="v => setOptions(channel, { enabled: v })" />
                </div>
              </td>
              <td class="px-3 py-2 text-center">
                <div class="flex justify-center">
                  <USwitch :model-value="channel.autoClaim" :aria-label="`Bonus auto sur ${channel.login}`" @update:model-value="v => setOptions(channel, { autoClaim: v })" />
                </div>
              </td>
              <td class="px-3 py-2 text-right">
                <UButton icon="i-lucide-trash-2" size="xs" variant="ghost" color="error" aria-label="Retirer de la liste" @click="toRemove = channel" />
              </td>
            </tr>
            <tr v-if="channels.length === 0">
              <td colspan="8" class="px-3 py-8 text-center text-muted">Aucune chaîne. Ajoute la première avec le champ ci-dessus.</td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>

    <UModal :open="toRemove !== null" title="Retirer la chaîne ?" @update:open="v => { if (!v) toRemove = null }">
      <template #body>
        <p>
          {{ toRemove?.login }} ne sera plus regardée par {{ account?.login }}.
          <template v-if="toRemove?.inSe">Elle reste dans la page StreamElements.</template>
          L'historique des gains est conservé.
        </p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="toRemove = null" />
          <UButton label="Retirer" color="error" @click="confirmRemove" />
        </div>
      </template>
    </UModal>
  </div>
</template>
