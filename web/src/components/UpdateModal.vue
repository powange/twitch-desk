<script setup lang="ts">
import { computed, ref } from 'vue'
import { ApiError, updatePc } from '../api'
import type { InstanceState, PhoneAction } from '../types'
import { timeAgo } from '../format'

// Mise à jour de l'application du PC : vérification puis installation, faites par le PC
const props = defineProps<{ state: InstanceState | null, token: string, pending: PhoneAction[] }>()
const open = defineModel<boolean>('open', { required: true })
const emit = defineEmits<{ requested: [PhoneAction] }>()
const toast = useToast()

const pc = computed(() => props.state?.pc ?? null)
// Version plus récente que celle installée (vérification antérieure à une mise à jour : ignorée)
const available = computed(() => {
  const v = pc.value?.update?.available
  return v && v !== pc.value?.version ? v : null
})
const checking = computed(() => props.pending.some(a => a.kind === 'update_check'))
const installing = computed(() => props.pending.some(a => a.kind === 'update_install'))
const sending = ref(false)

async function ask(kind: 'update_check' | 'update_install') {
  sending.value = true
  try {
    emit('requested', await updatePc<PhoneAction>(props.token, kind))
  } catch (e) {
    const message = e instanceof ApiError && e.status === 400 && /action inconnue/i.test(e.message) ? 'Mets à jour le serveur (version 0.8.2 ou plus) pour mettre à jour le PC depuis le téléphone.' : String(e instanceof Error ? e.message : e)
    toast.add({ title: 'Demande impossible', description: message, color: 'error' })
  } finally {
    sending.value = false
  }
}
</script>

<template>
  <UModal v-model:open="open" title="Mise à jour du PC">
    <template #body>
      <div class="flex flex-col gap-3 text-sm">
        <p v-if="!pc" class="text-muted">Mets à jour une première fois l'application du PC à la main (version 0.43 ou plus) : ensuite, tu pourras le faire d'ici.</p>
        <template v-else>
          <p>Version installée sur le PC : <b>{{ pc.version }}</b></p>
          <div v-if="available" class="rounded-lg border border-primary/40 bg-primary/10 p-3 flex flex-col gap-1">
            <p class="font-medium">Version {{ available }} disponible</p>
            <p class="text-muted">Le PC la télécharge, l'installe, puis l'application redémarre seule (environ une minute).</p>
          </div>
          <p v-else-if="pc.update" class="text-muted">À jour · vérifié {{ timeAgo(pc.update.checkedAt) }}</p>
          <p v-if="installing" class="text-muted">Installation demandée au PC…</p>
          <p class="text-xs text-muted">Le PC doit être allumé, avec l'application ouverte.</p>
        </template>
      </div>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton label="Fermer" color="neutral" variant="ghost" @click="open = false" />
        <UButton
          v-if="pc"
          label="Vérifier"
          icon="i-lucide-refresh-cw"
          :color="available ? 'neutral' : 'primary'"
          :variant="available ? 'soft' : 'solid'"
          :loading="checking"
          :disabled="sending || checking || installing"
          @click="ask('update_check')"
        />
        <UButton
          v-if="available"
          label="Installer"
          icon="i-lucide-download"
          :loading="installing"
          :disabled="sending || checking || installing"
          @click="ask('update_install')"
        />
      </div>
    </template>
  </UModal>
</template>
