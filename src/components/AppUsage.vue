<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

// Ressources de l'application (fenêtre des streams comprise), relevées toutes les 2 s par le cœur
interface AppUsage { cpu: number, memory: number, processes: number, down: number | null, up: number | null }
const usage = ref<AppUsage | null>(null)

let unlisten: UnlistenFn | null = null
onMounted(async () => {
  unlisten = await listen<AppUsage>('app-usage', (e) => { usage.value = e.payload })
})
onBeforeUnmount(() => unlisten?.())

const number = (n: number, digits: number) => n.toLocaleString('fr-FR', { maximumFractionDigits: digits })
function bytes(n: number) {
  if (n >= 1024 ** 3) return `${number(n / 1024 ** 3, 2)} Go`
  if (n >= 1024 ** 2) return `${number(n / 1024 ** 2, n >= 100 * 1024 ** 2 ? 0 : 1)} Mo`
  return `${number(n / 1024, 0)} Ko`
}
const rate = (n: number) => `${bytes(n)}/s`
</script>

<template>
  <div v-if="usage" class="flex flex-col gap-0.5 tabular-nums">
    <UTooltip :text="`Twitch Desk et ses ${usage.processes} processus (fenêtre des streams comprise)`">
      <div class="flex items-center gap-1.5" tabindex="0">
        <UIcon name="i-lucide-cpu" class="size-3.5 shrink-0" />
        <span>Processeur {{ number(usage.cpu, 1) }} %</span>
      </div>
    </UTooltip>
    <div class="flex items-center gap-1.5">
      <UIcon name="i-lucide-memory-stick" class="size-3.5 shrink-0" />
      <span>Mémoire {{ bytes(usage.memory) }}</span>
    </div>
    <UTooltip v-if="usage.down !== null && usage.up !== null" text="Estimation : octets lus et écrits par l'application, vidéo des streams comprise (peut inclure un peu d'accès disque)">
      <div class="flex items-center gap-3" tabindex="0">
        <span class="flex items-center gap-1"><UIcon name="i-lucide-arrow-down" class="size-3.5 shrink-0" aria-label="Descendant" />{{ rate(usage.down) }}</span>
        <span class="flex items-center gap-1"><UIcon name="i-lucide-arrow-up" class="size-3.5 shrink-0" aria-label="Montant" />{{ rate(usage.up) }}</span>
      </div>
    </UTooltip>
  </div>
</template>
