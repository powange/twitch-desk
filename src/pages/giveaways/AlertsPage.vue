<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { RouterLink } from 'vue-router'
import { useSnapshot } from '../../composables/useSnapshot'
import { useOpenExternal } from '../../utils/external'
import { useWatchStream } from '../../utils/viewer'
import { channelUrl, formatNumber, timeAgo } from '../../utils/format'
import type { GiveawayView, ParticipationResult } from '../../types'
import ChannelFlag from '../../components/ChannelFlag.vue'
import GiveawayConditions from '../../components/GiveawayConditions.vue'
import GiveawayFormModal from '../../components/GiveawayFormModal.vue'
import ChatLogModal from '../../components/ChatLogModal.vue'
import { useGiftCatalog } from '../../composables/useGiftCatalog'
import { GIVEAWAY_KIND, formatDrawAt } from '../../utils/giveaways'
import SolidIcon from '../../components/SolidIcon.vue'
import ConditionHeaders from '../../components/conditions/ConditionHeaders.vue'
import ConditionCells from '../../components/conditions/ConditionCells.vue'
import { useConditionActions } from '../../composables/useConditionActions'
import { useAction } from '../../composables/useAction'

const { snapshot, accounts } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()
const watchStream = useWatchStream()

const view = computed(() => snapshot.value?.giveaways)
const giveaways = computed(() => view.value?.giveaways ?? [])
// Live en cours d'une chaîne surveillée (catégorie, spectateurs), indexé une fois par mise à jour
const liveByLogin = computed(() => new Map((view.value?.channels ?? []).map(c => [c.login, c.live])))
const liveOf = (channel: string) => liveByLogin.value.get(channel) ?? null

// Conditions : follow, bonjour, Discord (giveaways ouverts seulement : rien à faire sur un giveaway clos)
const cond = useConditionActions(() => giveaways.value.filter(g => g.open))

// Ignorer, Vu, Clore, Supprimer : erreur affichée, bouton occupé pendant l'action
const { run, busy } = useAction()
const dismiss = (g: GiveawayView) => run(`dismiss-${g.id}`, 'Alerte non ignorée', () => invoke('dismiss_giveaway', { id: g.id }))
const dismissMention = (id: number) => run(`mention-${id}`, 'Mention non marquée comme vue', () => invoke('dismiss_mention', { id }))
const closeGiveaway = (g: GiveawayView) => run(`close-${g.id}`, 'Giveaway non clos', () => invoke('close_giveaway', { id: g.id }))
const toDelete = ref<GiveawayView | null>(null)
async function confirmDelete() {
  const g = toDelete.value
  if (!g) return
  if (await run('delete', 'Giveaway non supprimé', () => invoke('delete_giveaway', { id: g.id }))) toDelete.value = null
}

// Journal du chat d'une chaîne
const logChannel = ref<string | null>(null)
const logOpen = ref(false)
function openLog(login: string) {
  logChannel.value = login
  logOpen.value = true
}

// Commande : copiée d'un clic, modifiable dans une mini-fenêtre
async function copyCommand(command: string) {
  try {
    await navigator.clipboard.writeText(command)
    toast.add({ title: `${command} copiée`, color: 'success', icon: 'i-lucide-clipboard-check' })
  } catch {
    toast.add({ title: 'Copie impossible', color: 'error' })
  }
}
const editingCommand = ref<number | null>(null)
const commandDraft = ref('')
function startEditCommand(g: GiveawayView) {
  editingCommand.value = g.id
  commandDraft.value = g.command ?? ''
}
async function saveCommand(g: GiveawayView) {
  try {
    await invoke('set_giveaway_command', { id: g.id, command: commandDraft.value.trim() || null })
    editingCommand.value = null
  } catch (e) {
    toast.add({ title: 'Commande non enregistrée', description: String(e), color: 'error' })
  }
}

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

const { byId: giftById } = useGiftCatalog()

// Formulaire : null = nouveau giveaway
const formOpen = ref(false)
const editing = ref<GiveawayView | null>(null)
function openForm(g: GiveawayView | null) {
  editing.value = g
  formOpen.value = true
}

function share(g: GiveawayView) {
  return g.evidence ? Math.round(g.evidence.count / Math.max(1, g.evidence.window) * 100) : null
}
</script>

