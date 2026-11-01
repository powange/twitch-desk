<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { useSnapshot } from '../composables/useSnapshot'
import { CHANNEL_STATE, formatNumber } from '../utils/format'
import type { ChannelView } from '../types'

const { accounts } = useSnapshot()
const toast = useToast()

const accountId = ref<number | undefined>()
watch(accounts, (list) => {
  if (!list.some(a => a.id === accountId.value)) accountId.value = list[0]?.id
}, { immediate: true })

const account = computed(() => accounts.value.find(a => a.id === accountId.value))
const accountItems = computed(() => accounts.value.map(a => ({ label: a.login, value: a.id })))

const newLogin = ref('')
const adding = ref(false)

async function addChannel() {
  if (!account.value || !newLogin.value.trim()) return
  adding.value = true
  try {
    const login = await invoke<string>('add_channel', { accountId: account.value.id, login: newLogin.value })
    newLogin.value = ''
    toast.add({ title: `${login} ajoutée`, color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Chaîne non ajoutée', description: String(e), color: 'error' })
  } finally {
    adding.value = false
  }
}

async function move(channel: ChannelView, delta: number) {
  const list = account.value?.channels ?? []
  const ids = list.map(c => c.id)
  const from = ids.indexOf(channel.id)
  const to = from + delta
  if (from < 0 || to < 0 || to >= ids.length) return
  ids.splice(from, 1)
  ids.splice(to, 0, channel.id)
  await invoke('reorder_channels', { accountId: account.value!.id, ids })
}

async function setOptions(channel: ChannelView, patch: Partial<Pick<ChannelView, 'enabled' | 'autoClaim'>>) {
  await invoke('set_channel_options', {
    id: channel.id,
    enabled: patch.enabled ?? channel.enabled,
    autoClaim: patch.autoClaim ?? channel.autoClaim
  })
}

const toRemove = ref<ChannelView | null>(null)
async function confirmRemove() {
  if (!toRemove.value) return
  await invoke('remove_channel', { id: toRemove.value.id })
  toRemove.value = null
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
        title="Points désactivés pour ce compte"
        description="Active-les dans Comptes."
      />

      <form class="flex gap-2" @submit.prevent="addChannel">
        <UInput id="new-channel" v-model="newLogin" placeholder="Nom ou adresse de la chaîne (ex. lunarsea)" class="flex-1" icon="i-lucide-plus" />
        <UButton type="submit" label="Ajouter" :loading="adding" />
      </form>

      <div class="border border-default rounded-lg overflow-hidden">
        <table class="w-full text-sm">
          <thead class="bg-elevated/60 text-xs uppercase text-muted">
            <tr>
              <th class="text-left px-3 py-2 w-20">Ordre</th>
              <th class="text-left px-3 py-2">Chaîne</th>
              <th class="text-left px-3 py-2">État</th>
              <th class="text-right px-3 py-2">Solde</th>
              <th class="text-right px-3 py-2">Aujourd'hui</th>
              <th class="text-center px-3 py-2">Active</th>
              <th class="text-center px-3 py-2">Bonus auto</th>
              <th class="px-3 py-2" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="(channel, i) in account.channels" :key="channel.id" class="border-t border-default">
              <td class="px-3 py-2">
                <div class="flex items-center gap-0.5">
                  <span class="w-5 text-muted">{{ i + 1 }}</span>
                  <UButton icon="i-lucide-chevron-up" size="xs" variant="ghost" color="neutral" :disabled="i === 0" aria-label="Monter" @click="move(channel, -1)" />
                  <UButton icon="i-lucide-chevron-down" size="xs" variant="ghost" color="neutral" :disabled="i === account.channels.length - 1" aria-label="Descendre" @click="move(channel, 1)" />
                </div>
              </td>
              <td class="px-3 py-2 min-w-0">
                <button class="font-medium hover:underline" @click="openUrl(`https://www.twitch.tv/${channel.login}`)">{{ channel.login }}</button>
                <div v-if="channel.live" class="text-xs text-muted truncate max-w-[340px]">
                  {{ channel.live.gameName ?? 'Sans catégorie' }} · {{ formatNumber(channel.live.viewers) }} spectateurs
                </div>
                <div v-if="channel.error" class="text-xs text-error truncate max-w-[340px]" :title="channel.error">{{ channel.error }}</div>
              </td>
              <td class="px-3 py-2">
                <UBadge :color="CHANNEL_STATE[channel.state].color" variant="subtle" :label="CHANNEL_STATE[channel.state].label" />
              </td>
              <td class="px-3 py-2 text-right">{{ formatNumber(channel.balance) }}</td>
              <td class="px-3 py-2 text-right" :class="channel.earnedToday > 0 ? 'text-success' : 'text-muted'">
                {{ channel.earnedToday > 0 ? '+' : '' }}{{ formatNumber(channel.earnedToday) }}
              </td>
              <td class="px-3 py-2 text-center">
                <USwitch :model-value="channel.enabled" :aria-label="`Récolter sur ${channel.login}`" @update:model-value="v => setOptions(channel, { enabled: v })" />
              </td>
              <td class="px-3 py-2 text-center">
                <USwitch :model-value="channel.autoClaim" :aria-label="`Bonus auto sur ${channel.login}`" @update:model-value="v => setOptions(channel, { autoClaim: v })" />
              </td>
              <td class="px-3 py-2 text-right">
                <UButton icon="i-lucide-trash-2" size="xs" variant="ghost" color="error" aria-label="Retirer" @click="toRemove = channel" />
              </td>
            </tr>
            <tr v-if="account.channels.length === 0">
              <td colspan="8" class="px-3 py-8 text-center text-muted">Aucune chaîne. Ajoute la première avec le champ ci-dessus.</td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>

    <UModal :open="toRemove !== null" title="Retirer la chaîne ?" @update:open="v => { if (!v) toRemove = null }">
      <template #body>
        <p>{{ toRemove?.login }} ne sera plus suivie par {{ account?.login }}. L'historique des gains est conservé.</p>
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
