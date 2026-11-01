<script setup lang="ts">
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../../composables/useSnapshot'
import { useOpenExternal } from '../../utils/external'
import { useWatchStream } from '../../utils/viewer'
import { boxArt, channelUrl, formatNumber } from '../../utils/format'
import type { Gift, GiveawayChannelView, GiveawayView } from '../../types'
import ChannelFlag from '../../components/ChannelFlag.vue'
import GiveawayFormModal from '../../components/GiveawayFormModal.vue'
import ChatLogModal from '../../components/ChatLogModal.vue'
import { suggestGifts, useGiftCatalog } from '../../composables/useGiftCatalog'
import { GIVEAWAY_KIND } from '../../utils/giveaways'
import SolidIcon from '../../components/SolidIcon.vue'

const { snapshot } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()
const watchStream = useWatchStream()

const view = computed(() => snapshot.value?.giveaways)
const hasCategories = computed(() => (view.value?.games.length ?? 0) > 0)
const { gifts, byId } = useGiftCatalog()

// --- Ce qui rend une chaîne intéressante : giveaway en cours, cadeaux repérés ---
const insightOf = (c: GiveawayChannelView) => (c.live ? snapshot.value?.insights[c.login] : undefined)
// Giveaway ouvert connu de la chaîne (le plus récent)
function openGiveaway(c: GiveawayChannelView): GiveawayView | undefined {
  return view.value?.giveaways.find(g => g.open && g.channel === c.login)
}
// Type en cours : giveaway ouvert connu, sinon giveaway StreamElements actif pas encore ajouté
function currentKind(c: GiveawayChannelView) {
  const g = openGiveaway(c)
  if (g) return { kind: g.kind, command: g.command, added: true }
  const se = insightOf(c)?.streamelements
  if (se) return { kind: 'streamelements' as const, command: se.command, added: false }
  return null
}
// Cadeaux du giveaway ouvert, puis ceux repérés dans le titre du live, la description,
// le message épinglé et le giveaway StreamElements actif
function giftsOf(c: GiveawayChannelView): { gift: Gift, attached: boolean }[] {
  const attached = (openGiveaway(c)?.giftIds ?? []).map(id => byId.value.get(id)).filter((g): g is Gift => !!g)
  const i = insightOf(c)
  const replies = (view.value?.botReplies[c.login] ?? []).map(r => r.message)
  const text = c.live ? [c.live.title, i?.description, i?.pinnedMessage, i?.streamelements?.title, i?.streamelements?.description, ...replies].filter(Boolean).join('\n') : ''
  const found = suggestGifts(gifts.value, text).filter(g => !attached.some(a => a.id === g.id))
  return [...attached.map(gift => ({ gift, attached: true })), ...found.map(gift => ({ gift, attached: false }))]
}
// Cadeaux de chaque chaîne, calculés une fois par mise à jour
const giftsByLogin = computed(() => new Map((view.value?.channels ?? []).map(c => [c.login, giftsOf(c)])))
const channelGifts = (c: GiveawayChannelView) => giftsByLogin.value.get(c.login) ?? []

// Chaîne à suivre de près : en live dans une catégorie choisie avec un cadeau favori repéré
const interesting = (c: GiveawayChannelView) => !!c.live && c.inCategory && channelGifts(c).some(x => x.gift.favorite)

