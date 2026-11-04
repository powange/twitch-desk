<script setup lang="ts">
import { helloRequest, randomPhrase, sendHello } from '../composables/useConditionActions'

const toast = useToast()
const canSend = () => helloRequest.drafts.some(d => d.message.trim())
</script>

<template>
  <UModal
    v-model:open="helloRequest.open"
    :title="`Dire bonjour sur ${helloRequest.channel}`"
    description="Message écrit dans le chat par chaque compte qui ne l'a pas encore dit. Modifie-le ou tires-en un autre avant l'envoi."
  >
    <template #body>
      <div class="flex flex-col gap-3">
        <UFormField v-for="d in helloRequest.drafts" :key="d.accountId" :label="d.login">
          <div class="flex items-center gap-2">
            <UInput
              :id="`hello-${d.accountId}`"
              v-model="d.message"
              class="flex-1"
              maxlength="400"
              placeholder="Vide : ce compte n'écrit rien"
              @keydown.enter.prevent="canSend() && sendHello(toast)"
            />
            <UButton
              icon="i-lucide-dices"
              color="neutral"
              variant="ghost"
              :disabled="helloRequest.phrases.length < 2"
              :aria-label="`Autre phrase pour ${d.login}`"
              title="Autre phrase au hasard"
              @click="d.message = randomPhrase(helloRequest.phrases, d.message)"
            />
          </div>
        </UFormField>
        <p v-if="helloRequest.drafts.length > 1" class="text-xs text-muted">Les comptes écrivent à quelques secondes d'intervalle.</p>
      </div>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton label="Annuler" color="neutral" variant="ghost" @click="helloRequest.open = false" />
        <UButton label="Envoyer" icon="i-lucide-send" :loading="helloRequest.sending" :disabled="!canSend()" @click="sendHello(toast)" />
      </div>
    </template>
  </UModal>
</template>
