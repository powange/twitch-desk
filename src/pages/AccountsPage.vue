<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useOpenExternal } from '../utils/external'
import { useSnapshot } from '../composables/useSnapshot'
import { ACCOUNT_STATUS } from '../utils/format'
import type { AccountView, DeviceCode, LoginEvent } from '../types'

const { accounts } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()

const code = ref<DeviceCode | null>(null)
const starting = ref(false)
const remaining = ref(0)
let countdown: ReturnType<typeof setInterval> | null = null
let unlisten: UnlistenFn | null = null

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

onMounted(async () => {
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
            <th class="text-center px-3 py-2">Points de chaîne</th>
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
            <td class="px-3 py-2.5 text-center">
              <div class="flex justify-center">
                <USwitch :model-value="account.pointsEnabled" :aria-label="`Points pour ${account.login}`" @update:model-value="v => setPoints(account, v)" />
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
            <td colspan="6" class="px-3 py-8 text-center text-muted">Aucun compte. Connecte ton premier compte Twitch avec le bouton en haut.</td>
          </tr>
        </tbody>
      </table>
    </div>

    <p class="text-xs text-muted max-w-[75ch]">
      Les jetons de connexion sont rangés dans le Gestionnaire d'identification de Windows, jamais en clair dans les fichiers de l'application.
      Pour reconnecter un compte, utilise « Connecter un compte » avec le même compte Twitch.
    </p>

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