// En live dans une catégorie choisie (cadeau favori repéré d'abord), puis en live
// ailleurs, puis hors ligne ; plus regardées d'abord
function rank(c: GiveawayChannelView): number {
  if (interesting(c)) return 0
  if (c.live && c.inCategory) return 1
  if (c.live) return 2
  return c.liveChecked ? 3 : 4
}
// --- Filtres : catégorie du live et cadeaux (rien de choisi : tout) ---
const OFFLINE = 'offline'
const categoryFilter = ref<string[]>([])
const giftFilter = ref<number[]>([])
// Catégories des lives en cours, plus « Hors ligne »
const categoryItems = computed(() => {
  const map = new Map<string, { name: string, count: number }>()
  let offline = 0
  for (const c of view.value?.channels ?? []) {
    if (!c.live) {
      offline++
      continue
    }
    const id = c.live.gameId ?? ''
    const entry = map.get(id) ?? { name: c.live.gameName ?? 'Sans catégorie', count: 0 }
    entry.count++
    map.set(id, entry)
  }
  const items = [...map].map(([id, e]) => ({ label: `${e.name} (${e.count})`, value: id, gameId: id }))
    .sort((a, b) => a.label.localeCompare(b.label))
  if (offline) items.push({ label: `Hors ligne (${offline})`, value: OFFLINE, gameId: '' })
  return items
})
// Cadeaux repérés ou rattachés sur au moins une chaîne
const giftItems = computed(() => {
  const map = new Map<number, { gift: Gift, count: number }>()
  for (const list of giftsByLogin.value.values()) {
    for (const { gift } of list) {
      const entry = map.get(gift.id) ?? { gift, count: 0 }
      entry.count++
      map.set(gift.id, entry)
    }
  }
  return [...map.values()]
    .sort((a, b) => b.count - a.count || a.gift.title.localeCompare(b.gift.title))
    .map(({ gift, count }) => ({ label: `${gift.title} (${count})`, value: gift.id, image: gift.image }))
})
function matchesFilters(c: GiveawayChannelView) {
  if (categoryFilter.value.length) {
    const key = c.live ? (c.live.gameId ?? '') : OFFLINE
    if (!categoryFilter.value.includes(key)) return false
  }
  if (giftFilter.value.length && !channelGifts(c).some(x => giftFilter.value.includes(x.gift.id))) return false
  return true
}
const filtering = computed(() => categoryFilter.value.length > 0 || giftFilter.value.length > 0)
function clearFilters() {
  categoryFilter.value = []
  giftFilter.value = []
}

const allChannels = computed(() => view.value?.channels ?? [])
const channels = computed(() =>
  allChannels.value.filter(matchesFilters).sort((a, b) =>
    rank(a) - rank(b) || (b.live?.viewers ?? 0) - (a.live?.viewers ?? 0) || a.login.localeCompare(b.login)
  )
)

// --- Ajout ou modification du giveaway de la chaîne ---
const formOpen = ref(false)
const editing = ref<GiveawayView | null>(null)
const formChannel = ref<string | null>(null)
function openForm(c: GiveawayChannelView) {
  editing.value = openGiveaway(c) ?? null
  formChannel.value = c.login
  formOpen.value = true
}

function status(c: GiveawayChannelView): { label: string, color: 'success' | 'warning' | 'neutral', hint: string } {
  if (c.live && c.inCategory) return { label: 'En live', color: 'success', hint: 'Giveaways surveillés' }
  if (c.live) return { label: 'Hors catégorie', color: 'warning', hint: 'En live dans une autre catégorie : pas d\'alerte de giveaway' }
  if (c.liveChecked) return { label: 'Hors ligne', color: 'neutral', hint: '' }
  return { label: 'Vérification…', color: 'neutral', hint: '' }
}

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

// Fiche du streamer : chaîne (Discord, notes, commandes ignorées) et conditions habituelles
const streamerOpen = ref(false)
const streamerLogin = ref<string | null>(null)
const logChannel = ref<string | null>(null)
const logOpen = ref(false)
function openLog(login: string) {
  logChannel.value = login
  logOpen.value = true
}
function openStreamer(c: GiveawayChannelView) {
  streamerLogin.value = c.login
  streamerOpen.value = true
}

