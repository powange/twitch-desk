<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../../composables/useSnapshot'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { CUSTOM_SOUND, DEFAULT_SOUND, SOUND_ALERTS, SOUND_PRESETS, forgetCustomSound, playSoundAlert } from '../../composables/useSoundAlerts'
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

// Son de chaque alerte : sons prédéfinis, ou fichier de l'ordinateur
function soundChoice(kind: SoundAlertKind) {
  return settings.value?.soundChoices[kind] ?? DEFAULT_SOUND[kind]
}
function soundItems(kind: SoundAlertKind) {
  const file = settings.value?.soundFiles[kind]
  return [
    ...Object.entries(SOUND_PRESETS).map(([value, p]) => ({ label: value === DEFAULT_SOUND[kind] ? `${p.label} (par défaut)` : p.label, value })),
    { label: file ? `Mon fichier : ${file}` : 'Mon fichier…', value: CUSTOM_SOUND }
  ]
}
async function chooseFile(kind: SoundAlertKind): Promise<boolean> {
  const path = await openDialog({ multiple: false, filters: [{ name: 'Son', extensions: ['wav', 'mp3', 'ogg'] }] })
  if (!path || Array.isArray(path) || !settings.value) return false
  try {
    const name = await invoke<string>('import_sound', { kind, path })
    forgetCustomSound(kind)
    settings.value.soundFiles = { ...settings.value.soundFiles, [kind]: name }
    return true
  } catch (e) {
    toast.add({ title: 'Son non utilisable', description: String(e), color: 'error' })
    return false
  }
}
async function setChoice(kind: SoundAlertKind, value: string) {
  if (!settings.value) return
  // « Mon fichier » sans fichier encore choisi : on le demande
  if (value === CUSTOM_SOUND && !settings.value.soundFiles[kind] && !(await chooseFile(kind))) return
  settings.value.soundChoices = { ...settings.value.soundChoices, [kind]: value }
  void playSoundAlert(kind, settings.value.soundVolume, value)
}

function soundOn(kind: SoundAlertKind) {
  return settings.value?.soundEvents.includes(kind) ?? false
}
function setSound(kind: SoundAlertKind, on: boolean) {
  if (!settings.value) return
  const rest = settings.value.soundEvents.filter(k => k !== kind)
  settings.value.soundEvents = on ? [...rest, kind] : rest
}

// Détection : seuil en messages (même calcul que l'application), délai lisible
const detectCount = computed(() => {
  const s = settings.value
  return s ? Math.max(2, Math.ceil(s.detectWindow * s.detectPercent / 100)) : 0
})
function durationLabel(secs: number) {
  if (secs < 60) return `${secs} s`
  const m = Math.floor(secs / 60)
  const rest = secs % 60
  return rest ? `${m} min ${rest} s` : `${m} min`
}
function resetDetection() {
  if (!settings.value) return
  Object.assign(settings.value, { detectWindow: 20, detectPercent: 35, detectMaxPerUser: 2, detectCooldownSecs: 600, detectKeywords: true })
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
            <USelect
              :model-value="soundChoice(a.kind)"
              :items="soundItems(a.kind)"
              size="xs"
              class="w-52"
              :aria-label="`Son : ${a.label}`"
              @update:model-value="v => setChoice(a.kind, String(v))"
            />
            <UButton
              v-if="soundChoice(a.kind) === CUSTOM_SOUND"
              icon="i-lucide-folder-open"
              size="xs"
              color="neutral"
              variant="ghost"
              :aria-label="`Changer le fichier : ${a.label}`"
              title="Choisir un autre fichier"
              @click="chooseFile(a.kind)"
            />
            <UButton icon="i-lucide-play" size="xs" color="neutral" variant="ghost" :aria-label="`Écouter : ${a.label}`" title="Écouter" @click="playSoundAlert(a.kind, settings.soundVolume, soundChoice(a.kind))" />
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

    <div v-if="settings" class="flex flex-col gap-2">
      <div class="flex items-end gap-3">
        <div class="flex-1">
          <h2 class="font-semibold">Détection des giveaways</h2>
          <p class="text-sm text-muted">Plus le seuil est bas, plus la détection réagit tôt (et se trompe plus souvent).</p>
        </div>
        <UButton label="Valeurs par défaut" size="xs" color="neutral" variant="ghost" @click="resetDetection" />
      </div>
      <div class="border border-default rounded-lg divide-y divide-default">
        <div class="flex flex-col gap-2 px-4 py-3">
          <div class="font-medium">Seuil de l'alerte : {{ settings.detectPercent }} % ({{ detectCount }}+ messages)</div>
          <USlider v-model="settings.detectPercent" :min="10" :max="80" :step="5" aria-label="Seuil de l'alerte" />
          <div class="text-sm text-muted">Alerte quand ce pourcentage des derniers messages du chat utilise une même commande.</div>
        </div>
        <div class="flex flex-col gap-2 px-4 py-3">
          <div class="font-medium">Messages analysés : {{ settings.detectWindow }}</div>
          <USlider v-model="settings.detectWindow" :min="10" :max="60" :step="5" aria-label="Messages analysés" />
          <div class="text-sm text-muted">Nombre de derniers messages du chat pris en compte. Plus il est grand, plus la détection est stable sur les chats rapides.</div>
        </div>
        <div class="flex flex-col gap-2 px-4 py-3">
          <div class="font-medium">Délai entre deux alertes : {{ durationLabel(settings.detectCooldownSecs) }}</div>
          <USlider v-model="settings.detectCooldownSecs" :min="10" :max="1800" :step="10" aria-label="Délai entre deux alertes" />
          <div class="text-sm text-muted">Délai minimum avant une nouvelle alerte pour la même commande sur un même stream.</div>
        </div>
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div>
            <div class="font-medium">Détecter aussi les mots-clés sans « ! »</div>
            <div class="text-sm text-muted">Un même mot tapé seul par le chat (« bonbons ») compte comme une commande. Les mots courants (gg, lol, salut…) sont ignorés.</div>
          </div>
          <USwitch v-model="settings.detectKeywords" aria-label="Détecter les mots-clés sans !" />
        </div>
        <div class="flex flex-col gap-2 px-4 py-3">
          <div class="font-medium">Messages comptés par spectateur : {{ settings.detectMaxPerUser }}</div>
          <USlider v-model="settings.detectMaxPerUser" :min="1" :max="5" :step="1" aria-label="Messages comptés par spectateur" />
          <div class="text-sm text-muted">Un même spectateur ne compte pas plus de fois : un spammeur seul ne déclenche rien.</div>
        </div>
      </div>
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
