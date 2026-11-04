<script setup lang="ts">
import { computed } from 'vue'
import { duplicateHello, helloRequest, randomPhrase, sendHello } from '../composables/useConditionActions'

const toast = useToast()
const chosen = computed(() => helloRequest.drafts.filter(d => d.enabled && d.message.trim()))
const duplicate = computed(() => duplicateHello())
const canSend = computed(() => chosen.value.length > 0 && !duplicate.value)
// Autre phrase : différente de l'actuelle et de celles des autres comptes
function reroll(accountId: number) {
  const d = helloRequest.drafts.find(x => x.accountId === accountId)
  if (!d) return
  d.message = randomPhrase(helloRequest.phrases, helloRequest.drafts.map(x => x.message))
}
</script>

<template>
  <UModal
    v-model:open="helloRequest.open"
    :title="`Dire bonjour sur ${helloRequest.channel}`"
    description="Coche les comptes qui écrivent. Chacun a sa propre phrase : modifie-la ou tires-en une autre avant l'envoi."
  >
    <template #body>
      <div class="flex flex-col gap-3">
        <div v-for="d in helloRequest.drafts" :key="d.accountId" class="flex flex-col gap-1">
          <UCheckbox v-model="d.enabled" :label="d.login" />
          <div class="flex items-center gap-2" :class="d.enabled ? '' : 'opacity-50'">
            <UInput
              :id="`hello-${d.accountId}`"
              v-model="d.message"
              class="flex-1"
              maxlength="400"
              :disabled="!d.enabled"
              placeholder="Message de bonjour"
              @keydown.enter.prevent="canSend && sendHello(toast)"
            />
            <UButton
              icon="i-lucide-dices"
              color="neutral"
              variant="ghost"
              :disabled="!d.enabled || helloRequest.phrases.length < 2"
              :aria-label="`Autre phrase pour ${d.login}`"
              title="Autre phrase au hasard"
              @click="reroll(d.accountId)"
            />
          </div>
        </div>
        <p v-if="duplicate" class="text-sm text-error">Deux comptes ont le même message : change l'un des deux.</p>
        <p v-else-if="chosen.length > 1" class="text-xs text-muted">Les comptes écrivent à quelques secondes d'intervalle.</p>
      </div>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton label="Annuler" color="neutral" variant="ghost" @click="helloRequest.open = false" />
        <UButton
          :label="chosen.length > 1 ? `Envoyer (${chosen.length} comptes)` : 'Envoyer'"
          icon="i-lucide-send"
          :loading="helloRequest.sending"
          :disabled="!canSend"
          @click="sendHello(toast)"
        />
      </div>
    </template>
  </UModal>
</template>
