<script setup lang="ts">
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useChannelList } from '../composables/useChannelList'
import { useDragReorder } from '../composables/useDragReorder'
import { useOpenExternal } from '../utils/external'
import { CHANNEL_STATE, formatNumber } from '../utils/format'
import type { ChannelView } from '../types'
import ChannelFlag from '../components/ChannelFlag.vue'
import AccountPicker from '../components/AccountPicker.vue'
import PointsChannelSettings from '../components/PointsChannelSettings.vue'

const openExternal = useOpenExternal()
const { accounts, account, channels, newLogin, adding, addChannel, setOptions, optionsOf, toRemove, removing, confirmRemove } = useChannelList('points')

// Réglages d'une chaîne (solde maximum…) : la vue à jour est relue par son identifiant
const settingsId = ref<number | null>(null)
const settingsOpen = ref(false)
const settingsChannel = computed(() => channels.value.find(c => c.id === settingsId.value) ?? null)
function openSettings(channel: ChannelView) {
  settingsId.value = channel.id
  settingsOpen.value = true
}

// L'ordre ne concerne que cette liste : c'est lui qui décide des 2 chaînes regardées.
// Glisser-déposer par la poignée, ou Alt+flèches sur la poignée
const { grabbed, dragging, ordered, onDragStart, onDragOver, onDrop, endDrag, onHandleKeydown } = useDragReorder(
  () => channels.value,
  ids => account.value ? invoke('reorder_channels', { accountId: account.value.id, ids }) : Promise.resolve(),
  'Ordre non enregistré'
)
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
        description="L'application regarde les 2 premières chaînes en live de la liste. Change l'ordre en faisant glisser une ligne par sa poignée (ou Alt+flèches sur la poignée)."
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
              <th class="text-right px-3 py-2 whitespace-nowrap">Solde max</th>
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
              @dragstart="onDragStart($event, channel, channel.login)"
              @dragover="onDragOver($event, channel)"
              @drop="onDrop"
              @dragend="endDrag"
            >
              <td class="px-3 py-2">
                <button
                  type="button"
                  class="flex items-center gap-1.5 cursor-grab active:cursor-grabbing select-none rounded focus-visible:outline-2 focus-visible:outline-primary"
                  title="Glisser pour changer l'ordre (ou Alt+flèche haut/bas)"
                  :aria-label="`Position ${i + 1} : ${channel.login}. Alt+flèche haut ou bas pour déplacer`"
                  @mousedown="grabbed = channel.id"
                  @mouseup="grabbed = null"
                  @keydown="onHandleKeydown($event, channel)"
                >
                  <UIcon name="i-lucide-grip-vertical" class="size-4 text-muted" />
                  <span class="w-5 text-muted text-left">{{ i + 1 }}</span>
                </button>
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
              <td class="px-3 py-2 text-right whitespace-nowrap">
                <span v-if="channel.maxBalance" :class="channel.state === 'balanceReached' ? 'text-info' : ''">{{ formatNumber(channel.maxBalance) }}</span>
                <span v-else class="text-muted">—</span>
              </td>
              <td class="px-3 py-2 text-right" :class="channel.earnedToday > 0 ? 'text-success' : 'text-muted'">
                {{ channel.earnedToday > 0 ? '+' : '' }}{{ formatNumber(channel.earnedToday) }}
              </td>
              <td class="px-3 py-2 text-center">
                <div class="flex justify-center">
                  <USwitch :model-value="optionsOf(channel).enabled" :aria-label="`Récolter sur ${channel.login}`" @update:model-value="v => setOptions(channel, { enabled: v })" />
                </div>
              </td>
              <td class="px-3 py-2 text-center">
                <div class="flex justify-center">
                  <USwitch :model-value="optionsOf(channel).autoClaim" :aria-label="`Bonus auto sur ${channel.login}`" @update:model-value="v => setOptions(channel, { autoClaim: v })" />
                </div>
              </td>
              <td class="px-3 py-2 text-right">
                <div class="flex justify-end gap-1">
                  <UButton icon="i-lucide-settings-2" size="xs" variant="ghost" color="neutral" :aria-label="`Réglages de ${channel.login}`" title="Réglages de la chaîne" @click="openSettings(channel)" />
                  <UButton icon="i-lucide-trash-2" size="xs" variant="ghost" color="error" aria-label="Retirer de la liste" @click="toRemove = channel" />
                </div>
              </td>
            </tr>
            <tr v-if="channels.length === 0">
              <td colspan="9" class="px-3 py-8 text-center text-muted">Aucune chaîne. Ajoute la première avec le champ ci-dessus.</td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>

    <PointsChannelSettings v-model:open="settingsOpen" :channel="settingsChannel" />

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
          <UButton label="Retirer" color="error" :loading="removing" @click="confirmRemove" />
        </div>
      </template>
    </UModal>
  </div>
</template>
