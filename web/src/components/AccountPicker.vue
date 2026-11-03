<script setup lang="ts">
// Compte affiché (pages Points de chaîne et StreamElements) ; masqué s'il n'y en a qu'un
defineProps<{ accounts: string[] }>()
const model = defineModel<string>({ required: true })
</script>

<template>
  <div v-if="accounts.length > 1" class="flex gap-1.5 overflow-x-auto -mx-4 px-4 pb-1" role="tablist" aria-label="Compte">
    <UButton
      v-for="a in accounts"
      :key="a"
      :label="a"
      size="sm"
      role="tab"
      :aria-selected="model === a"
      :color="model === a ? 'primary' : 'neutral'"
      :variant="model === a ? 'solid' : 'soft'"
      class="shrink-0"
      @click="model = a"
    />
  </div>
</template>
