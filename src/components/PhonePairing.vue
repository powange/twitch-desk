<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import QRCode from 'qrcode'
import { timeAgo } from '../utils/format'

// Web app mobile (serveur 0.6 et plus) : nom de cette application, téléphones associés par
// un code à 6 chiffres, révocation
const props = defineProps<{ serverUrl: string }>()
const toast = useToast()

interface Device { id: string, name: string, createdAt: string, lastSeen: string | null }
interface Pairing { code: string, expiresAt: string, link: string }

const name = ref('')
const defaultName = ref('')
const webUrl = ref('')
const savedName = ref('')
const savedWebUrl = ref('')
const savingInstance = ref(false)

const devices = ref<Device[]>([])
const devicesError = ref<string | null>(null)
const loadingDevices = ref(false)

const pairing = ref<Pairing | null>(null)
const pairOpen = ref(false)
const pairingBusy = ref(false)
const qr = ref('')
const now = ref(Date.now())
let ticker: ReturnType<typeof setInterval> | null = null
let poller: ReturnType<typeof setInterval> | null = null

const webPlaceholder = computed(() => `${props.serverUrl.replace(/\/+$/, '') || 'https://sync.exemple.fr'}/app`)
const instanceChanged = computed(() => name.value.trim() !== savedName.value || webUrl.value.trim() !== savedWebUrl.value)
const remaining = computed(() => {
  if (!pairing.value) return 0
  return Math.max(0, Math.round((new Date(pairing.value.expiresAt).getTime() - now.value) / 1000))
})
const remainingLabel = computed(() => `${Math.floor(remaining.value / 60)}:${String(remaining.value % 60).padStart(2, '0')}`)

onMounted(async () => {
  const config = await invoke<{ instanceName: string, defaultName: string, webUrl: string }>('get_sync_config')
  name.value = config.instanceName
  savedName.value = config.instanceName
  defaultName.value = config.defaultName
  webUrl.value = config.webUrl
  savedWebUrl.value = config.webUrl
  await loadDevices()
})

onBeforeUnmount(stopTimers)

