<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../composables/useSnapshot'
import { useOpenExternal } from '../utils/external'
import { timeAgo } from '../utils/format'
import type { GiveawayChannelView, GiveawayView, ParticipationResult, Settings } from '../types'

const { snapshot, accounts } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()

const view = computed(() => snapshot.value?.giveaways)
const giveaways = computed(() => view.value?.giveaways ?? [])
const channels = computed(() => view.value?.channels ?? [])
const mentions = computed(() => view.value?.mentions ?? [])
// File d'alertes : giveaways ouverts pas encore traités, et mentions
const alerts = computed(() => giveaways.value.filter(g => g.open && !g.handled))

// Rafraîchit les « il y a … »
const now = ref(Date.now())
let clock: ReturnType<typeof setInterval> | null = null
onMounted(() => { clock = setInterval(() => { now.value = Date.now() }, 30_000) })
onBeforeUnmount(() => { if (clock) clearInterval(clock) })

// --- Participation ---
// Comptes cochés par giveaway (tous par défaut)
const chosen = reactive<Record<number, number[]>>({})
function chosenFor(g: GiveawayView): number[] {
  return chosen[g.id] ?? accounts.value.map(a => a.id)
}
function toggleAccount(g: GiveawayView, id: number) {
  const list = chosenFor(g)
  chosen[g.id] = list.includes(id) ? list.filter(x => x !== id) : [...list, id]
}
function entered(g: GiveawayView, id: number) {
  return g.entriesView.some(e => e.accountId === id)
}
const participating = ref<number | null>(null)

async function participate(g: GiveawayView, ids = chosenFor(g)) {
  const todo = ids.filter(id => !entered(g, id))
  if (todo.length === 0) return
  participating.value = g.id
  try {
    const results = await invoke<ParticipationResult[]>('participate_giveaway', { giveawayId: g.id, accountIds: todo })
    const failed = results.filter(r => !r.ok)
    if (failed.length === 0) {
      toast.add({ title: `${g.command} envoyé sur ${g.channel}`, description: results.map(r => r.login).join(', '), color: 'success', icon: 'i-lucide-check' })
    } else {
      toast.add({ title: 'Participation incomplète', description: failed.map(r => `${r.login} : ${r.message}`).join(' · '), color: 'error' })
    }
  } catch (e) {
    toast.add({ title: 'Participation impossible', description: String(e), color: 'error' })
  } finally {
    participating.value = null
  }
}

function evidenceText(g: GiveawayView) {
  const e = g.evidence
  if (!e) return ''
  return `${e.count} messages sur les ${e.window} derniers, par ${e.users.length} pseudos différents : ${e.users.slice(0, 8).join(', ')}${e.users.length > 8 ? '…' : ''}`
}

function share(g: GiveawayView) {
  return g.evidence ? Math.round(g.evidence.count / Math.max(1, g.evidence.window) * 100) : null
}

