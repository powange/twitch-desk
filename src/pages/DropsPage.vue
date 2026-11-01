<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../composables/useSnapshot'
import { useOpenExternal } from '../utils/external'
import { formatNumber } from '../utils/format'
import type { DropCampaign, DropGameStatus, DropGameView, TimedDrop } from '../types'

const { accounts } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()

const accountId = ref<number | undefined>()
watch(accounts, (all) => {
  if (!all.some(a => a.id === accountId.value)) accountId.value = all[0]?.id
}, { immediate: true })
const account = computed(() => accounts.value.find(a => a.id === accountId.value))
const accountItems = computed(() => accounts.value.map(a => ({ label: a.login, value: a.id })))
const drops = computed(() => account.value?.drops)

// --- Jeux souhaités ---
const newGame = ref('')
const adding = ref(false)

async function addGame(input = newGame.value) {
  if (!account.value || !input.trim()) return
  adding.value = true
  try {
    const name = await invoke<string>('add_drop_game', { accountId: account.value.id, input })
    if (input === newGame.value) newGame.value = ''
    toast.add({ title: `${name} ajouté`, color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Jeu non ajouté', description: String(e), color: 'error' })
  } finally {
    adding.value = false
  }
}

async function setGameEnabled(game: DropGameView, enabled: boolean) {
  await invoke('set_drop_game_enabled', { id: game.id, enabled })
}

const toRemove = ref<DropGameView | null>(null)
async function confirmRemove() {
  if (!toRemove.value) return
  await invoke('remove_drop_game', { id: toRemove.value.id })
  toRemove.value = null
}

const STATUS: Record<DropGameStatus, { label: string, color: 'success' | 'warning' | 'neutral' | 'info' | 'primary', hint: string }> = {
  watching: { label: 'En cours', color: 'success', hint: 'Un stream de ce jeu est regardé pour ses drops' },
  waiting: { label: 'En attente', color: 'warning', hint: 'Des streams ont les drops activés, mais un jeu plus haut dans la liste passe avant' },
  noStreams: { label: 'Pas de campagne', color: 'neutral', hint: 'Pas assez de streams en live avec les drops activés pour une campagne (3 minimum sans campagne commencée) : surveillé jusqu\'à la prochaine' },
  done: { label: 'Terminé', color: 'info', hint: 'Tous les drops de la campagne sont obtenus' },
  noProgress: { label: 'Aucune progression', color: 'warning', hint: 'Campagne déjà terminée ou pas ouverte à ce compte. Nouvel essai dans 1 h' },
  unknown: { label: 'Vérification…', color: 'neutral', hint: '' },
  disabled: { label: 'Désactivé', color: 'neutral', hint: '' }
}

// Glisser-déposer par la poignée (même principe que la page Points de chaîne)
const games = computed(() => drops.value?.games ?? [])
const grabbed = ref<number | null>(null)
const dragging = ref<number | null>(null)
const preview = ref<number[] | null>(null)
const pending = ref<number[] | null>(null)
let pendingTimer: ReturnType<typeof setTimeout> | null = null

const orderedGames = computed(() => {
  const order = preview.value ?? pending.value
  if (!order) return games.value
  const byId = new Map(games.value.map(g => [g.id, g]))
  const list = order.map(id => byId.get(id)).filter((g): g is DropGameView => !!g)
  return [...list, ...games.value.filter(g => !order.includes(g.id))]
})

watch(games, (list) => {
  if (pending.value && list.map(g => g.id).join() === pending.value.join()) clearPending()
})

function clearPending() {
  pending.value = null
  if (pendingTimer) clearTimeout(pendingTimer)
  pendingTimer = null
}

function onDragStart(event: DragEvent, game: DropGameView) {
  dragging.value = game.id
  preview.value = games.value.map(g => g.id)
  event.dataTransfer?.setData('text/plain', game.name)
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
}

function onDragOver(event: DragEvent, target: DropGameView) {
  if (dragging.value === null || !preview.value) return
  event.preventDefault()
  if (target.id === dragging.value) return
  const ids = [...preview.value]
  ids.splice(ids.indexOf(dragging.value), 1)
  ids.splice(ids.indexOf(target.id), 0, dragging.value)
  preview.value = ids
}

async function onDrop(event: DragEvent) {
  event.preventDefault()
  const ids = preview.value
  const changed = ids && ids.some((id, i) => id !== games.value[i]?.id)
  if (changed) {
    pending.value = ids
    pendingTimer = setTimeout(clearPending, 5000)
  }
  endDrag()
  if (changed && account.value) {
    try {
      await invoke('reorder_drop_games', { accountId: account.value.id, ids })
    } catch {
      clearPending()
    }
  }
}

function endDrag() {
  dragging.value = null
  grabbed.value = null
  preview.value = null
}

// --- Campagnes ---
const campaigns = computed(() => drops.value?.campaigns ?? [])
const wantedIds = computed(() => new Set(games.value.map(g => g.gameId)))

// Jaquette Twitch : l'adresse porte sa taille (…-40x56.jpg), on demande le double pour un rendu net.
// Inconnue (jeu sans stream en live) : adresse construite à partir de l'identifiant du jeu
function boxArt(url: string | null, gameId: string) {
  if (!url) return `https://static-cdn.jtvnw.net/ttv-boxart/${gameId}-80x112.jpg`
  return url.replace(/-\d+x\d+\./, '-80x112.').replace('{width}', '80').replace('{height}', '112')
}

function categoryUrl(slug: string | null) {
  return slug ? `https://www.twitch.tv/directory/category/${slug}` : null
}

function dropState(campaign: DropCampaign, drop: TimedDrop): { label: string, color: 'success' | 'warning' | 'neutral' | 'info' | 'error' } {
  if (drop.claimed) return { label: 'Obtenu', color: 'success' }
  if (drop.currentMinutes >= drop.requiredMinutes && drop.requiredMinutes > 0) {
    return campaign.accountLinked ? { label: 'À réclamer', color: 'info' } : { label: 'Compte du jeu à lier', color: 'error' }
  }
  if (drop.currentMinutes > 0) return { label: 'En cours', color: 'warning' }
  return { label: 'À faire', color: 'neutral' }
}

function percent(drop: TimedDrop) {
  if (drop.claimed) return 100
  return drop.requiredMinutes ? Math.min(100, Math.round(drop.currentMinutes / drop.requiredMinutes * 100)) : 0
}

function formatMinutes(m: number) {
  if (m < 60) return `${m} min`
  const h = Math.floor(m / 60)
  return m % 60 ? `${h} h ${m % 60}` : `${h} h`
}

function formatDate(iso: string) {
  if (!iso) return '—'
  return new Date(iso).toLocaleDateString('fr-FR', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}

function rewardNames(drop: TimedDrop) {
  return drop.rewards.map(r => r.name).join(', ') || drop.name
}

async function setDropsEnabled(enabled: boolean) {
  if (account.value) await invoke('set_drops_enabled', { id: account.value.id, enabled })
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <div class="flex items-center gap-3">
      <h1 class="text-xl font-semibold flex-1">Drops</h1>
      <USelect v-if="accounts.length > 1" v-model="accountId" :items="accountItems" class="w-48" icon="i-lucide-user" />
    </div>

    <UAlert
      v-if="accounts.length === 0"
      title="Connecte d'abord un compte Twitch"
      :actions="[{ label: 'Comptes', to: '/accounts' }]"
      color="neutral"
      variant="subtle"
    />

    <template v-else-if="account && drops">
      <UAlert
        icon="i-lucide-info"
        color="info"
        variant="subtle"
        title="Choisis les jeux dont tu veux les drops, dans l'ordre de priorité, même sans campagne en cours."
        description="L'application regarde un stream avec les drops activés du premier jeu disponible et réclame chaque drop terminé. Les drops passent avant les points de chaîne : la chaîne regardée pour les drops prend une des 2 places du compte."
      />

      <div v-if="!drops.enabled" class="flex items-center gap-3 border border-warning/40 bg-warning/10 rounded-lg px-4 py-2.5 text-sm">
        <UIcon name="i-lucide-pause" class="size-4 text-warning" />
        <span class="flex-1">Drops désactivés pour {{ account.login }}.</span>
        <UButton label="Activer" size="xs" @click="setDropsEnabled(true)" />
      </div>

      <div v-if="drops.target" class="flex items-center gap-3 border border-success/40 bg-success/10 rounded-lg px-4 py-2.5 text-sm">
        <UIcon name="i-lucide-package-open" class="size-5 text-success" />
        <span class="flex-1 min-w-0 truncate">
          <b>{{ drops.target.gameName }}</b> : visionnage de
          <button class="font-medium hover:underline" @click="openExternal(`https://www.twitch.tv/${drops.target.login}`)">{{ drops.target.login }}</button>
          <span class="inline-flex items-center gap-0.5 text-error ml-1.5 align-middle" :title="`${formatNumber(drops.target.viewers)} spectateurs`"><UIcon name="i-lucide-eye" class="size-3.5" />{{ formatNumber(drops.target.viewers) }}</span>
          <span class="text-muted"> · {{ drops.target.title }}</span>
        </span>
      </div>

      <UAlert v-if="drops.error" icon="i-lucide-triangle-alert" color="warning" variant="subtle" :title="drops.error" />

      <!-- Jeux souhaités -->
      <section class="flex flex-col gap-2">
        <h2 class="text-sm font-semibold uppercase text-muted tracking-wide">Jeux souhaités</h2>
        <form class="flex gap-2" @submit.prevent="addGame()">
          <UInput
            id="new-drop-game"
            v-model="newGame"
            placeholder="Nom exact du jeu (ex. Sea of Thieves) ou adresse de sa catégorie Twitch"
            class="flex-1"
            icon="i-lucide-plus"
          />
          <UButton type="submit" label="Ajouter" :loading="adding" />
        </form>

        <div class="border border-default rounded-lg overflow-x-auto">
          <table class="w-full text-sm">
            <thead class="bg-elevated/60 text-xs uppercase text-muted">
              <tr>
                <th class="text-left px-3 py-2 w-16">Ordre</th>
                <th class="text-left px-3 py-2">Jeu</th>
                <th class="text-left px-3 py-2">État</th>
                <th class="text-right px-3 py-2 whitespace-nowrap">Streams avec drops</th>
                <th class="text-center px-3 py-2">Actif</th>
                <th class="px-3 py-2" />
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="(game, i) in orderedGames"
                :key="game.id"
                class="border-t border-default transition-colors"
                :class="dragging === game.id ? 'bg-primary/10 opacity-70' : ''"
                :draggable="grabbed === game.id"
                @dragstart="onDragStart($event, game)"
                @dragover="onDragOver($event, game)"
                @drop="onDrop"
                @dragend="endDrag"
              >
                <td class="px-3 py-2">
                  <div
                    class="flex items-center gap-1.5 cursor-grab active:cursor-grabbing select-none"
                    title="Glisser pour changer l'ordre"
                    @mousedown="grabbed = game.id"
                    @mouseup="grabbed = null"
                  >
                    <UIcon name="i-lucide-grip-vertical" class="size-4 text-muted" />
                    <span class="w-5 text-muted">{{ i + 1 }}</span>
                  </div>
                </td>
                <td class="px-3 py-2">
                  <div class="flex items-center gap-3">
                    <img :src="boxArt(game.boxArt, game.gameId)" alt="" class="w-10 h-14 rounded object-cover shrink-0 bg-elevated">
                    <button class="font-medium hover:underline text-left" @click="openExternal(categoryUrl(game.slug) ?? '')">{{ game.name }}</button>
                  </div>
                </td>
                <td class="px-3 py-2">
                  <UTooltip :text="STATUS[game.status].hint" :disabled="!STATUS[game.status].hint">
                    <UBadge :color="STATUS[game.status].color" variant="subtle" :label="STATUS[game.status].label" class="whitespace-nowrap" />
                  </UTooltip>
                </td>
                <td class="px-3 py-2 text-right" :class="game.streams ? '' : 'text-muted'">
                  {{ game.status === 'unknown' ? '—' : formatNumber(game.streams) }}
                </td>
                <td class="px-3 py-2">
                  <div class="flex justify-center">
                    <USwitch :model-value="game.enabled" :aria-label="`Drops de ${game.name}`" @update:model-value="v => setGameEnabled(game, v)" />
                  </div>
                </td>
                <td class="px-3 py-2 text-right">
                  <UButton icon="i-lucide-trash-2" size="xs" variant="ghost" color="error" aria-label="Retirer le jeu" @click="toRemove = game" />
                </td>
              </tr>
              <tr v-if="games.length === 0">
                <td colspan="6" class="px-3 py-8 text-center text-muted">Aucun jeu. Ajoute le premier avec le champ ci-dessus.</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <!-- Campagnes commencées -->
      <section class="flex flex-col gap-2">
        <div class="flex items-baseline gap-3">
          <h2 class="text-sm font-semibold uppercase text-muted tracking-wide flex-1">Campagnes commencées</h2>
          <UTooltip text="La progression avance en direct ; l'inventaire complet est relu toutes les 10 minutes">
            <span v-if="drops.inventoryAt" class="text-xs text-muted">Inventaire relu à {{ new Date(drops.inventoryAt).toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' }) }}</span>
          </UTooltip>
          <UButton label="Relire maintenant" icon="i-lucide-refresh-cw" size="xs" color="neutral" variant="ghost" @click="account && invoke('refresh_drops', { accountId: account.id })" />
        </div>
        <p v-if="campaigns.length === 0" class="text-sm text-muted">
          Aucune campagne commencée. Une campagne apparaît ici dès la première minute regardée.
        </p>

        <div v-for="campaign in campaigns" :key="campaign.id" class="border border-default rounded-lg overflow-hidden">
          <div class="flex items-center gap-3 px-4 py-3 bg-elevated/40 border-b border-default flex-wrap">
            <img :src="boxArt(campaign.game.boxArt, campaign.game.id)" alt="" class="w-8 h-11 rounded object-cover bg-elevated">
            <div class="flex-1 min-w-0">
              <div class="font-medium">{{ campaign.game.name }} <span class="text-muted font-normal">· {{ campaign.name }}</span></div>
              <div class="text-xs text-muted">Jusqu'au {{ formatDate(campaign.endsAt) }}<template v-if="campaign.allowedChannels.length"> · {{ campaign.allowedChannels.length }} chaîne(s) imposée(s)</template></div>
            </div>
            <UBadge v-if="campaign.accountLinked" color="success" variant="subtle" label="Compte du jeu lié" icon="i-lucide-link" />
            <template v-else>
              <UBadge color="error" variant="subtle" label="Compte du jeu non lié" icon="i-lucide-unlink" />
              <UButton v-if="campaign.linkUrl" label="Lier le compte" size="xs" icon="i-lucide-external-link" @click="openExternal(campaign.linkUrl)" />
            </template>
            <UButton
              v-if="!wantedIds.has(campaign.game.id) && campaign.game.slug"
              label="Ajouter ce jeu"
              size="xs"
              color="neutral"
              variant="soft"
              icon="i-lucide-plus"
              :loading="adding"
              @click="addGame(categoryUrl(campaign.game.slug) ?? '')"
            />
          </div>
          <table class="w-full text-sm">
            <tbody>
              <tr v-for="drop in campaign.drops" :key="drop.id" class="border-t border-default first:border-t-0">
                <td class="px-4 py-2 min-w-0">
                  <div class="flex items-center gap-2">
                    <img v-if="drop.rewards[0]?.image" :src="drop.rewards[0].image" alt="" class="size-8 rounded object-cover">
                    <span>{{ rewardNames(drop) }}</span>
                  </div>
                </td>
                <td class="px-4 py-2 w-[34%]">
                  <UProgress :model-value="percent(drop)" size="sm" :color="drop.claimed ? 'success' : 'primary'" />
                  <div class="text-xs text-muted mt-1">{{ formatMinutes(Math.min(drop.currentMinutes, drop.requiredMinutes)) }} / {{ formatMinutes(drop.requiredMinutes) }}</div>
                </td>
                <td class="px-4 py-2 text-right w-44">
                  <UBadge :color="dropState(campaign, drop).color" variant="subtle" :label="dropState(campaign, drop).label" class="whitespace-nowrap" />
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </template>

    <UModal :open="toRemove !== null" title="Retirer le jeu ?" @update:open="v => { if (!v) toRemove = null }">
      <template #body>
        <p>{{ toRemove?.name }} ne sera plus regardé pour ses drops. Les drops déjà commencés restent dans ton inventaire Twitch.</p>
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
