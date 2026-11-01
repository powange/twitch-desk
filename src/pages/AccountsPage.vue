<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useOpenExternal } from '../utils/external'
import { useSnapshot } from '../composables/useSnapshot'
import { ACCOUNT_STATUS } from '../utils/format'
import type { AccountView, DeviceCode, LoginEvent, SeLoginEvent } from '../types'

const { accounts } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()

const code = ref<DeviceCode | null>(null)
const starting = ref(false)
const remaining = ref(0)
let countdown: ReturnType<typeof setInterval> | null = null
let unlisten: UnlistenFn | null = null
let unlistenSe: UnlistenFn | null = null

function stopCountdown() {
  if (countdown) clearInterval(countdown)
  countdown = null
}

async function connect() {
  starting.value = true
  try {
    code.value = await invoke<DeviceCode>('start_login')
    remaining.value = code.value.expires_in
    stopCountdown()
    countdown = setInterval(() => {
      remaining.value = Math.max(0, remaining.value - 1)
    }, 1000)
  } catch (e) {
    toast.add({ title: 'Connexion impossible', description: String(e), color: 'error' })
  } finally {
    starting.value = false
  }
}

async function cancel() {
  await invoke('cancel_login')
  code.value = null
  stopCountdown()
}

async function copyCode() {
  if (!code.value) return
  try {
    await navigator.clipboard.writeText(code.value.user_code)
    toast.add({ title: 'Code copié', color: 'success', icon: 'i-lucide-check' })
  } catch {
    toast.add({ title: 'Copie impossible, recopie le code à la main', color: 'warning' })
  }
}

function activationUrl() {
  return code.value?.verification_uri ?? 'https://www.twitch.tv/activate'
}