const toRemove = ref<GiveawayChannelView | null>(null)
async function confirmRemove() {
  if (!toRemove.value) return
  await invoke('remove_giveaway_channel', { id: toRemove.value.id })
  toRemove.value = null
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <div class="flex items-center gap-3">
      <h1 class="text-xl font-semibold flex-1">Chaînes surveillées</h1>
      <UButton label="Ajouter mes chaînes Points et StreamElements" icon="i-lucide-list-plus" size="sm" color="neutral" variant="soft" @click="importChannels" />
    </div>

    <UAlert
      v-if="!hasCategories"
      icon="i-lucide-info"
      color="info"
      variant="subtle"
      title="Aucune catégorie choisie : les giveaways sont surveillés quel que soit le jeu."
      :actions="[{ label: 'Choisir des catégories', to: '/giveaways/categories' }]"
    />

    <div class="flex items-center gap-2 flex-wrap">
      <USelectMenu
        v-model="categoryFilter"
        :items="categoryItems"
        value-key="value"
        multiple
        icon="i-lucide-gamepad-2"
        placeholder="Toutes les catégories"
        :search-input="{ placeholder: 'Rechercher une catégorie' }"
        class="w-72"
      >
        <template #item-leading="{ item }">
          <img v-if="item.gameId" :src="boxArt(null, item.gameId)" alt="" class="w-4 h-5 rounded-sm object-cover">
        </template>
      </USelectMenu>
      <USelectMenu
        v-model="giftFilter"
        :items="giftItems"
        value-key="value"
        multiple
        icon="i-lucide-gift"
        placeholder="Tous les cadeaux"
        :search-input="{ placeholder: 'Rechercher un cadeau' }"
        class="w-72"
      >
        <template #item-leading="{ item }">
          <img v-if="item.image" :src="item.image" referrerpolicy="no-referrer" alt="" class="size-5 rounded object-contain bg-elevated">
          <UIcon v-else name="i-lucide-gift" class="size-5 text-muted" />
        </template>
      </USelectMenu>
      <UButton v-if="filtering" label="Effacer les filtres" icon="i-lucide-x" size="sm" color="neutral" variant="ghost" @click="clearFilters" />
      <span v-if="filtering" class="text-sm text-muted">{{ channels.length }} chaîne(s) sur {{ allChannels.length }}</span>
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
            <th class="text-left px-3 py-2">État</th>
            <th class="text-left px-3 py-2">Titre du live</th>
            <th class="text-left px-3 py-2">Giveaway</th>
            <th class="text-left px-3 py-2">Cadeaux</th>
            <th class="text-left px-3 py-2">Chat lu par</th>
            <th class="px-3 py-2" />
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="c in channels"
            :key="c.id"
            class="border-t border-default"
            :class="[c.live && !c.inCategory ? 'opacity-70' : '', interesting(c) ? 'bg-primary/5 shadow-[inset_3px_0_0_var(--ui-primary)]' : '']"
          >
            <td class="px-3 py-2">
              <div class="flex items-center gap-1.5 whitespace-nowrap">
                <ChannelFlag :login="c.login" />
                <button class="font-medium hover:underline" @click="openExternal(channelUrl(c.login))">{{ c.login }}</button>
              </div>
              <div v-if="c.live" class="flex items-center gap-1.5 text-xs text-muted min-w-0">
                <span class="truncate max-w-[220px]" :title="c.live.gameName ?? 'Sans catégorie'">{{ c.live.gameName ?? 'Sans catégorie' }}</span>
                <span class="inline-flex items-center gap-0.5 whitespace-nowrap text-error" :title="`${formatNumber(c.live.viewers)} spectateurs`">
                  <UIcon name="i-lucide-eye" class="size-3.5" />{{ formatNumber(c.live.viewers) }}
                </span>
              </div>
            </td>
            <td class="px-3 py-2">
              <UTooltip :text="status(c).hint" :disabled="!status(c).hint">
                <UBadge :label="status(c).label" :color="status(c).color" variant="subtle" class="whitespace-nowrap" />
              </UTooltip>
            </td>
            <td class="px-3 py-2 min-w-64 max-w-md">
              <span v-if="c.live" class="line-clamp-2" :title="c.live.title">{{ c.live.title }}</span>
              <span v-else class="text-muted">—</span>
            </td>
            <td class="px-3 py-2 whitespace-nowrap">
              <template v-if="currentKind(c)">
                <div class="flex items-center gap-1.5">
                  <UBadge :label="GIVEAWAY_KIND[currentKind(c)!.kind].label" :color="GIVEAWAY_KIND[currentKind(c)!.kind].color" variant="subtle" />
                  <UTooltip v-if="!currentKind(c)!.added" text="Giveaway StreamElements actif, pas encore ajouté à tes giveaways">
                    <UIcon name="i-lucide-sparkles" class="size-4 text-primary" />
                  </UTooltip>
                </div>
                <div v-if="currentKind(c)!.command" class="font-mono text-xs mt-0.5">{{ currentKind(c)!.command }}</div>
              </template>
              <span v-else-if="c.live && !insightOf(c)" class="text-muted text-xs">Vérification…</span>
              <span v-else class="text-muted">—</span>
            </td>
            <td class="px-3 py-2">
              <div class="flex flex-wrap items-center gap-1 min-w-36 max-w-80">
                <UTooltip v-for="x in channelGifts(c)" :key="x.gift.id" :text="`${x.gift.title} (${x.attached ? 'rattaché au giveaway' : 'repéré'}${x.gift.favorite ? ', favori' : ''})`">
                  <span class="relative">
                    <img v-if="x.gift.image" :src="x.gift.image" referrerpolicy="no-referrer" alt="" class="size-8 rounded object-contain bg-elevated">
                    <span v-else class="size-8 rounded bg-elevated flex items-center justify-center"><UIcon name="i-lucide-gift" class="size-4 text-muted" /></span>
                    <UIcon v-if="!x.attached" name="i-lucide-sparkles" class="absolute -top-1 -right-1 size-3 text-primary" />
                    <SolidIcon v-if="x.gift.favorite" name="star" class="absolute -bottom-1 -right-1 size-3 text-warning" />
                  </span>
                </UTooltip>
                <span v-if="channelGifts(c).length === 0" class="text-muted">—</span>
              </div>
            </td>
            <td class="px-3 py-2">
              <UTooltip :text="c.reader === 'anonyme' ? 'Connexion anonyme commune (lecture seule)' : c.reader ? 'Connexion déjà présente pour StreamElements : pas de connexion en plus' : 'En cours de connexion'">
                <span class="inline-flex items-center gap-1.5">
                  <span class="size-2 rounded-full" :class="c.reader ? 'bg-success' : 'bg-neutral-400'" />
                  {{ c.reader ?? 'Connexion…' }}
                </span>
              </UTooltip>
            </td>
            <td class="px-3 py-2">
              <div class="flex justify-end items-center gap-1">
                <UButton v-if="c.live" icon="i-lucide-tv" size="xs" color="neutral" variant="ghost" :aria-label="`Regarder ${c.login}`" title="Regarder (fenêtre multi-stream)" @click="watchStream(c.login)" />
                <UButton
                  v-if="openGiveaway(c)"
                  label="Modifier le giveaway"
                  icon="i-lucide-pencil"
                  size="xs"
                  color="neutral"
                  variant="soft"
                  class="whitespace-nowrap"
                  @click="openForm(c)"
                />
                <UButton
                  v-else
                  label="Ajouter le giveaway"
                  icon="i-lucide-plus"
                  size="xs"
                  :variant="interesting(c) ? 'solid' : 'soft'"
                  class="whitespace-nowrap"
                  @click="openForm(c)"
                />
                <UButton icon="i-lucide-scroll-text" size="xs" variant="ghost" color="neutral" :aria-label="`Journal du chat de ${c.login}`" title="Journal du chat" @click="openLog(c.login)" />
                <UButton
                  icon="i-lucide-user-pen"
                  size="xs"
                  variant="ghost"
                  color="neutral"
                  :aria-label="`Fiche de ${c.login}`"
                  title="Fiche du streamer (Discord, notes, commandes ignorées, conditions habituelles)"
                  @click="openStreamer(c)"
                />
                <UButton icon="i-lucide-trash-2" size="xs" variant="ghost" color="error" aria-label="Ne plus surveiller" @click="toRemove = c" />
              </div>
            </td>
          </tr>
          <tr v-if="channels.length === 0">
            <td colspan="7" class="px-3 py-8 text-center text-muted">
              {{ filtering ? 'Aucune chaîne ne correspond aux filtres.' : 'Aucune chaîne surveillée. Ajoute la première avec le champ ci-dessus.' }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p class="text-xs text-muted max-w-[85ch]">
      Une chaîne où l'un de tes comptes est déjà présent pour StreamElements est lue via cette connexion. Les autres sont lues par une seule connexion anonyme commune, qui n'apparaît pour aucun de tes comptes.
      Une commande ignorée (ex. <span class="font-mono">!points</span>, dans la fiche du streamer) ne déclenche jamais d'alerte. Quand une chaîne passe hors ligne, ses giveaways ouverts sont clos.
    </p>

    <GiveawayFormModal v-model:open="formOpen" :giveaway="editing" :channel="formChannel" />
    <GiveawayFormModal v-model:open="streamerOpen" :giveaway="null" :channel="streamerLogin" streamer-only />
    <ChatLogModal v-model:open="logOpen" :channel="logChannel" />

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
