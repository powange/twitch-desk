<script setup lang="ts">
import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import IgnoredCommandsEditor from './IgnoredCommandsEditor.vue'
import type { GiveawayChannelView } from '../types'

// Réglages d'une chaîne surveillée. Les commandes ignorées sont enregistrées sur la fiche du
// streamer : on les retrouve dans le formulaire de giveaway, et inversement
const props = defineProps<{ channel: GiveawayChannelView | null }>()
const open = defineModel<boolean>('open', { required: true })
const toast = useToast()

const ignored = ref<string[]>([])
watch(open, (isOpen) => {
  if (isOpen) ignored.value = [...(props.channel?.excluded ?? [])]
})

async function save(list: string[]) {
  if (!props.channel) return
  try {
    ignored.value = await invoke<string[]>('set_ignored_commands', { login: props.channel.login, commands: list })
  } catch (e) {
    toast.add({ title: 'Commandes non enregistrées', description: String(e), color: 'error' })
  }
}
</script>

<template>
  <UModal v-model:open="open" :title="`Réglages de ${channel?.login ?? ''}`">
    <template #body>
      <UFormField
        label="Commandes ignorées"
        help="Commandes de son chat qui ne déclenchent jamais d'alerte de giveaway (!points, !discord…). Gardées sur la fiche du streamer."
      >
        <IgnoredCommandsEditor id="monitored-ignored" :model-value="ignored" @update:model-value="save" />
      </UFormField>
    </template>
    <template #footer>
      <div class="flex justify-end w-full">
        <UButton label="Fermer" color="neutral" variant="soft" @click="open = false" />
      </div>
    </template>
  </UModal>
</template>
