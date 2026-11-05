<script setup lang="ts">
import { computed } from 'vue'
import type { Gift } from '../types'
import GiftThumb from './GiftThumb.vue'
import SolidIcon from './SolidIcon.vue'
import { useGiftPreview } from '../composables/useGiftPreview'

// Vignette d'un cadeau dans une colonne Cadeaux : étoile si favori, étincelle s'il est seulement
// repéré (`attached` à false ; absent : pas de notion de rattachement), aperçu en grand au clic
const props = withDefaults(defineProps<{ gift: Gift, attached?: boolean, foundLabel?: string, lazy?: boolean }>(), {
  attached: undefined,
  foundLabel: 'repéré dans le live',
  lazy: false
})
const { openGift } = useGiftPreview()
const tooltip = computed(() => {
  const parts = [props.attached === true ? 'rattaché au giveaway' : props.attached === false ? props.foundLabel : null, props.gift.favorite ? 'favori' : null].filter(Boolean)
  return parts.length ? `${props.gift.title} (${parts.join(', ')})` : props.gift.title
})
</script>

<template>
  <UTooltip :text="tooltip">
    <button type="button" class="relative cursor-zoom-in rounded focus-visible:outline-2 focus-visible:outline-primary" :aria-label="`Voir ${gift.title} en grand`" @click="openGift(gift.id)">
      <GiftThumb :src="gift.image" class="size-8" :lazy="lazy" />
      <UIcon v-if="attached === false" name="i-lucide-sparkles" class="absolute -top-1 -right-1 size-3 text-primary" />
      <SolidIcon v-if="gift.favorite" name="star" class="absolute -bottom-1 -right-1 size-3 text-warning" />
    </button>
  </UTooltip>
</template>
