<script setup lang="ts">
import { useUpdater } from '../composables/useUpdater'

const { available, installing, progress, error, install } = useUpdater()
</script>

<template>
  <div v-if="available" class="border-b border-default bg-primary/10 px-5 py-2.5 flex items-center gap-3 text-sm">
    <UIcon name="i-lucide-download" class="text-primary size-5 shrink-0" />
    <div class="flex-1 min-w-0">
      <b>Version {{ available.version }} disponible.</b>
      <span v-if="available.body" class="text-muted ml-1 truncate">{{ available.body }}</span>
      <span v-if="error" class="text-error ml-1">Échec : {{ error }}</span>
    </div>
    <UProgress v-if="installing" v-model="progress" class="w-40" />
    <UButton
      v-else
      label="Installer et redémarrer"
      size="sm"
      @click="install"
    />
  </div>
</template>
