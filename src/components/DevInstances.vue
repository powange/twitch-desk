<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { timeAgo } from '../utils/format'

// Applications connues du serveur de synchronisation : ouvertes ou fermées, comptes Twitch,
// web apps associées et ouvertes
interface Instance { id: string, name: string, online: boolean, lastSeen: string, accounts: string[], version: string | null, devices: number, devicesOnline: number, current: boolean }
const list = ref<Instance[] | null>(null)
const error = ref<string | null>(null)
const loading = ref(false)
const online = computed(() => (list.value ?? []).filter(i => i.online).length)

async function load() {
  loading.value = true
  try {
    list.value = await invoke<Instance[]>('sync_instances')
    error.value = null
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
}
onMounted(load)
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex items-center gap-3">
      <div class="text-sm">
        <template v-if="list">
          <b>{{ list.length }}</b> application(s) connue(s) du serveur, <b>{{ online }}</b> ouverte(s)
        </template>
      </div>
      <span class="flex-1" />
      <UButton label="Actualiser" icon="i-lucide-refresh-cw" size="sm" color="neutral" variant="soft" :loading="loading" @click="load" />
    </div>
    <UAlert v-if="error" icon="i-lucide-triangle-alert" color="warning" variant="subtle" :title="error" />
    <div v-if="list?.length" class="overflow-x-auto border border-default rounded-lg">
      <table class="text-sm w-full">
        <thead class="text-xs uppercase text-muted bg-elevated">
          <tr>
            <th class="text-left px-3 py-2 font-medium">Application</th>
            <th class="text-left px-3 py-2 font-medium">État</th>
            <th class="text-left px-3 py-2 font-medium">Version</th>
            <th class="text-left px-3 py-2 font-medium">Dernière activité</th>
            <th class="text-left px-3 py-2 font-medium">Comptes Twitch</th>
            <th class="text-right px-3 py-2 font-medium">Web apps associées</th>
            <th class="text-right px-3 py-2 font-medium">Web apps ouvertes</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="i in list" :key="i.id" class="border-t border-default">
            <td class="px-3 py-2">
              <div class="font-medium">{{ i.name }} <UBadge v-if="i.current" label="ce PC" size="sm" color="primary" variant="subtle" class="ml-1" /></div>
              <div class="text-xs text-muted font-mono">{{ i.id }}</div>
            </td>
            <td class="px-3 py-2 whitespace-nowrap">
              <span class="inline-flex items-center gap-1.5">
                <span class="size-2 rounded-full" :class="i.online ? 'bg-success' : 'bg-muted'" />
                {{ i.online ? 'Ouverte' : 'Fermée' }}
              </span>
            </td>
            <td class="px-3 py-2 whitespace-nowrap tabular-nums">
              <template v-if="i.version">v{{ i.version }}</template>
              <span v-else class="text-muted">—</span>
            </td>
            <td class="px-3 py-2 whitespace-nowrap text-muted">{{ timeAgo(i.lastSeen) }}</td>
            <td class="px-3 py-2">{{ i.accounts.length ? i.accounts.join(', ') : '—' }}</td>
            <td class="px-3 py-2 text-right tabular-nums">{{ i.devices }}</td>
            <td class="px-3 py-2 text-right tabular-nums">{{ i.devicesOnline }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <p class="text-xs text-muted">« Ouverte » : son flux temps réel est connecté au serveur en ce moment (serveur 0.8.8 et application 0.48.35 ou plus). Comptes et version : ceux du dernier état publié par l'application (version : serveur 0.8.9 ou plus).</p>
  </div>
</template>
