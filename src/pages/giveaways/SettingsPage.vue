<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../../composables/useSnapshot'
import { SOUND_ALERTS, playSoundAlert } from '../../composables/useSoundAlerts'
import type { SoundAlertKind } from '../../composables/useSoundAlerts'
import type { Settings } from '../../types'

const { accounts } = useSnapshot()
const toast = useToast()

const settings = ref<Settings | null>(null)
onMounted(async () => { settings.value = await invoke<Settings>('get_settings') })
watch(settings, async (value, old) => {
  if (!value || !old) return
  try {
    await invoke('save_settings', { settings: value })
  } catch (e) {
    toast.add({ title: 'Réglage non enregistré', description: String(e), color: 'error' })
  }
}, { deep: true })

const autoAccountItems = computed(() => accounts.value.map(a => ({ label: a.login, value: a.id })))
const autoAccountsModel = computed({
  get: () => settings.value?.autoAccounts.length ? settings.value.autoAccounts : accounts.value.map(a => a.id),
  set: (ids: number[]) => {
    if (!settings.value) return
    // Tous cochés : liste vide (= tous, y compris les comptes ajoutés plus tard)
    settings.value.autoAccounts = ids.length === accounts.value.length ? [] : ids
  }
})

function soundOn(kind: SoundAlertKind) {
  return settings.value?.soundEvents.includes(kind) ?? false
}
function setSound(kind: SoundAlertKind, on: boolean) {
  if (!settings.value) return
  const rest = settings.value.soundEvents.filter(k => k !== kind)
  settings.value.soundEvents = on ? [...rest, kind] : rest
}

const confirmAuto = ref(false)
function setAuto(value: boolean) {
  if (!settings.value) return
  if (value) confirmAuto.value = true
  else settings.value.autoParticipate = false
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5 max-w-3xl">
    <h1 class="text-xl font-semibold">Réglages des giveaways</h1>

    <div v-if="settings" class="border border-default rounded-lg divide-y divide-default">
      <div class="flex items-center justify-between gap-4 px-4 py-3">
        <div>
          <div class="font-medium">Notifications Windows</div>
          <div class="text-sm text-muted">Giveaway détecté, mention de tes pseudos, victoire.</div>
        </div>
        <USwitch v-model="settings.notifications" aria-label="Notifications Windows" />
      </div>
      <div class="flex items-center justify-between gap-4 px-4 py-3">
        <div>
          <div class="font-medium">Alertes sonores</div>
          <div class="text-sm text-muted">Un son différent pour chaque type d'alerte, même fenêtre cachée.</div>
        </div>
        <USwitch v-model="settings.soundAlerts" aria-label="Alertes sonores" />
      </div>
      <template v-if="settings.soundAlerts">
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div class="font-medium">Volume</div>
          <div class="flex items-center gap-3 w-72">
            <UIcon name="i-lucide-volume-1" class="size-4 text-muted" />
            <USlider v-model="settings.soundVolume" :min="5" :max="100" :step="5" aria-label="Volume des alertes" class="flex-1" />
            <span class="w-10 text-right text-sm tabular-nums">{{ settings.soundVolume }} %</span>
          </div>
        </div>
        <div v-for="a in SOUND_ALERTS" :key="a.kind" class="flex items-center justify-between gap-4 px-4 py-2.5 pl-8">
          <div>
            <div class="text-sm font-medium">{{ a.label }}</div>
            <div class="text-xs text-muted">{{ a.hint }}</div>
          </div>
          <div class="flex items-center gap-2">
            <UButton icon="i-lucide-play" size="xs" color="neutral" variant="ghost" :aria-label="`Écouter : ${a.label}`" title="Écouter" @click="playSoundAlert(a.kind, settings.soundVolume)" />
            <USwitch :model-value="soundOn(a.kind)" :aria-label="`Son : ${a.label}`" @update:model-value="v => setSound(a.kind, v)" />
          </div>
        </div>
      </template>
      <div class="flex items-center justify-between gap-4 px-4 py-3">
        <div>
          <div class="font-medium">Participation automatique</div>
          <div class="text-sm text-muted">Envoie la commande toute seule, une fois par giveaway et par compte, après un délai aléatoire.</div>
        </div>
        <USwitch :model-value="settings.autoParticipate" aria-label="Participation automatique" @update:model-value="setAuto" />
      </div>
      <template v-if="settings.autoParticipate">
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div class="font-medium">Délai avant d'envoyer</div>
          <div class="flex items-center gap-2 text-sm">
            entre
            <UInputNumber id="auto-delay-min" v-model="settings.autoDelayMin" :min="3" :max="600" size="xs" class="w-24" />
            et
            <UInputNumber id="auto-delay-max" v-model="settings.autoDelayMax" :min="3" :max="600" size="xs" class="w-24" />
            secondes
          </div>
        </div>
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div class="font-medium">Comptes qui participent</div>
          <USelect v-model="autoAccountsModel" :items="autoAccountItems" multiple class="w-64" placeholder="Aucun compte" />
        </div>
      </template>
    </div>

    <UModal :open="confirmAuto" title="Activer la participation automatique ?" @update:open="v => { if (!v) confirmAuto = false }">
      <template #body>
        <div class="flex flex-col gap-2 text-sm">
          <p>L'application enverra la commande détectée dans le chat avec tes comptes, sans te demander.</p>
          <p>Une détection peut se tromper (commande d'un jeu, d'un sondage…), et des messages automatiques peuvent être mal vus par un streamer ou par Twitch. Ajoute les commandes à ne jamais envoyer dans « Commandes ignorées » de chaque chaîne.</p>
        </div>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="confirmAuto = false" />
          <UButton label="Activer" @click="() => { if (settings) settings.autoParticipate = true; confirmAuto = false }" />
        </div>
      </template>
    </UModal>
  </div>
</template>