// --- Chaînes surveillées ---
const newChannel = ref('')
const adding = ref(false)
async function addChannel() {
  if (!newChannel.value.trim()) return
  adding.value = true
  try {
    const login = await invoke<string>('add_giveaway_channel', { login: newChannel.value })
    newChannel.value = ''
    toast.add({ title: `${login} surveillée`, color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Chaîne non ajoutée', description: String(e), color: 'error' })
  } finally {
    adding.value = false
  }
}

async function importChannels() {
  const added = await invoke<number>('import_giveaway_channels')
  toast.add({ title: added ? `${added} chaîne(s) ajoutée(s)` : 'Toutes tes chaînes sont déjà surveillées', color: 'success', icon: 'i-lucide-check' })
}

const excludedInput = reactive<Record<number, string>>({})
async function addExcluded(c: GiveawayChannelView) {
  const value = (excludedInput[c.id] ?? '').trim()
  if (!value) return
  await invoke('set_giveaway_excluded', { id: c.id, excluded: [...c.excluded, value] })
  excludedInput[c.id] = ''
}
async function removeExcluded(c: GiveawayChannelView, cmd: string) {
  await invoke('set_giveaway_excluded', { id: c.id, excluded: c.excluded.filter(x => x !== cmd) })
}

const toRemove = ref<GiveawayChannelView | null>(null)
async function confirmRemove() {
  if (!toRemove.value) return
  await invoke('remove_giveaway_channel', { id: toRemove.value.id })
  toRemove.value = null
}

// --- Réglages ---
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
const confirmAuto = ref(false)
function setAuto(value: boolean) {
  if (!settings.value) return
  if (value) confirmAuto.value = true
  else settings.value.autoParticipate = false
}

function channelUrl(login: string) {
  return `https://www.twitch.tv/${login}`
}
</script>

<template>
  <div class="p-6 flex flex-col gap-6">
    <h1 class="text-xl font-semibold">Giveaways</h1>

    <UAlert
      v-if="accounts.length === 0"
      title="Connecte d'abord un compte Twitch"
      :actions="[{ label: 'Comptes', to: '/accounts' }]"
      color="neutral"
      variant="subtle"
    />

    <!-- File d'alertes -->
    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">
        File d'alertes <span v-if="alerts.length + mentions.length" class="text-primary">({{ alerts.length + mentions.length }})</span>
      </h2>
      <p v-if="alerts.length === 0 && mentions.length === 0" class="text-sm text-muted">
        Rien en attente. Les giveaways détectés dans les chaînes surveillées et les mentions de tes pseudos apparaîtront ici, avec une notification Windows.
      </p>

      <div v-for="g in alerts" :key="g.id" class="border border-primary/40 bg-primary/5 rounded-lg px-4 py-3 flex flex-col gap-2">
        <div class="flex items-center gap-3 flex-wrap">
          <UIcon name="i-lucide-gift" class="size-5 text-primary" />
          <span class="font-medium">
            {{ g.kind === 'streamelements' ? 'Giveaway StreamElements' : 'Giveaway' }} sur
            <button class="hover:underline" @click="openExternal(channelUrl(g.channel))">{{ g.channel }}</button>
          </span>
          <span v-if="g.command" class="font-mono bg-elevated px-1.5 py-0.5 rounded">{{ g.command }}</span>
          <UTooltip v-if="g.evidence" :text="evidenceText(g)">
            <span class="text-sm text-muted underline decoration-dotted cursor-help">{{ share(g) }} % du chat</span>
          </UTooltip>
          <span class="text-xs text-muted ml-auto">{{ timeAgo(g.detectedAt, now) }}</span>
        </div>
        <div class="flex items-center gap-2 flex-wrap">
          <template v-if="g.kind === 'command'">
            <span class="text-sm text-muted">Participer avec</span>
            <UButton
              v-for="a in accounts"
              :key="a.id"
              :label="a.login"
              size="xs"
              :icon="entered(g, a.id) ? 'i-lucide-check' : chosenFor(g).includes(a.id) ? 'i-lucide-square-check' : 'i-lucide-square'"
              :color="entered(g, a.id) ? 'success' : 'neutral'"
              :variant="chosenFor(g).includes(a.id) || entered(g, a.id) ? 'soft' : 'ghost'"
              :disabled="entered(g, a.id)"
              @click="toggleAccount(g, a.id)"
            />
            <UButton label="Participer" icon="i-lucide-send" size="sm" :loading="participating === g.id" :disabled="!chosenFor(g).some(id => !entered(g, id))" @click="participate(g)" />
          </template>
          <template v-else>
            <UButton v-if="g.url" label="Voir le giveaway" icon="i-lucide-external-link" size="sm" @click="openExternal(g.url)" />
            <UButton label="Acheter des tickets" icon="i-lucide-ticket" size="sm" color="neutral" variant="soft" to="/streamelements" />
          </template>
          <UButton label="Ignorer" size="sm" color="neutral" variant="ghost" @click="invoke('dismiss_giveaway', { id: g.id })" />
        </div>
      </div>

      <div v-for="m in mentions" :key="`m${m.id}`" class="border border-warning/40 bg-warning/10 rounded-lg px-4 py-3 flex items-start gap-3">
        <UIcon name="i-lucide-at-sign" class="size-5 text-warning mt-0.5" />
        <div class="flex-1 min-w-0">
          <div class="font-medium">{{ m.author }} mentionne {{ m.account }} sur <button class="hover:underline" @click="openExternal(channelUrl(m.channel))">{{ m.channel }}</button></div>
          <div class="text-sm text-muted break-words">« {{ m.message }} »</div>
        </div>
        <span class="text-xs text-muted">{{ timeAgo(m.at, now) }}</span>
        <UButton label="Vu" size="xs" color="neutral" variant="soft" @click="invoke('dismiss_mention', { id: m.id })" />
      </div>
    </section>

    <!-- Giveaways récents -->
    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">Giveaways récents</h2>
      <div class="border border-default rounded-lg overflow-x-auto">
        <table class="w-full text-sm">
          <thead class="bg-elevated/60 text-xs uppercase text-muted">
            <tr>
              <th class="text-left px-3 py-2">Détecté</th>
              <th class="text-left px-3 py-2">Chaîne</th>
              <th class="text-left px-3 py-2">Commande</th>
              <th class="text-left px-3 py-2">Participations</th>
              <th class="text-left px-3 py-2">Gagnant</th>
              <th class="text-left px-3 py-2">État</th>
              <th class="px-3 py-2" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="g in giveaways" :key="g.id" class="border-t border-default" :class="g.winnerOurs ? 'bg-success/10' : ''">
              <td class="px-3 py-2 whitespace-nowrap text-muted">{{ timeAgo(g.detectedAt, now) }}</td>
              <td class="px-3 py-2"><button class="font-medium hover:underline" @click="openExternal(channelUrl(g.channel))">{{ g.channel }}</button></td>
              <td class="px-3 py-2">
                <span v-if="g.command" class="font-mono">{{ g.command }}</span>
                <button v-else-if="g.url" class="text-primary hover:underline" @click="openExternal(g.url)">StreamElements</button>
              </td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap gap-1">
                  <UBadge v-for="e in g.entriesView" :key="e.accountId" :label="e.login" :icon="e.auto ? 'i-lucide-bot' : 'i-lucide-check'" color="success" variant="subtle" size="sm" :title="e.auto ? 'Participation automatique' : 'Participation'" />
                  <span v-if="g.entriesView.length === 0" class="text-muted">—</span>
                </div>
              </td>
              <td class="px-3 py-2">
                <template v-if="g.winner">
                  <span :class="g.winnerOurs ? 'text-success font-semibold' : ''">{{ g.winner }}</span>
                  <span v-if="g.winnerChance" class="text-xs text-muted"> ({{ g.winnerChance }})</span>
                  <UBadge v-if="g.winnerOurs" label="Gagné !" color="success" size="sm" class="ml-1" />
                </template>
                <span v-else class="text-muted">—</span>
              </td>
              <td class="px-3 py-2">
                <UBadge :label="g.open ? 'Ouvert' : 'Terminé'" :color="g.open ? 'primary' : 'neutral'" variant="subtle" />
              </td>
              <td class="px-3 py-2">
                <div class="flex justify-end gap-1">
                  <UButton
                    v-if="g.open && g.kind === 'command' && accounts.some(a => !entered(g, a.id))"
                    label="Participer"
                    size="xs"
                    :loading="participating === g.id"
                    @click="participate(g)"
                  />
                  <UButton v-if="g.open" label="Clore" size="xs" color="neutral" variant="ghost" @click="invoke('close_giveaway', { id: g.id })" />
                  <UButton icon="i-lucide-trash-2" size="xs" color="error" variant="ghost" aria-label="Supprimer" @click="invoke('delete_giveaway', { id: g.id })" />
                </div>
              </td>
            </tr>
            <tr v-if="giveaways.length === 0">
              <td colspan="7" class="px-3 py-8 text-center text-muted">Aucun giveaway détecté pour l'instant.</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <!-- Chaînes surveillées -->
    <section class="flex flex-col gap-2">
      <div class="flex items-center gap-3">
        <h2 class="text-sm font-semibold uppercase text-muted tracking-wide flex-1">Chaînes surveillées</h2>
        <UButton label="Ajouter mes chaînes Points et StreamElements" icon="i-lucide-list-plus" size="xs" color="neutral" variant="soft" @click="importChannels" />
      </div>
      <form class="flex gap-2" @submit.prevent="addChannel">
        <UInput id="new-giveaway-channel" v-model="newChannel" placeholder="Nom ou adresse de la chaîne (ex. lunarsea)" class="flex-1" icon="i-lucide-plus" />
        <UButton type="submit" label="Ajouter" :loading="adding" />
      </form>
      <div class="border border-default rounded-lg overflow-x-auto">
        <table class="w-full text-sm">
          <thead class="bg-elevated/60 text-xs uppercase text-muted">
            <tr>
              <th class="text-left px-3 py-2">Chaîne</th>
              <th class="text-left px-3 py-2">Chat lu par</th>
              <th class="text-left px-3 py-2">Commandes ignorées</th>
              <th class="px-3 py-2" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="c in channels" :key="c.id" class="border-t border-default">
              <td class="px-3 py-2"><button class="font-medium hover:underline" @click="openExternal(channelUrl(c.login))">{{ c.login }}</button></td>
              <td class="px-3 py-2">
                <UTooltip :text="c.reader === 'anonyme' ? 'Connexion anonyme commune (lecture seule)' : c.reader ? 'Connexion déjà présente pour StreamElements : pas de connexion en plus' : 'En cours de connexion'">
                  <span class="inline-flex items-center gap-1.5">
                    <span class="size-2 rounded-full" :class="c.reader ? 'bg-success' : 'bg-neutral-400'" />
                    {{ c.reader ?? 'Connexion…' }}
                  </span>
                </UTooltip>
              </td>
              <td class="px-3 py-2">
                <div class="flex items-center gap-1 flex-wrap">
                  <UBadge v-for="cmd in c.excluded" :key="cmd" color="neutral" variant="subtle" class="font-mono">
                    {{ cmd }}
                    <button class="ml-1 hover:text-error" :aria-label="`Ne plus ignorer ${cmd}`" @click="removeExcluded(c, cmd)">×</button>
                  </UBadge>
                  <form class="inline-flex" @submit.prevent="addExcluded(c)">
                    <UInput :id="`excluded-${c.id}`" v-model="excludedInput[c.id]" placeholder="!commande" size="xs" class="w-28" />
                  </form>
                </div>
              </td>
              <td class="px-3 py-2 text-right">
                <UButton icon="i-lucide-trash-2" size="xs" variant="ghost" color="error" aria-label="Ne plus surveiller" @click="toRemove = c" />
              </td>
            </tr>
            <tr v-if="channels.length === 0">
              <td colspan="4" class="px-3 py-8 text-center text-muted">Aucune chaîne surveillée. Ajoute la première avec le champ ci-dessus.</td>
            </tr>
          </tbody>
        </table>
      </div>
      <p class="text-xs text-muted max-w-[85ch]">
        Une chaîne où l'un de tes comptes est déjà présent pour StreamElements est lue via cette connexion. Les autres sont lues par une seule connexion anonyme commune, qui n'apparaît pour aucun de tes comptes.
        Une commande ignorée (ex. <span class="font-mono">!points</span>) ne déclenche jamais d'alerte sur la chaîne.
      </p>
    </section>

    <!-- Réglages -->
    <section v-if="settings" class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">Alertes et participation</h2>
      <div class="border border-default rounded-lg divide-y divide-default">
        <div class="flex items-center justify-between gap-4 px-4 py-3">
          <div>
            <div class="font-medium">Notifications Windows</div>
            <div class="text-sm text-muted">Giveaway détecté, mention de tes pseudos, victoire.</div>
          </div>
          <USwitch v-model="settings.notifications" aria-label="Notifications Windows" />
        </div>
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
    </section>

    <UModal :open="confirmAuto" title="Activer la participation automatique ?" @update:open="v => { if (!v) confirmAuto = false }">
      <template #body>
        <div class="flex flex-col gap-2 text-sm">
          <p>L'application enverra la commande détectée dans le chat avec tes comptes, sans te demander.</p>
          <p>Une détection peut se tromper (commande d'un jeu, d'un sondage…), et des messages automatiques peuvent être mal vus par un streamer ou par Twitch. Ajoute les commandes à ne jamais envoyer dans « Commandes ignorées ».</p>
        </div>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="confirmAuto = false" />
          <UButton label="Activer" @click="() => { if (settings) settings.autoParticipate = true; confirmAuto = false }" />
        </div>
      </template>
    </UModal>

    <UModal :open="toRemove !== null" title="Ne plus surveiller la chaîne ?" @update:open="v => { if (!v) toRemove = null }">
      <template #body>
        <p>Les giveaways de {{ toRemove?.login }} ne seront plus détectés. L'historique est conservé.</p>
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