<template>
  <div class="p-6 flex flex-col gap-6">
    <div class="flex items-center gap-3">
      <h1 class="text-xl font-semibold flex-1">Alertes</h1>
      <UButton label="Nouveau giveaway" icon="i-lucide-plus" @click="openForm(null)" />
    </div>

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
            <ChannelFlag :login="g.channel" class="inline-block align-middle mx-0.5" />
            <button class="hover:underline" @click="openExternal(channelUrl(g.channel))">{{ g.channel }}</button>
          </span>
          <span v-if="liveOf(g.channel)" class="inline-flex items-center gap-1.5 text-xs text-muted">
            {{ liveOf(g.channel)?.gameName ?? 'Sans catégorie' }}
            <span class="inline-flex items-center gap-0.5 text-error"><UIcon name="i-lucide-eye" class="size-3.5" />{{ formatNumber(liveOf(g.channel)?.viewers ?? 0) }}</span>
          </span>
          <UBadge :label="GIVEAWAY_KIND[g.kind].label" :color="GIVEAWAY_KIND[g.kind].color" variant="subtle" size="sm" />
          <UTooltip v-if="g.command" text="Copier la commande">
            <button type="button" class="font-mono bg-elevated px-1.5 py-0.5 rounded hover:text-primary" @click="copyCommand(g.command)">{{ g.command }}</button>
          </UTooltip>
          <!-- Follow requis : cœur rempli si suivie par tous tes comptes, sinon cliquable pour suivre -->
          <UTooltip v-if="g.requireFollow" :text="cond.followTooltip(g)">
            <button
              v-if="cond.notFollowing(g.channel).length"
              type="button"
              class="inline-flex items-center gap-0.5 text-warning hover:text-pink-500 disabled:opacity-50"
              :disabled="cond.following === g.channel"
              :aria-label="`${cond.followLabel(g)}. Suivre ${g.channel}`"
              @click="cond.followChannel(g.channel)"
            >
              <UIcon :name="cond.following === g.channel ? 'i-lucide-loader-circle' : 'i-lucide-heart'" class="size-4" :class="cond.following === g.channel ? 'animate-spin' : ''" />
              <span v-if="g.followMinDays" class="text-xs font-medium">{{ g.followMinDays }} j</span>
            </button>
            <span
              v-else
              class="inline-flex items-center gap-0.5 rounded focus-visible:outline-2 focus-visible:outline-primary"
              :class="cond.followOk(g) ? 'text-success' : cond.tooRecent(g).length ? 'text-warning' : 'text-pink-600 dark:text-pink-400'"
              tabindex="0"
              role="img"
              :aria-label="cond.followLabel(g)"
            >
              <SolidIcon v-if="cond.followOk(g)" name="heart" class="size-4" />
              <UIcon v-else name="i-lucide-heart" class="size-4" />
              <span v-if="g.followMinDays" class="text-xs font-medium">{{ g.followMinDays }} j</span>
            </span>
          </UTooltip>
          <GiveawayConditions :details="{ ...g, requireFollow: g.requireFollow ? null : g.requireFollow }" :kind="g.kind" />
          <span v-if="g.drawAt" class="text-sm text-muted inline-flex items-center gap-1"><UIcon name="i-lucide-clock" class="size-4" />Tirage {{ formatDrawAt(g.drawAt) }}</span>
          <UTooltip v-if="g.evidence" :text="evidenceText(g)">
            <span class="text-sm text-muted underline decoration-dotted cursor-help rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" :aria-label="evidenceText(g)">{{ share(g) }} % du chat</span>
          </UTooltip>
          <span class="text-xs text-muted ml-auto">{{ timeAgo(g.detectedAt, now) }}</span>
        </div>
        <!-- Réponse d'un bot à !giveaway… sur la chaîne -->
        <div v-for="r in view?.botReplies[g.channel] ?? []" :key="r.command" class="text-sm flex items-start gap-2">
          <UIcon name="i-lucide-message-square-text" class="size-4 text-muted mt-0.5 shrink-0" />
          <span><span class="text-muted">Réponse de {{ r.bot }} à <span class="font-mono">{{ r.command }}</span> :</span> {{ r.message }}</span>
        </div>
        <div class="flex items-center gap-2 flex-wrap">
          <span v-if="(g.kind === 'command' || g.kind === 'ticket') && !g.command" class="text-sm text-muted">
            <UIcon name="i-lucide-hourglass" class="size-3.5 align-middle" /> Commande pas encore connue : elle sera remplie dès que la détection la repère dans le chat.
          </span>
          <template v-else-if="g.kind === 'command' || g.kind === 'ticket'">
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
          <UButton label="Regarder" icon="i-lucide-tv" size="sm" color="neutral" variant="soft" title="Ajouter le stream à la fenêtre multi-stream" @click="watchStream(g.channel)" />
          <UButton label="Modifier" icon="i-lucide-pencil" size="sm" color="neutral" variant="ghost" @click="openForm(g)" />
          <UButton label="Ignorer" size="sm" color="neutral" variant="ghost" :loading="busy(`dismiss-${g.id}`)" @click="dismiss(g)" />
        </div>
      </div>

      <div v-for="m in mentions" :key="`m${m.id}`" class="border border-warning/40 bg-warning/10 rounded-lg px-4 py-3 flex items-start gap-3">
        <UIcon name="i-lucide-at-sign" class="size-5 text-warning mt-0.5" />
        <div class="flex-1 min-w-0">
          <div class="font-medium">{{ m.author }} mentionne {{ m.account }} sur <button class="hover:underline" @click="openExternal(channelUrl(m.channel))">{{ m.channel }}</button></div>
          <div class="text-sm text-muted break-words">« {{ m.message }} »</div>
        </div>
        <span class="text-xs text-muted">{{ timeAgo(m.at, now) }}</span>
        <UButton label="Vu" size="xs" color="neutral" variant="soft" :loading="busy(`mention-${m.id}`)" @click="dismissMention(m.id)" />
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
              <th class="px-2 py-2"><span class="sr-only">Regarder</span></th>
              <th class="text-left px-3 py-2">Chaîne</th>
              <th class="text-left px-3 py-2">Type</th>
              <th class="text-left px-3 py-2">Commande</th>
              <th class="text-left px-3 py-2">Cadeaux</th>
              <ConditionHeaders />
              <th class="text-left px-3 py-2">Tirage</th>
              <th class="text-left px-3 py-2">Participations</th>
              <th class="text-left px-3 py-2">Gagnant</th>
              <th class="text-left px-3 py-2">État</th>
              <th class="px-3 py-2" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="g in giveaways" :key="g.id" class="border-t border-default" :class="g.winnerOurs ? 'bg-success/10' : ''">
              <td class="px-3 py-2 whitespace-nowrap text-muted">{{ timeAgo(g.detectedAt, now) }}</td>
              <td class="px-2 py-2">
                <UButton v-if="liveOf(g.channel)" icon="i-lucide-tv" size="xs" color="neutral" variant="ghost" :aria-label="`Regarder ${g.channel}`" title="Regarder (fenêtre multi-stream)" @click="watchStream(g.channel)" />
              </td>
              <td class="px-3 py-2">
                <div class="flex items-center gap-1.5">
                  <ChannelFlag :login="g.channel" />
                  <button class="font-medium hover:underline" @click="openExternal(channelUrl(g.channel))">{{ g.channel }}</button>
                </div>
                <div v-if="liveOf(g.channel)" class="flex items-center gap-1.5 text-xs text-muted min-w-0">
                  <span class="truncate max-w-[220px]" :title="liveOf(g.channel)?.gameName ?? 'Sans catégorie'">{{ liveOf(g.channel)?.gameName ?? 'Sans catégorie' }}</span>
                  <span class="inline-flex items-center gap-0.5 whitespace-nowrap text-error" :title="`${formatNumber(liveOf(g.channel)?.viewers ?? 0)} spectateurs`">
                    <UIcon name="i-lucide-eye" class="size-3.5" />{{ formatNumber(liveOf(g.channel)?.viewers ?? 0) }}
                  </span>
                </div>
              </td>
              <td class="px-3 py-2">
                <div class="flex items-center gap-1.5">
                  <UBadge :label="GIVEAWAY_KIND[g.kind].label" :color="GIVEAWAY_KIND[g.kind].color" variant="subtle" class="whitespace-nowrap" />
                  <!-- Ticket : usage unique ou multi-tickets -->
                  <GiveawayConditions v-if="g.kind === 'ticket' && g.ticketUnique !== null" :details="{ ...g, requireFollow: null, requirePresence: null, requireDiscord: null }" :kind="g.kind" />
                </div>
              </td>
              <td class="px-3 py-2 whitespace-nowrap group/cmd">
                <span class="inline-flex items-center gap-1">
                  <UTooltip v-if="g.command" text="Copier la commande">
                    <button type="button" class="font-mono hover:text-primary" @click="copyCommand(g.command)">{{ g.command }}</button>
                  </UTooltip>
                  <button v-else-if="g.url" class="text-primary hover:underline" @click="openExternal(g.url)">Lien du giveaway</button>
                  <span v-else-if="g.rewardName">{{ g.rewardName }}<template v-if="g.rewardCost !== null"> ({{ g.rewardCost }} pts)</template></span>
                  <span v-else class="text-muted">—</span>
                  <UPopover :open="editingCommand === g.id" @update:open="(v: boolean) => { if (!v) editingCommand = null }">
                    <UButton
                      icon="i-lucide-pencil"
                      size="xs"
                      color="neutral"
                      variant="ghost"
                      class="opacity-0 group-hover/cmd:opacity-100 focus-visible:opacity-100"
                      :class="editingCommand === g.id ? 'opacity-100' : ''"
                      :aria-label="`Modifier la commande du giveaway de ${g.channel}`"
                      title="Modifier la commande"
                      @click="startEditCommand(g)"
                    />
                    <template #content>
                      <form class="flex items-center gap-2 p-2" @submit.prevent="saveCommand(g)">
                        <UInput :id="`cmd-${g.id}`" v-model="commandDraft" placeholder="!join ou mot-clé" size="sm" class="w-44 font-mono" autofocus />
                        <UButton type="submit" icon="i-lucide-check" size="sm" aria-label="Enregistrer la commande" />
                      </form>
                    </template>
                  </UPopover>
                </span>
              </td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap items-center gap-1 min-w-24 max-w-64">
                  <UTooltip v-for="id in g.giftIds" :key="id" :text="giftById.get(id)?.title ?? ''">
                    <img v-if="giftById.get(id)?.image" :src="giftById.get(id)?.image ?? ''" referrerpolicy="no-referrer" alt="" class="size-8 rounded object-contain bg-elevated">
                    <span v-else class="size-8 rounded bg-elevated flex items-center justify-center"><UIcon name="i-lucide-gift" class="size-4 text-muted" /></span>
                  </UTooltip>
                  <span v-if="g.giftIds.length === 0" class="text-muted">—</span>
                </div>
              </td>
              <ConditionCells :target="g" :actions="cond" />
              <td class="px-3 py-2 whitespace-nowrap" :class="g.drawAt ? '' : 'text-muted'">{{ formatDrawAt(g.drawAt) }}</td>
              <td class="px-3 py-2">
                <div class="flex flex-wrap gap-1">
                  <component
                    :is="g.kind === 'streamelements' ? RouterLink : 'span'"
                    v-for="e in g.entriesView"
                    :key="e.accountId"
                    v-bind="g.kind === 'streamelements' ? { to: { path: '/streamelements', query: { channels: g.channel } } } : {}"
                    :title="g.kind === 'streamelements' ? 'Voir sur la page StreamElements' : undefined"
                  >
                    <UBadge
                      :label="e.tickets !== null ? `${e.login} · ${e.tickets} ticket${e.tickets > 1 ? 's' : ''}` : e.login"
                      :icon="e.tickets !== null ? 'i-lucide-ticket' : e.auto ? 'i-lucide-bot' : 'i-lucide-check'"
                      color="success"
                      variant="subtle"
                      size="sm"
                      class="whitespace-nowrap"
                      :class="g.kind === 'streamelements' ? 'hover:bg-success/20 cursor-pointer' : ''"
                      :title="e.tickets !== null ? 'Tickets StreamElements relevés sur StreamElements' : e.auto ? 'Participation automatique' : 'Participation envoyée par l\'application'"
                    />
                  </component>
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
                    v-if="g.open && (g.kind === 'command' || g.kind === 'ticket') && g.command && accounts.some(a => !entered(g, a.id))"
                    label="Participer"
                    size="xs"
                    :loading="participating === g.id"
                    @click="participate(g)"
                  />
                  <UButton icon="i-lucide-scroll-text" size="xs" color="neutral" variant="ghost" :aria-label="`Journal du chat de ${g.channel}`" title="Journal du chat" @click="openLog(g.channel)" />
                  <UButton icon="i-lucide-pencil" size="xs" color="neutral" variant="ghost" aria-label="Modifier" @click="openForm(g)" />
                  <UButton v-if="g.open" label="Clore" size="xs" color="neutral" variant="ghost" :loading="busy(`close-${g.id}`)" @click="closeGiveaway(g)" />
                  <UButton icon="i-lucide-trash-2" size="xs" color="error" variant="ghost" aria-label="Supprimer" @click="toDelete = g" />
                </div>
              </td>
            </tr>
            <tr v-if="giveaways.length === 0">
              <td colspan="16" class="px-3 py-8 text-center text-muted">Aucun giveaway pour l'instant. Ils apparaissent à la détection, ou avec « Nouveau giveaway ».</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <GiveawayFormModal v-model:open="formOpen" :giveaway="editing" />
    <ChatLogModal v-model:open="logOpen" :channel="logChannel" />

    <UModal :open="toDelete !== null" title="Supprimer le giveaway ?" @update:open="v => { if (!v) toDelete = null }">
      <template #body>
        <p>Le giveaway de {{ toDelete?.channel }} sera retiré de l'historique. Cette action est définitive.</p>
      </template>
      <template #footer>
        <div class="flex justify-end gap-2 w-full">
          <UButton label="Annuler" color="neutral" variant="ghost" @click="toDelete = null" />
          <UButton label="Supprimer" color="error" :loading="busy('delete')" @click="confirmDelete" />
        </div>
      </template>
    </UModal>
  </div>
</template>