async function saveInstance() {
  savingInstance.value = true
  try {
    await invoke('save_sync_instance', { name: name.value, webUrl: webUrl.value })
    savedName.value = name.value.trim()
    savedWebUrl.value = webUrl.value.trim()
    toast.add({ title: 'Enregistré', color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Non enregistré', description: String(e), color: 'error' })
  } finally {
    savingInstance.value = false
  }
}

async function loadDevices(quiet = false) {
  if (!quiet) loadingDevices.value = true
  try {
    devices.value = await invoke<Device[]>('sync_devices')
    devicesError.value = null
  } catch (e) {
    devicesError.value = String(e)
  } finally {
    loadingDevices.value = false
  }
}

async function startPairing() {
  pairingBusy.value = true
  try {
    const p = await invoke<Pairing>('sync_pair_start')
    pairing.value = p
    qr.value = await QRCode.toDataURL(p.link, { margin: 1, width: 240, errorCorrectionLevel: 'M' })
    pairOpen.value = true
    startTimers()
  } catch (e) {
    toast.add({ title: 'Impossible de connecter un téléphone', description: String(e), color: 'error' })
  } finally {
    pairingBusy.value = false
  }
}

// Pendant l'affichage du code : compte à rebours, et liste relue pour voir arriver le téléphone
function startTimers() {
  stopTimers()
  const known = new Set(devices.value.map(d => d.id))
  now.value = Date.now()
  ticker = setInterval(() => { now.value = Date.now() }, 1000)
  poller = setInterval(async () => {
    await loadDevices(true)
    const added = devices.value.find(d => !known.has(d.id))
    if (added) {
      pairOpen.value = false
      toast.add({ title: 'Téléphone connecté', description: added.name, color: 'success', icon: 'i-lucide-smartphone' })
    }
  }, 3000)
}

function stopTimers() {
  if (ticker) clearInterval(ticker)
  if (poller) clearInterval(poller)
  ticker = null
  poller = null
}

watch(pairOpen, (open) => {
  if (!open) {
    stopTimers()
    pairing.value = null
    void loadDevices(true)
  }
})

const revokeTarget = ref<Device | null>(null)
async function revoke() {
  const device = revokeTarget.value
  revokeTarget.value = null
  if (!device) return
  try {
    await invoke('sync_revoke_device', { deviceId: device.id })
    devices.value = devices.value.filter(d => d.id !== device.id)
    toast.add({ title: 'Téléphone déconnecté', description: device.name, color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Révocation impossible', description: String(e), color: 'error' })
  }
}

async function copyLink() {
  if (!pairing.value) return
  try {
    await navigator.clipboard.writeText(pairing.value.link)
    toast.add({ title: 'Lien copié', color: 'success', icon: 'i-lucide-check' })
  } catch {
    toast.add({ title: 'Copie impossible', color: 'error' })
  }
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <div>
      <div class="text-sm font-medium">Téléphone</div>
      <div class="text-xs text-muted">Sur ton téléphone, la web app montre les giveaways en cours, la commande à copier et les participations de cette application. Elle ne modifie rien.</div>
    </div>
    <UFormField label="Nom de cette application" help="Affiché sur le téléphone pour reconnaître ce poste">
      <UInput id="sync-instance-name" v-model="name" :placeholder="defaultName" class="w-full" maxlength="60" autocomplete="off" />
    </UFormField>
    <UFormField label="Adresse de la web app" help="À changer seulement si ton proxy sert /app ailleurs que sur l'adresse du serveur">
      <UInput id="sync-web-url" v-model="webUrl" :placeholder="webPlaceholder" class="w-full" autocomplete="off" />
    </UFormField>
    <div class="flex items-center gap-2">
      <UButton v-if="instanceChanged" label="Enregistrer" size="sm" :loading="savingInstance" @click="saveInstance" />
      <UButton label="Connecter un téléphone" icon="i-lucide-smartphone" size="sm" :loading="pairingBusy" @click="startPairing" />
    </div>

    <div class="flex flex-col gap-1">
      <div class="flex items-center gap-2">
        <div class="text-sm font-medium">Téléphones connectés</div>
        <UButton icon="i-lucide-refresh-cw" size="xs" color="neutral" variant="ghost" aria-label="Relire la liste" :loading="loadingDevices" @click="loadDevices()" />
      </div>
      <p v-if="devicesError" class="text-sm text-muted">{{ devicesError }}</p>
      <p v-else-if="!devices.length" class="text-sm text-muted">Aucun téléphone connecté.</p>
      <table v-else class="text-sm w-full max-w-xl">
        <thead class="text-xs uppercase text-muted">
          <tr>
            <th class="text-left py-1 font-medium">Appareil</th>
            <th class="text-left py-1 font-medium">Connecté</th>
            <th class="text-left py-1 font-medium">Dernière visite</th>
            <th class="py-1" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="d in devices" :key="d.id" class="border-t border-default">
            <td class="py-1.5">{{ d.name }}</td>
            <td class="py-1.5 text-muted">{{ timeAgo(d.createdAt) }}</td>
            <td class="py-1.5 text-muted">{{ d.lastSeen ? timeAgo(d.lastSeen) : '—' }}</td>
            <td class="py-1.5 text-right">
              <UButton label="Déconnecter" icon="i-lucide-unplug" size="xs" color="error" variant="soft" @click="revokeTarget = d" />
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <UModal v-model:open="pairOpen" title="Connecter un téléphone">
      <template #body>
        <div v-if="pairing" class="flex flex-col items-center gap-3 text-center">
          <p class="text-sm">Scanne ce QR code avec l'appareil photo du téléphone, ou ouvre la web app et tape le code.</p>
          <img :src="qr" alt="QR code de connexion" width="240" height="240" class="rounded bg-white">
          <div class="font-mono text-3xl tracking-[0.3em] tabular-nums" aria-label="Code de connexion">{{ pairing.code }}</div>
          <div v-if="remaining > 0" class="text-sm text-muted">Valable encore {{ remainingLabel }} · une seule fois</div>
          <div v-else class="text-sm text-warning">Code expiré</div>
          <div class="flex items-center gap-2 max-w-full">
            <code class="text-xs text-muted truncate">{{ pairing.link }}</code>
            <UButton icon="i-lucide-copy" size="xs" color="neutral" variant="ghost" aria-label="Copier le lien" @click="copyLink" />
          </div>
        </div>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Nouveau code" icon="i-lucide-refresh-cw" color="neutral" variant="soft" :loading="pairingBusy" @click="startPairing" />
          <UButton label="Fermer" color="neutral" variant="ghost" @click="pairOpen = false" />
        </div>
      </template>
    </UModal>

    <UModal :open="revokeTarget !== null" title="Déconnecter ce téléphone ?" @update:open="v => { if (!v) revokeTarget = null }">
      <template #body>
        <p class="text-sm">{{ revokeTarget?.name }} n'aura plus accès à la web app. Pour le reconnecter, il faudra un nouveau code.</p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="revokeTarget = null" />
          <UButton label="Déconnecter" color="error" @click="revoke" />
        </div>
      </template>
    </UModal>
  </div>
</template>
