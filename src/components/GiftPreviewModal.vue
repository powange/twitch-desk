<script setup lang="ts">
import { computed } from 'vue'
import GiftThumb from './GiftThumb.vue'
import { useGiftCatalog } from '../composables/useGiftCatalog'
import { useGiftPreview } from '../composables/useGiftPreview'
import { boxArt } from '../utils/format'
import SolidIcon from './SolidIcon.vue'

const { byId, groups } = useGiftCatalog()
const { previewId, closeGift } = useGiftPreview()

const gift = computed(() => (previewId.value === null ? null : byId.value.get(previewId.value) ?? null))
const group = computed(() => groups.value.find(g => g.id === gift.value?.groupId) ?? null)
</script>

<template>
  <UModal :open="gift !== null" :title="gift?.title ?? ''" :ui="{ content: 'max-w-xl' }" @update:open="v => { if (!v) closeGift() }">
    <template #body>
      <div v-if="gift" class="flex flex-col gap-4">
        <div class="bg-elevated rounded-lg flex items-center justify-center min-h-48">
          <GiftThumb :src="gift.image" class="max-h-[60vh] w-full min-h-48 rounded-lg" icon="size-16" />
        </div>
        <dl class="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm items-center">
          <dt class="text-muted">Catégorie</dt>
          <dd class="flex items-center gap-2">
            <template v-if="gift.gameId">
              <img :src="boxArt(null, gift.gameId)" alt="" class="w-5 h-7 rounded-sm object-cover">
              {{ gift.gameName ?? gift.gameId }}
            </template>
            <span v-else class="text-muted">Sans catégorie</span>
          </dd>
          <dt class="text-muted">Groupe</dt>
          <dd class="flex items-center gap-2">
            <template v-if="group">
              <UIcon name="i-lucide-folder" class="size-4 text-muted" />{{ group.name }}
              <UBadge v-if="!group.gameId" label="tous les jeux" size="sm" color="neutral" variant="subtle" />
            </template>
            <span v-else class="text-muted">Sans groupe</span>
          </dd>
          <template v-if="gift.favorite">
            <dt class="text-muted">Favori</dt>
            <dd class="flex items-center gap-1.5 text-warning"><SolidIcon name="star" class="size-4" />Cadeau recherché</dd>
          </template>
        </dl>
      </div>
    </template>
  </UModal>
</template>
