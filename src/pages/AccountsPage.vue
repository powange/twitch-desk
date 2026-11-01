<script setup lang="ts">
import { ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useOpenExternal } from '../utils/external'
import { useSnapshot } from '../composables/useSnapshot'
import { useLogin } from '../composables/useLogin'
import { useAction } from '../composables/useAction'
import { ACCOUNT_STATUS } from '../utils/format'
import type { AccountView, SeLoginEvent } from '../types'

const { accounts } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()
const { run, busy } = useAction()

// Connexion en cours (code Twitch, fenêtres StreamElements) : gardée en changeant de page
const { code, starting, remaining, seConnecting, connect, cancel: cancelLogin, connectSe, seLoginToast } = useLogin()

const cancel = () => run('cancel', 'Annulation impossible', cancelLogin)

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
const disconnectSe = (account: AccountView) =>
  run(`se-${account.id}`, 'Déconnexion StreamElements impossible', () => invoke('disconnect_streamelements', { accountId: account.id }))

// Secours : jeton d'accès personnel collé à la main
const tokenFor = ref<AccountView | null>(null)
const tokenValue = ref('')
const savingToken = ref(false)
// Fenêtre fermée (Annuler, Échap, enregistrement) : le jeton collé n'est pas gardé
watch(tokenFor, (account) => { if (!account) tokenValue.value = '' })
async function saveToken() {
  if (!tokenFor.value) return
  savingToken.value = true
  try {
    const ev = await invoke<SeLoginEvent>('set_streamelements_token', { accountId: tokenFor.value.id, token: tokenValue.value })
    seLoginToast(ev)
    if (ev.status === 'done') tokenFor.value = null
  } catch (e) {
    toast.add({ title: 'Jeton non enregistré', description: String(e), color: 'error' })
  } finally {
    savingToken.value = false
  }
}

const setPoints = (account: AccountView, enabled: boolean) =>
  run(`points-${account.id}`, 'Points de chaîne non modifiés', () => invoke('set_points_enabled', { id: account.id, enabled }))
const setDrops = (account: AccountView, enabled: boolean) =>
  run(`drops-${account.id}`, 'Drops non modifiés', () => invoke('set_drops_enabled', { id: account.id, enabled }))
const restart = (account: AccountView) =>
  run(`restart-${account.id}`, 'Redémarrage du compte impossible', () => invoke('restart_account', { id: account.id }))

const toRemove = ref<AccountView | null>(null)
async function confirmRemove() {
  const account = toRemove.value
  if (!account) return
  if (await run('remove', 'Compte non retiré', () => invoke('remove_account', { id: account.id }))) toRemove.value = null
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
        <UButton label="Annuler" color="neutral" variant="ghost" :loading="busy('cancel')" @click="cancel" />
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
                  <UButton label="Déconnecter" size="xs" color="neutral" variant="ghost" :loading="busy(`se-${account.id}`)" @click="disconnectSe(account)" />
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
                <USwitch :model-value="account.pointsEnabled" :loading="busy(`points-${account.id}`)" :aria-label="`Points pour ${account.login}`" @update:model-value="v => setPoints(account, v)" />
              </div>
            </td>
            <td class="px-3 py-2.5 text-center">
              <div class="flex justify-center">
                <USwitch :model-value="account.drops.enabled" :loading="busy(`drops-${account.id}`)" :aria-label="`Drops pour ${account.login}`" @update:model-value="v => setDrops(account, v)" />
              </div>
            </td>
            <td class="px-3 py-2.5">
              <div class="flex justify-end gap-1">
                <UButton v-if="account.status === 'expired'" label="Reconnecter" size="xs" @click="connect" />
                <UButton v-if="account.status === 'error'" label="Réessayer" size="xs" color="neutral" variant="soft" :loading="busy(`restart-${account.id}`)" @click="restart(account)" />
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
          <UButton label="Retirer" color="error" :loading="busy('remove')" @click="confirmRemove" />
        </div>
      </template>
    </UModal>
  </div>
</template>
