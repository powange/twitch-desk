import { computed, ref, watch } from 'vue'
import { useSnapshot } from './useSnapshot'

// Compte affiché par les pages Points de chaîne, StreamElements et Drops : le même
// d'une page à l'autre, gardé d'une session à l'autre
const STORAGE_KEY = 'selectedAccount'
const accountId = ref<number | undefined>(readStored())

function readStored(): number | undefined {
  try {
    const v = Number(localStorage.getItem(STORAGE_KEY))
    return Number.isFinite(v) && v > 0 ? v : undefined
  } catch {
    return undefined
  }
}

watch(accountId, (id) => {
  try {
    if (id) localStorage.setItem(STORAGE_KEY, String(id))
  } catch {
    // Stockage indisponible : le choix vaut pour la session
  }
})

export function useSelectedAccount() {
  const { accounts } = useSnapshot()
  watch(accounts, (all) => {
    if (all.length && !all.some(a => a.id === accountId.value)) accountId.value = all[0]?.id
  }, { immediate: true })
  const account = computed(() => accounts.value.find(a => a.id === accountId.value))
  const accountItems = computed(() => accounts.value.map(a => ({ label: a.login, value: a.id })))
  return { accounts, accountId, account, accountItems }
}
