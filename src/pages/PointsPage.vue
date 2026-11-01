<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { useChannelList } from '../composables/useChannelList'
import { useOpenExternal } from '../utils/external'
import { CHANNEL_STATE, formatNumber } from '../utils/format'
import type { ChannelView } from '../types'

const openExternal = useOpenExternal()
const { accounts, accountId, account, accountItems, channels, newLogin, adding, addChannel, setOptions, toRemove, confirmRemove } = useChannelList('points')

// L'ordre ne concerne que cette liste : c'est lui qui décide des 2 chaînes regardées
async function move(channel: ChannelView, delta: number) {
  const ids = channels.value.map(c => c.id)
  const from = ids.indexOf(channel.id)
  const to = from + delta
  if (from < 0 || to < 0 || to >= ids.length || !account.value) return
  ids.splice(from, 1)
  ids.splice(to, 0, channel.id)
  await invoke('reorder_channels', { accountId: account.value.id, ids })
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <div class="flex items-center gap-3">
      <h1 class="text-xl font-semibold flex-1">Points de chaîne</h1>
      <USelect v-if="accounts.length > 1" v-model="accountId" :items="accountItems" class="w-48" icon="i-lucide-user" />
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
        description="L'application regarde les 2 premières chaînes en live de la liste. Change l'ordre avec les flèches."
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
              <th class="text-left px-3 py-2 w-24">Ordre</th>
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
            <tr v-for="(channel, i) in channels" :key="channel.id" class="border-t border-default">
              <td class="px-3 py-2">
                <div class="flex items-center gap-0.5">
                  <span class="w-5 text-muted">{{ i + 1 }}</span>
                  <UButton icon="i-lucide-chevron-up" size="xs" variant="ghost" color="neutral" :disabled="i === 0" aria-label="Monter" @click="move(channel, -1)" />
                  <UButton icon="i-lucide-chevron-down" size="xs" variant="ghost" color="neutral" :disabled="i === channels.length - 1" aria-label="Descendre" @click="move(channel, 1)" />
                </div>
              </td>
              <td class="px-3 py-2 min-w-0">
                <button class="font-medium hover:underline" @click="openExternal(`https://www.twitch.tv/${channel.login}`)">{{ channel.login }}</button>
                <div v-if="channel.live" class="text-xs text-muted truncate max-w-[320px]">
                  {{ channel.live.gameName ?? 'Sans catégorie' }} · {{ formatNumber(channel.live.viewers) }} spectateurs
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
