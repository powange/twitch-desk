<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useGiftCatalog, useKnownCategories } from '../composables/useGiftCatalog'
import GiftGroupsModal from './GiftGroupsModal.vue'
import { boxArt } from '../utils/format'
import { firstImage, imageToDataUrl } from '../utils/images'
import type { SelectItem } from '@nuxt/ui'
import type { Gift, TwitchGame } from '../types'

// Ajout (gift = null) ou modification d'un cadeau du catalogue
const props = defineProps<{ gift: Gift | null }>()
const open = defineModel<boolean>('open', { required: true })
const emit = defineEmits<{ saved: [id: number] }>()

const { groups, refresh } = useGiftCatalog()
const toast = useToast()
const image = ref<string | null>(null)

// --- Catégorie Twitch (jeu) ---
const gameId = ref<string | null>(null)
const gameName = ref<string | null>(null)
const categoryInput = ref('')
const resolving = ref(false)
const knownCategories = useKnownCategories()
async function chooseCategory() {
  const value = categoryInput.value.trim()
  if (!value) return
  const known = knownCategories.value.find(c => c.name.toLowerCase() === value.toLowerCase())
  if (known) {
    gameId.value = known.id
    gameName.value = known.name
    categoryInput.value = ''
    return
  }
  resolving.value = true
  try {
    const game = await invoke<TwitchGame>('resolve_category', { input: value })
    gameId.value = game.id
    gameName.value = game.name
    categoryInput.value = ''
  } catch (e) {
    toast.add({ title: 'Catégorie introuvable', description: String(e), color: 'error' })
  } finally {
    resolving.value = false
  }
}
// Liste des catégories connues sous le champ : jaquette et nom, filtrée par la saisie
const categoryFocused = ref(false)
const highlighted = ref(0)
const categoryMatches = computed(() => {
  const q = categoryInput.value.trim().toLowerCase()
  return knownCategories.value.filter(c => !q || c.name.toLowerCase().includes(q)).slice(0, 8)
})
watch(categoryMatches, () => { highlighted.value = 0 })
function pickCategory(c: { id: string, name: string }) {
  gameId.value = c.id
  gameName.value = c.name
  categoryInput.value = ''
  categoryFocused.value = false
}
function onCategoryKey(e: KeyboardEvent) {
  const list = categoryMatches.value
  if (e.key === 'ArrowDown' && list.length) {
    e.preventDefault()
    highlighted.value = (highlighted.value + 1) % list.length
  } else if (e.key === 'ArrowUp' && list.length) {
    e.preventDefault()
    highlighted.value = (highlighted.value - 1 + list.length) % list.length
  } else if (e.key === 'Enter') {
    e.preventDefault()
    const c = categoryFocused.value ? list[highlighted.value] : undefined
    // Un nom tapé qui n'est pas dans la liste : recherche sur Twitch
    if (c && (!categoryInput.value.trim() || c.name.toLowerCase().includes(categoryInput.value.trim().toLowerCase()))) pickCategory(c)
    else void chooseCategory()
  } else if (e.key === 'Escape' && categoryFocused.value) {
    e.stopPropagation()
    categoryFocused.value = false
  }
}
function onCategoryBlur() {
  // Laisser le clic sur une ligne de la liste aboutir
  setTimeout(() => { categoryFocused.value = false }, 150)
}

function clearCategory() {
  gameId.value = null
  gameName.value = null
}
// Groupe : un groupe de la catégorie choisie, ou un groupe sans catégorie (0 : aucun)
const NO_GROUP = 0
const group = ref<number>(NO_GROUP)
const groupItems = computed(() => {
  const own = groups.value.filter(g => gameId.value && g.gameId === gameId.value)
  const loose = groups.value.filter(g => !g.gameId)
  const items: SelectItem[] = [{ label: 'Aucun groupe', value: NO_GROUP }]
  if (own.length) items.push({ type: 'separator' }, { type: 'label', label: gameName.value ?? '' }, ...own.map(g => ({ label: g.name, value: g.id })))
  if (loose.length) items.push({ type: 'separator' }, { type: 'label', label: 'Sans catégorie' }, ...loose.map(g => ({ label: g.name, value: g.id })))
  return items
})
// Catégorie changée : un groupe d'une autre catégorie ne convient plus
watch(gameId, (id) => {
  const g = groups.value.find(x => x.id === group.value)
  if (g?.gameId && g.gameId !== id) group.value = NO_GROUP
})
const groupsOpen = ref(false)
function onGroupCreated(id: number) {
  group.value = id
}