function formatRemaining(s: number) {
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

// --- Compte StreamElements ---
// Comptes dont la fenêtre de connexion StreamElements est ouverte
const seConnecting = ref<Set<number>>(new Set())

async function connectSe(account: AccountView) {
  try {
    await invoke('connect_streamelements', { accountId: account.id })
    seConnecting.value = new Set(seConnecting.value).add(account.id)
  } catch (e) {
    toast.add({ title: 'Connexion StreamElements impossible', description: String(e), color: 'error' })
  }
}

function seDone(accountId: number) {
  const next = new Set(seConnecting.value)
  next.delete(accountId)
  seConnecting.value = next
}

function seLoginToast(ev: SeLoginEvent) {
  const login = accounts.value.find(a => a.id === ev.accountId)?.login ?? ''
  if (ev.status === 'done') toast.add({ title: `StreamElements connecté pour ${login}`, color: 'success', icon: 'i-lucide-check' })
  if (ev.status === 'wrongAccount') toast.add({ title: 'Mauvais compte', description: `Tu t'es connecté à StreamElements avec ${ev.username}, pas avec ${login}. Recommence avec le bon compte Twitch.`, color: 'warning' })
  if (ev.status === 'error') toast.add({ title: 'Connexion StreamElements échouée', description: ev.message, color: 'error' })
}

async function disconnectSe(account: AccountView) {
  await invoke('disconnect_streamelements', { accountId: account.id })
}

// Secours : jeton d'accès personnel collé à la main
const tokenFor = ref<AccountView | null>(null)
const tokenValue = ref('')
const savingToken = ref(false)
async function saveToken() {
  if (!tokenFor.value) return
  savingToken.value = true
  try {
    const ev = await invoke<SeLoginEvent>('set_streamelements_token', { accountId: tokenFor.value.id, token: tokenValue.value })
    seLoginToast(ev)
    if (ev.status === 'done') {
      tokenFor.value = null
      tokenValue.value = ''
    }
  } catch (e) {
    toast.add({ title: 'Jeton non enregistré', description: String(e), color: 'error' })
  } finally {
    savingToken.value = false
  }
}

onMounted(async () => {
  unlistenSe = await listen<SeLoginEvent>('se-login', (e) => {
    seDone(e.payload.accountId)
    seLoginToast(e.payload)
  })
  unlisten = await listen<LoginEvent>('login', (e) => {
    code.value = null
    stopCountdown()
    const ev = e.payload
    if (ev.status === 'done') toast.add({ title: `${ev.login} connecté`, color: 'success', icon: 'i-lucide-check' })
    if (ev.status === 'expired') toast.add({ title: 'Le code a expiré', description: 'Relance la connexion.', color: 'warning' })
    if (ev.status === 'denied') toast.add({ title: 'Connexion refusée sur Twitch', color: 'warning' })
    if (ev.status === 'error') toast.add({ title: 'Connexion échouée', description: ev.message, color: 'error' })
  })
})

onBeforeUnmount(() => {
  unlisten?.()
  unlistenSe?.()
  stopCountdown()
})

async function setPoints(account: AccountView, enabled: boolean) {
  await invoke('set_points_enabled', { id: account.id, enabled })
}

const toRemove = ref<AccountView | null>(null)
async function confirmRemove() {
  if (!toRemove.value) return
  await invoke('remove_account', { id: toRemove.value.id })
  toRemove.value = null
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <div class="flex items-center">
      <h1 class="text-xl font-semibold flex-1">Comptes</h1>
      <UButton v-if="!code" icon="i-lucide-user-plus" label="Connecter un compte" :loading="starting" @click="connect" />
    </div>

    <div v-if="code" class="border border-dashed border-primary rounded-lg bg-primary/5 p-5 flex items-center gap-6">
      <div>
        <div class="text-sm text-muted">
          Ouvre <button class="text-primary font-medium hover:underline" @click="openExternal(activationUrl())">twitch.tv/activate</button>,
          connecte-toi avec le compte voulu et saisis :
        </div>
        <div class="font-mono text-3xl tracking-[0.2em] text-primary mt-1 select-all">{{ code.user_code }}</div>
      </div>
      <div class="flex-1 text-sm text-muted">
        Le code expire dans {{ formatRemaining(remaining) }}. Le compte apparaît ici dès que tu as validé.
      </div>
      <div class="flex flex-col gap-2">
        <UButton icon="i-lucide-external-link" label="Ouvrir Twitch" @click="openExternal(activationUrl())" />
        <UButton icon="i-lucide-copy" label="Copier le code" color="neutral" variant="soft" @click="copyCode" />
        <UButton label="Annuler" color="neutral" variant="ghost" @click="cancel" />
      </div>
    </div>

    <div class="border border-default rounded-lg overflow-hidden">
      <table class="w-full text-sm">
        <thead class="bg-elevated/60 text-xs uppercase text-muted">
          <tr>
            <th class="text-left px-3 py-2">Compte</th>
            <th class="text-left px-3 py-2">État</th>
            <th class="text-left px-3 py-2">Temps réel</th>
            <th class="text-left px-3 py-2">Chat</th>
            <th class="text-left px-3 py-2">StreamElements</th>
            <th class="text-center px-3 py-2">Points de chaîne</th>
            <th class="text-center px-3 py-2">Drops</th>
            <th class="px-3 py-2" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="account in accounts" :key="account.id" class="border-t border-default">
            <td class="px-3 py-2.5">
              <div class="flex items-center gap-2">
                <UAvatar :text="account.login.slice(0, 2).toUpperCase()" size="sm" />
                <span class="font-medium">{{ account.login }}</span>
              </div>
            </td>
            <td class="px-3 py-2.5">
              <UBadge :color="ACCOUNT_STATUS[account.status].color" variant="subtle" :label="ACCOUNT_STATUS[account.status].label" />
              <div v-if="account.statusText" class="text-xs text-muted mt-1 max-w-[320px]">{{ account.statusText }}</div>
            </td>
            <td class="px-3 py-2.5 text-muted">
              <span class="inline-flex items-center gap-1.5">
                <span class="size-2 rounded-full" :class="account.realtime ? 'bg-success' : 'bg-neutral-400'" />
                {{ account.realtime ? 'Connecté' : 'Interrogation régulière' }}
              </span>
            </td>
            <td class="px-3 py-2.5 text-muted">
              <span class="inline-flex items-center gap-1.5">
                <span class="size-2 rounded-full" :class="account.chatConnected ? 'bg-success' : 'bg-neutral-400'" />
                {{ account.chatConnected ? `${account.channels.filter(c => c.inChat).length} chaîne(s)` : 'Déconnecté' }}
              </span>
            </td>
            <td class="px-3 py-2.5">
              <div class="flex items-center gap-2">
                <template v-if="seConnecting.has(account.id)">
                  <UIcon name="i-lucide-loader-circle" class="size-4 animate-spin text-muted" />
                  <span class="text-muted">Connexion dans l'autre fenêtre…</span>
                </template>
                <template v-else-if="account.seAccount === 'connected'">
                  <UBadge color="success" variant="subtle" label="Connecté" />
                  <UButton label="Déconnecter" size="xs" color="neutral" variant="ghost" @click="disconnectSe(account)" />
                </template>
                <template v-else>
                  <UBadge v-if="account.seAccount === 'expired'" color="error" variant="subtle" label="Jeton expiré" />
                  <UButton :label="account.seAccount === 'expired' ? 'Reconnecter' : 'Connecter'" size="xs" color="neutral" variant="soft" @click="connectSe(account)" />
                  <UButton label="Coller un jeton" size="xs" color="neutral" variant="link" @click="tokenFor = account" />
                </template>
              </div>
            </td>
            <td class="px-3 py-2.5 text-center">
              <div class="flex justify-center">
                <USwitch :model-value="account.pointsEnabled" :aria-label="`Points pour ${account.login}`" @update:model-value="v => setPoints(account, v)" />
              </div>
            </td>
            <td class="px-3 py-2.5 text-center">
              <div class="flex justify-center">
                <USwitch :model-value="account.drops.enabled" :aria-label="`Drops pour ${account.login}`" @update:model-value="v => invoke('set_drops_enabled', { id: account.id, enabled: v })" />
              </div>
            </td>
            <td class="px-3 py-2.5">
              <div class="flex justify-end gap-1">
                <UButton v-if="account.status === 'expired'" label="Reconnecter" size="xs" @click="connect" />
                <UButton v-if="account.status === 'error'" label="Réessayer" size="xs" color="neutral" variant="soft" @click="invoke('restart_account', { id: account.id })" />
                <UButton icon="i-lucide-trash-2" size="xs" variant="ghost" color="error" aria-label="Retirer le compte" @click="toRemove = account" />
              </div>
            </td>
          </tr>
          <tr v-if="accounts.length === 0">
            <td colspan="8" class="px-3 py-8 text-center text-muted">Aucun compte. Connecte ton premier compte Twitch avec le bouton en haut.</td>
          </tr>
        </tbody>
      </table>
    </div>

    <p class="text-xs text-muted max-w-[75ch]">
      Les jetons de connexion sont rangés dans le Gestionnaire d'identification de Windows, jamais en clair dans les fichiers de l'application.
      Pour reconnecter un compte, utilise « Connecter un compte » avec le même compte Twitch.
      Le compte StreamElements sert à lire tes tickets exacts et à en acheter depuis l'application ; sans lui, les tickets sont lus dans les réponses du bot.
    </p>

    <UModal :open="tokenFor !== null" :title="`Jeton StreamElements de ${tokenFor?.login}`" @update:open="v => { if (!v) tokenFor = null }">
      <template #body>
        <div class="flex flex-col gap-3 text-sm">
          <p>
            Sur streamelements.com, connecté avec ce compte Twitch : paramètres du compte, onglet <b>Sécurité</b>, <b>Jeton d'accès personnel</b>.
            Il équivaut à ton mot de passe StreamElements : l'application le range dans le Gestionnaire d'identification de Windows.
          </p>
          <UTextarea id="se-token" v-model="tokenValue" :rows="4" placeholder="Colle le jeton ici" class="font-mono" autoresize />
        </div>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="tokenFor = null" />
          <UButton label="Enregistrer" :loading="savingToken" @click="saveToken" />
        </div>
      </template>
    </UModal>

    <UModal :open="toRemove !== null" title="Retirer le compte ?" @update:open="v => { if (!v) toRemove = null }">
      <template #body>
        <p>{{ toRemove?.login }} sera déconnecté, et ses chaînes suivies seront supprimées de l'application.</p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="toRemove = null" />
          <UButton label="Retirer" color="error" @click="confirmRemove" />
        </div>
      </template>
    </UModal>
  </div>
</template>
