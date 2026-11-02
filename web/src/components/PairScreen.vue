<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ApiError, claim, type Device } from '../api'
import { defaultDeviceName } from '../format'

// Association de cet appareil : code à 6 chiffres affiché par l'application du PC
// (Paramètres → Synchronisation → « Connecter un téléphone »), lu dans l'adresse (#pair=…)
// quand le QR code a été scanné
const props = defineProps<{ notice?: string | null }>()
const emit = defineEmits<{ paired: [device: Device] }>()

const code = ref('')
const name = ref(defaultDeviceName())
const busy = ref(false)
const error = ref<string | null>(null)
const valid = computed(() => /^\d{6}$/.test(code.value))

onMounted(() => {
  const m = location.hash.match(/pair=(\d{6})/)
  // Le code ne reste pas dans l'adresse (historique, favoris)
  if (location.hash) history.replaceState(null, '', location.pathname + location.search)
  if (m?.[1]) {
    code.value = m[1]
    void submit()
  }
})

function onInput(value: string | number) {
  code.value = String(value).replace(/\D/g, '').slice(0, 6)
}

async function submit() {
  if (!valid.value || busy.value) return
  busy.value = true
  error.value = null
  try {
    emit('paired', await claim(code.value, name.value.trim() || defaultDeviceName()))
  } catch (e) {
    if (e instanceof ApiError && e.status === 429) error.value = e.message.replace('réessayer', 'réessaie')
    else if (e instanceof ApiError && e.status === 400) error.value = `${e.message}. Demande un nouveau code sur le PC.`
    else error.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <main class="min-h-dvh flex items-center justify-center px-4 py-8">
    <form class="w-full max-w-sm flex flex-col gap-5" @submit.prevent="submit">
      <div class="flex flex-col items-center gap-3 text-center">
        <img src="/icon.svg" alt="" width="64" height="64">
        <h1 class="text-xl font-semibold">Connecter ce téléphone</h1>
        <p class="text-sm text-muted">
          Sur le PC, dans Twitch Desk : Paramètres → Synchronisation → « Connecter un téléphone ». Scanne le QR code ou tape le code affiché.
        </p>
      </div>
      <UAlert v-if="props.notice" color="warning" variant="subtle" icon="i-lucide-unplug" :description="props.notice" />
      <UFormField label="Code à 6 chiffres">
        <UInput
          :model-value="code"
          inputmode="numeric"
          autocomplete="one-time-code"
          maxlength="6"
          placeholder="123456"
          size="xl"
          class="w-full"
          :ui="{ base: 'text-center font-mono text-2xl tracking-[0.4em]' }"
          autofocus
          @update:model-value="onInput"
        />
      </UFormField>
      <UFormField label="Nom de ce téléphone" help="Affiché sur le PC, dans la liste des téléphones connectés">
        <UInput v-model="name" maxlength="60" class="w-full" size="lg" />
      </UFormField>
      <UAlert v-if="error" color="error" variant="subtle" icon="i-lucide-circle-alert" :description="error" />
      <UButton type="submit" label="Connecter" size="xl" block :loading="busy" :disabled="!valid" />
    </form>
  </main>
</template>