// --- Image : téléversée ou en ligne ---
// L'adresse saisie est l'image du cadeau : aperçu immédiat, enregistrée avec le cadeau
const imageUrl = ref('')
const imageBroken = ref(false)
watch(imageUrl, (value) => {
  const url = value.trim()
  if (url.startsWith('https://')) {
    image.value = url
    imageBroken.value = false
  }
})
watch(image, () => { imageBroken.value = false })
function removeImage() {
  image.value = null
  imageUrl.value = ''
}

const title = ref('')
const keywords = ref<string[]>([])
const keywordInput = ref('')
const saving = ref(false)
const dragging = ref(false)

watch(open, (isOpen) => {
  if (!isOpen) return
  title.value = props.gift?.title ?? ''
  group.value = props.gift?.groupId ?? NO_GROUP
  keywords.value = [...(props.gift?.keywords ?? [])]
  keywordInput.value = ''
  image.value = props.gift?.image ?? null
  // Image en ligne : son adresse s'affiche dans le champ
  imageUrl.value = props.gift?.image?.startsWith('https://') ? props.gift.image : ''
  gameId.value = props.gift?.gameId ?? null
  gameName.value = props.gift?.gameName ?? null
  categoryInput.value = ''
})

function addKeyword() {
  for (const raw of keywordInput.value.split(',')) {
    const k = raw.trim().toLowerCase()
    if (k && !keywords.value.includes(k)) keywords.value.push(k)
  }
  keywordInput.value = ''
}

async function useFile(file: File | null) {
  if (!file) return
  try {
    image.value = await imageToDataUrl(file)
    imageUrl.value = ''
  } catch (e) {
    toast.add({ title: 'Image non utilisable', description: String(e), color: 'error' })
  }
}

function onPaste(e: ClipboardEvent) {
  const file = firstImage(e.clipboardData?.items)
  if (file) {
    e.preventDefault()
    void useFile(file)
  }
}

function onDrop(e: DragEvent) {
  dragging.value = false
  void useFile(firstImage(e.dataTransfer?.files))
}

const fileInput = ref<HTMLInputElement | null>(null)

