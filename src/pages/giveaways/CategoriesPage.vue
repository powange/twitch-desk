<script setup lang="ts">
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from '../../composables/useSnapshot'
import { useOpenExternal } from '../../utils/external'
import { boxArt } from '../../utils/format'
import type { GiveawayGame } from '../../types'
import CategorySearchInput from '../../components/CategorySearchInput.vue'

const { snapshot } = useSnapshot()
const toast = useToast()
const openExternal = useOpenExternal()

const games = computed(() => snapshot.value?.giveaways.games ?? [])
// Chaînes surveillées en live, par catégorie
const liveCount = computed(() => {
  const counts = new Map<string, number>()
  for (const c of snapshot.value?.giveaways.channels ?? []) {
    const id = c.live?.gameId
    if (id) counts.set(id, (counts.get(id) ?? 0) + 1)
  }
  return counts
})

const input = ref('')
const adding = ref(false)
async function addGame(value = input.value) {
  if (!value.trim()) return
  adding.value = true
  try {
    const name = await invoke<string>('add_giveaway_game', { input: value })
    input.value = ''
    toast.add({ title: `${name} ajoutée`, color: 'success', icon: 'i-lucide-check' })
  } catch (e) {
    toast.add({ title: 'Catégorie non ajoutée', description: String(e), color: 'error' })
  } finally {
    adding.value = false
  }
}

async function remove(game: GiveawayGame) {
  await invoke('remove_giveaway_game', { id: game.id })
}

function categoryUrl(slug: string) {
  return `https://www.twitch.tv/directory/category/${slug}`
}
</script>

<template>
  <div class="p-6 flex flex-col gap-5">
    <h1 class="text-xl font-semibold">Catégories surveillées</h1>

    <UAlert
      icon="i-lucide-info"
      color="info"
      variant="subtle"
      title="Les giveaways ne sont détectés que sur les chaînes en live dans ces catégories."
      description="Une chaîne en live dans une autre catégorie passe en bas de la liste des chaînes en live et ne déclenche pas d'alerte. Sans aucune catégorie, tout est surveillé."
    />

    <div class="flex gap-2">
      <CategorySearchInput id="new-giveaway-game" v-model="input" placeholder="Rechercher une catégorie (ex. Sea of Thieves) ou coller son adresse Twitch" :loading="adding" @submit="addGame" />
      <UButton label="Ajouter" :loading="adding" :disabled="!input.trim()" @click="addGame()" />
    </div>

    <div v-if="games.length" class="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-3">
      <div v-for="game in games" :key="game.id" class="border border-default rounded-lg p-3 flex items-center gap-3">
        <img :src="boxArt(game.boxArt, game.gameId)" alt="" class="w-10 h-14 rounded object-cover bg-elevated shrink-0">
        <div class="flex-1 min-w-0">
          <button class="font-medium hover:underline text-left truncate block max-w-full" @click="openExternal(categoryUrl(game.slug))">{{ game.name }}</button>
          <div class="text-xs text-muted">{{ liveCount.get(game.gameId) ?? 0 }} chaîne(s) en live</div>
        </div>
        <UButton icon="i-lucide-x" size="xs" color="neutral" variant="ghost" :aria-label="`Retirer ${game.name}`" @click="remove(game)" />
      </div>
    </div>
    <p v-else class="text-sm text-muted">Aucune catégorie : les giveaways sont surveillés quel que soit le jeu.</p>
  </div>
</template>
