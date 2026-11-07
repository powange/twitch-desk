<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { ApiError, pcAction } from '../api'
import { usePhone } from '../phone'
import type { HelloDraftData, PhoneAction } from '../types'

// Bonjour à confirmer, comme sur le PC : une phrase tirée au hasard pour chaque compte qui ne l'a
// pas encore dit, modifiable ou à retirer au dé, puis envoyée par le PC
const props = defineProps<{ item: string | null, draft: HelloDraftData | null }>()
const open = defineModel<boolean>('open', { default: false })
const phone = usePhone()
const toast = useToast()

interface Draft { login: string, message: string, enabled: boolean }
const drafts = ref<Draft[]>([])
const sending = ref(false)

// Phrase au hasard, différente de celles déjà prises tant qu'il en reste
function randomPhrase(phrases: string[], taken: string[] = []) {
  const used = new Set(taken.map(t => t.trim()))
  const free = phrases.filter(p => !used.has(p))
  const pool = free.length ? free : phrases
  return pool[Math.floor(Math.random() * pool.length)] ?? ''
}
watch(() => [open.value, props.draft] as const, ([isOpen, d]) => {
  if (!isOpen || !d) return
  sending.value = false
  drafts.value = d.accounts.filter(a => !a.greeted).reduce<Draft[]>((list, a) => [...list, { login: a.login, enabled: true, message: randomPhrase(d.phrases, list.map(x => x.message)) }], [])
})
function reroll(login: string) {
  const d = drafts.value.find(x => x.login === login)
  if (d && props.draft) d.message = randomPhrase(props.draft.phrases, drafts.value.map(x => x.message))
}
const chosen = computed(() => drafts.value.filter(d => d.enabled && d.message.trim()))
// Deux comptes cochés avec le même message (Twitch et les modérateurs le repèrent)
const duplicate = computed(() => {
  const seen = new Set<string>()
  return chosen.value.some((d) => {
    const m = d.message.trim().toLowerCase()
    if (seen.has(m)) return true
    seen.add(m)
    return false
  })
})
const canSend = computed(() => chosen.value.length > 0 && !duplicate.value)

async function send() {
  const token = phone.token.value
  if (!token || !props.item || !canSend.value) return
  sending.value = true
  try {
    const messages = chosen.value.map(d => ({ account: d.login, message: d.message.trim() }))
    phone.requested(await pcAction<PhoneAction>(token, { kind: 'giveaway_hello', item: props.item, messages }))
    open.value = false
  } catch (e) {
    toast.add({ title: 'Demande non envoyée', description: e instanceof ApiError && e.status === 400 ? 'Mets à jour le serveur de synchronisation (version 0.8.16 ou plus).' : String(e), color: 'error' })
  } finally {
    sending.value = false
  }
}
</script>

<template>
  <UModal
    v-model:open="open"
    :title="draft ? `Dire bonjour sur ${draft.channel}` : 'Dire bonjour'"
    description="Coche les comptes qui écrivent. Chacun a sa propre phrase : modifie-la ou tires-en une autre avant l'envoi."
  >
    <template #body>
      <div class="flex flex-col gap-3">
        <div v-for="d in drafts" :key="d.login" class="flex flex-col gap-1">
          <UCheckbox v-model="d.enabled" :label="d.login" />
          <div class="flex items-center gap-2" :class="d.enabled ? '' : 'opacity-50'">
            <UInput v-model="d.message" class="flex-1 min-w-0" maxlength="400" :disabled="!d.enabled" placeholder="Message de bonjour" @keydown.enter.prevent="send" />
            <UButton
              icon="i-lucide-dices"
              color="neutral"
              variant="ghost"
              :disabled="!d.enabled || (draft?.phrases.length ?? 0) < 2"
              :aria-label="`Autre phrase pour ${d.login}`"
              @click="reroll(d.login)"
            />
          </div>
        </div>
        <p v-if="duplicate" class="text-sm text-error">Deux comptes ont le même message : change l'un des deux.</p>
        <p v-else-if="chosen.length > 1" class="text-xs text-muted">Les comptes écrivent à quelques secondes d'intervalle.</p>
      </div>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton label="Annuler" color="neutral" variant="ghost" @click="open = false" />
        <UButton :label="chosen.length > 1 ? `Envoyer (${chosen.length} comptes)` : 'Envoyer'" icon="i-lucide-send" :loading="sending" :disabled="!canSend" @click="send" />
      </div>
    </template>
  </UModal>
</template>