async function save() {
  if (keywordInput.value.trim()) addKeyword()
  saving.value = true
  try {
    if (categoryInput.value.trim()) await chooseCategory()
    const url = imageUrl.value.trim()
    if (url && !url.startsWith('https://')) {
      toast.add({ title: 'Adresse d\'image invalide', description: 'Elle doit commencer par https://', color: 'error' })
      return
    }
    const gift: Gift = {
      id: props.gift?.id ?? 0,
      title: title.value,
      image: image.value,
      groupId: group.value || null,
      keywords: keywords.value,
      gameId: gameId.value,
      gameName: gameName.value,
      // Ignoré à l'enregistrement : le favori se change depuis la page Cadeaux
      favorite: props.gift?.favorite ?? false
    }
    const id = await invoke<number>('save_gift', { gift })
    await refresh()
    toast.add({ title: props.gift ? 'Cadeau modifié' : 'Cadeau ajouté', color: 'success', icon: 'i-lucide-check' })
    emit('saved', id)
    open.value = false
  } catch (e) {
    toast.add({ title: 'Cadeau non enregistré', description: String(e), color: 'error' })
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <UModal v-model:open="open" :title="gift ? 'Modifier le cadeau' : 'Nouveau cadeau'">
    <template #body>
      <form id="gift-form" class="flex flex-col gap-4" @submit.prevent="save" @paste="onPaste">
        <div class="flex gap-4">
          <!-- Image : coller (Ctrl+V), glisser ou choisir un fichier -->
          <button
            type="button"
            class="size-32 shrink-0 rounded-lg border-2 border-dashed flex items-center justify-center overflow-hidden transition-colors"
            :class="dragging ? 'border-primary bg-primary/10' : 'border-default hover:border-primary/60'"
            title="Coller (Ctrl+V), glisser une image ou cliquer pour choisir"
            @click="fileInput?.click()"
            @dragover.prevent="dragging = true"
            @dragleave="dragging = false"
            @drop.prevent="onDrop"
          >
            <img v-if="image && !imageBroken" :src="image" alt="" referrerpolicy="no-referrer" class="w-full h-full object-contain" @error="imageBroken = true">
            <span v-else-if="image && imageBroken" class="flex flex-col items-center gap-1 text-xs text-error px-2 text-center">
              <UIcon name="i-lucide-image-off" class="size-6" />
              Image introuvable à cette adresse
            </span>
            <span v-else class="flex flex-col items-center gap-1 text-xs text-muted px-2 text-center">
              <UIcon name="i-lucide-image-plus" class="size-6" />
              Coller, glisser ou choisir une image
            </span>
          </button>
          <input ref="fileInput" type="file" accept="image/*" class="hidden" @change="e => useFile((e.target as HTMLInputElement).files?.[0] ?? null)">
          <div class="flex-1 flex flex-col gap-3">
            <UFormField label="Titre">
              <UInput id="gift-title" v-model="title" placeholder="Navire Obsidienne" class="w-full" autofocus />
            </UFormField>
            <UFormField label="Catégorie Twitch" help="Nom exact du jeu ou adresse de sa catégorie">
              <div v-if="gameId" class="flex items-center gap-2 border border-default rounded-md px-2 py-1">
                <img :src="boxArt(null, gameId)" alt="" class="w-6 h-8 rounded object-cover">
                <span class="flex-1 font-medium">{{ gameName }}</span>
                <UButton icon="i-lucide-x" size="xs" color="neutral" variant="ghost" aria-label="Retirer la catégorie" @click="clearCategory" />
              </div>
              <div v-else class="relative">
                <div class="flex gap-2">
                  <UInput
                    id="gift-category"
                    v-model="categoryInput"
                    placeholder="Sea of Thieves"
                    autocomplete="off"
                    class="flex-1"
                    role="combobox"
                    :aria-expanded="categoryFocused && categoryMatches.length > 0"
                    aria-controls="gift-category-list"
                    @focus="categoryFocused = true"
                    @blur="onCategoryBlur"
                    @keydown="onCategoryKey"
                  />
                  <UButton label="Valider" size="sm" color="neutral" variant="soft" :loading="resolving" @click="chooseCategory" />
                </div>
                <ul
                  v-if="categoryFocused && categoryMatches.length"
                  id="gift-category-list"
                  role="listbox"
                  class="absolute z-50 left-0 right-0 mt-1 max-h-64 overflow-y-auto rounded-md border border-default bg-default shadow-lg py-1"
                >
                  <li
                    v-for="(c, i) in categoryMatches"
                    :key="c.id"
                    role="option"
                    :aria-selected="i === highlighted"
                    class="flex items-center gap-2.5 px-2 py-1.5 cursor-pointer text-sm"
                    :class="i === highlighted ? 'bg-elevated' : ''"
                    @mouseenter="highlighted = i"
                    @mousedown.prevent="pickCategory(c)"
                  >
                    <img :src="boxArt(null, c.id)" alt="" class="w-7 h-10 rounded object-cover bg-elevated shrink-0">
                    <span class="font-medium">{{ c.name }}</span>
                  </li>
                </ul>
              </div>
            </UFormField>
            <UFormField label="Groupe" :help="gameId ? `Groupes de ${gameName} ou sans catégorie` : 'Groupes sans catégorie (choisis une catégorie pour voir les siens)'">
              <div class="flex gap-2">
                <USelect id="gift-group" v-model="group" :items="groupItems" class="flex-1" />
                <UButton icon="i-lucide-folder-cog" color="neutral" variant="soft" title="Gérer les groupes" aria-label="Gérer les groupes" @click="groupsOpen = true" />
              </div>
            </UFormField>
            <GiftGroupsModal v-model:open="groupsOpen" :game-id="gameId" :game-name="gameName" @created="onGroupCreated" />
          </div>
        </div>

        <UFormField label="Ou adresse d'une image en ligne" help="Adresse directe de l'image (clic droit sur l'image → Copier l'adresse de l'image)">
          <div class="flex gap-2">
            <UInput id="gift-image-url" v-model="imageUrl" placeholder="https://…/image.png" class="flex-1" @keydown.enter.prevent />
            <UButton v-if="image" label="Retirer l'image" icon="i-lucide-x" size="sm" color="neutral" variant="ghost" @click="removeImage" />
          </div>
        </UFormField>

        <UFormField label="Mots-clés" help="Le titre du cadeau compte aussi. Cherchés au début des mots (majuscules et accents ignorés) dans le titre du live, la description et le message épinglé de la chaîne, et le giveaway StreamElements">
          <div class="flex flex-wrap items-center gap-1.5">
            <UBadge v-for="k in keywords" :key="k" color="neutral" variant="subtle">
              {{ k }}
              <button type="button" class="ml-1 hover:text-error" :aria-label="`Retirer ${k}`" @click="keywords = keywords.filter(x => x !== k)">×</button>
            </UBadge>
            <UInput id="gift-keyword" v-model="keywordInput" placeholder="obsidienne, navire…" size="sm" class="w-48" @keydown.enter.prevent="addKeyword" @blur="addKeyword" />
          </div>
        </UFormField>
      </form>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton label="Annuler" color="neutral" variant="ghost" @click="open = false" />
        <UButton type="submit" form="gift-form" :label="gift ? 'Enregistrer' : 'Ajouter'" :loading="saving" />
      </div>
    </template>
  </UModal>
</template>
