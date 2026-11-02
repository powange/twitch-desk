<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { ALERTS, active, choose, disable, enable, info, serverTooOld, support } from '../push'

const props = defineProps<{ token: string }>()
const open = defineModel<boolean>('open', { default: false })
const toast = useToast()

const state = ref(support())
const busy = ref(false)
const error = ref<string | null>(null)
// Alertes cochées : celles du serveur, sinon celles par défaut
const selected = ref<string[]>([])
watch(open, (o) => {
  if (!o) return
  state.value = support()
  error.value = null
  selected.value = info.value?.events.length ? [...info.value.events] : ['giveaway', 'win', 'reminder', 'mention']
}, { immediate: true })

const help = computed(() => {
  switch (state.value) {
    case 'install': return 'Sur iPhone et iPad, les notifications ne marchent que depuis l\'écran d\'accueil : touche Partager puis « Sur l\'écran d\'accueil », et ouvre Twitch Desk depuis cette icône (iOS 16.4 ou plus).'
    case 'unsupported': return 'Ce navigateur ne reçoit pas de notifications push. Essaie Chrome ou Firefox sur Android, ou Safari sur iPhone (depuis l\'écran d\'accueil).'
    case 'denied': return 'Les notifications sont bloquées pour ce site : autorise-les dans les réglages du navigateur, puis reviens ici.'
    default: return null
  }
})

async function run(action: () => Promise<void>, done?: string) {
  busy.value = true
  error.value = null
  try {
    await action()
    if (done) toast.add({ title: done, color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
    state.value = support()
  } finally {
    busy.value = false
  }
}

function toggle(key: string, on: boolean) {
  const next = on ? [...selected.value, key] : selected.value.filter(k => k !== key)
  selected.value = ALERTS.map(a => a.key).filter(k => next.includes(k))
  if (active.value) void run(() => choose(props.token, selected.value))
}
</script>

<template>
  <UModal v-model:open="open" title="Notifications" description="Reçues sur ce téléphone, même web app fermée">
    <template #body>
      <div class="flex flex-col gap-4">
        <UAlert v-if="serverTooOld" color="warning" variant="subtle" icon="i-lucide-server" description="Le serveur de synchronisation doit être mis à jour (version 0.7 ou plus) pour envoyer des notifications." />
        <UAlert v-else-if="help" color="info" variant="subtle" icon="i-lucide-info" :description="help" />
        <UAlert v-if="error" color="error" variant="subtle" icon="i-lucide-circle-alert" :description="error" />

        <div class="flex items-center gap-3">
          <UIcon :name="active ? 'i-lucide-bell-ring' : 'i-lucide-bell-off'" class="size-5 shrink-0" :class="active ? 'text-success' : 'text-muted'" />
          <div class="flex-1 text-sm">{{ active ? 'Activées sur ce téléphone' : 'Désactivées sur ce téléphone' }}</div>
        </div>

        <fieldset class="flex flex-col gap-3" :disabled="busy">
          <legend class="text-sm font-medium mb-2">Alertes à recevoir</legend>
          <USwitch
            v-for="a in ALERTS"
            :key="a.key"
            :model-value="selected.includes(a.key)"
            :label="a.label"
            :description="a.description"
            @update:model-value="(v: boolean) => toggle(a.key, v)"
          />
        </fieldset>
        <p class="text-xs text-muted">Les alertes viennent de l'application du PC : elle doit être ouverte et connectée au serveur.</p>
      </div>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton
          v-if="active"
          label="Désactiver"
          color="neutral"
          variant="soft"
          :loading="busy"
          @click="run(() => disable(token), 'Notifications désactivées')"
        />
        <UButton
          v-else
          label="Activer les notifications"
          icon="i-lucide-bell"
          :loading="busy"
          :disabled="state !== 'ok' || serverTooOld || !selected.length"
          @click="run(() => enable(token, selected), 'Notifications activées')"
        />
      </div>
    </template>
  </UModal>
</template>
