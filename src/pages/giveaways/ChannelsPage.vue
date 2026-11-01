<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../../composables/useSnapshot'
import { useOpenExternal } from '../../utils/external'
import { channelUrl, formatNumber } from '../../utils/format'
import type { GiveawayChannelView } from '../../types'
import ChannelFlag from '../../components/ChannelFlag.vue'

const { snapshot } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()

const view = computed(() => snapshot.value?.giveaways)
const hasCategories = computed(() => (view.value?.games.length ?? 0) > 0)

// En live dans une catégorie choisie, puis en live ailleurs, puis hors ligne ; plus regardées d'abord
function rank(c: GiveawayChannelView): number {
  if (c.live && c.inCategory) return 0
  if (c.live) return 1
  return c.liveChecked ? 2 : 3
}
const channels = computed(() =>
  [...(view.value?.channels ?? [])].sort((a, b) =>
    rank(a) - rank(b) || (b.live?.viewers ?? 0) - (a.live?.viewers ?? 0) || a.login.localeCompare(b.login)
  )
)

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
            <th class="text-left px-3 py-2">Chat lu par</th>
            <th class="text-left px-3 py-2">Commandes ignorées</th>
            <th class="px-3 py-2" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="c in channels" :key="c.id" class="border-t border-default" :class="c.live && !c.inCategory ? 'opacity-70' : ''">
            <td class="px-3 py-2 min-w-0">
              <div class="flex items-center gap-1.5">
                <ChannelFlag :login="c.login" />
                <button class="font-medium hover:underline" @click="openExternal(channelUrl(c.login))">{{ c.login }}</button>
              </div>
              <div v-if="c.live" class="flex items-center gap-1.5 text-xs text-muted">
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
            <td colspan="5" class="px-3 py-8 text-center text-muted">Aucune chaîne surveillée. Ajoute la première avec le champ ci-dessus.</td>
          </tr>
        </tbody>
      </table>
    </div>
    <p class="text-xs text-muted max-w-[85ch]">
      Une chaîne où l'un de tes comptes est déjà présent pour StreamElements est lue via cette connexion. Les autres sont lues par une seule connexion anonyme commune, qui n'apparaît pour aucun de tes comptes.
      Une commande ignorée (ex. <span class="font-mono">!points</span>) ne déclenche jamais d'alerte. Quand une chaîne passe hors ligne, ses giveaways ouverts sont clos.
    </p>

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
