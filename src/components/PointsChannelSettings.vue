<script setup lang="ts">
import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { formatNumber } from '../utils/format'
import type { ChannelView } from '../types'

// Réglages d'une chaîne de la liste Points de chaîne
const props = defineProps<{ channel: ChannelView | null }>()
const open = defineModel<boolean>('open', { required: true })
const toast = useToast()

const maxBalance = ref<number | null>(null)
const saving = ref(false)

watch(open, (isOpen) => {
  if (isOpen) maxBalance.value = props.channel?.maxBalance ?? null
})

async function save() {
  if (!props.channel) return
  saving.value = true
  try {
    const max = maxBalance.value && maxBalance.value > 0 ? Math.round(maxBalance.value) : null
    await invoke('set_max_balance', { id: props.channel.id, max })
    open.value = false
  } catch (e) {
    toast.add({ title: 'Réglage non enregistré', description: String(e), color: 'error' })
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <UModal v-model:open="open" :title="`Réglages de ${channel?.login ?? ''}`">
    <template #body>
      <form id="points-channel-settings" class="flex flex-col gap-4" @submit.prevent="save">
        <UFormField
          label="Solde maximum"
          help="Une fois ce solde atteint, la chaîne n'est plus regardée : sa place revient à la chaîne suivante en live dans la liste. Vide : pas de limite."
        >
          <div class="flex items-center gap-2">
            <UInputNumber
              id="points-max-balance"
              v-model="maxBalance"
              :min="1"
              :step="1000"
              :format-options="{ useGrouping: true }"
              placeholder="Pas de limite"
              class="w-48"
            />
            <UButton v-if="maxBalance" label="Pas de limite" size="sm" color="neutral" variant="ghost" @click="maxBalance = null" />
          </div>
        </UFormField>
        <p v-if="channel?.balance !== null && channel?.balance !== undefined" class="text-sm text-muted">
          Solde actuel : {{ formatNumber(channel.balance) }}
        </p>
      </form>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton label="Annuler" color="neutral" variant="ghost" @click="open = false" />
        <UButton type="submit" form="points-channel-settings" label="Enregistrer" :loading="saving" />
      </div>
    </template>
  </UModal>
</template>
