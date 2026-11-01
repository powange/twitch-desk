import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSnapshot } from './useSnapshot'
import type { ChannelList, ChannelView } from '../types'

// Une liste de chaînes (page Points de chaîne ou StreamElements) : compte
// affiché, chaînes de cette liste, ajout et retrait. Les deux listes sont
// indépendantes : retirer une chaîne d'une page ne la retire pas de l'autre.
export function useChannelList(list: ChannelList) {
  const { accounts } = useSnapshot()
  const toast = useToast()

  const accountId = ref<number | undefined>()
  watch(accounts, (all) => {
    if (!all.some(a => a.id === accountId.value)) accountId.value = all[0]?.id
  }, { immediate: true })

  const account = computed(() => accounts.value.find(a => a.id === accountId.value))
  const accountItems = computed(() => accounts.value.map(a => ({ label: a.login, value: a.id })))
  const channels = computed(() =>
    (account.value?.channels ?? []).filter(c => list === 'points' ? c.inPoints : c.inSe)
  )

  const newLogin = ref('')
  const adding = ref(false)

  async function addChannel() {
    if (!account.value || !newLogin.value.trim()) return
    adding.value = true
    try {
      const login = await invoke<string>('add_channel', { accountId: account.value.id, login: newLogin.value, list })
      newLogin.value = ''
      toast.add({ title: `${login} ajoutée`, color: 'success', icon: 'i-lucide-check' })
    } catch (e) {
      toast.add({ title: 'Chaîne non ajoutée', description: String(e), color: 'error' })
    } finally {
      adding.value = false
    }
  }

  async function setOptions(channel: ChannelView, patch: Partial<Pick<ChannelView, 'enabled' | 'autoClaim' | 'chatPresence'>>) {
    await invoke('set_channel_options', {
      id: channel.id,
      enabled: patch.enabled ?? channel.enabled,
      autoClaim: patch.autoClaim ?? channel.autoClaim,
      chatPresence: patch.chatPresence ?? channel.chatPresence
    })
  }

  const toRemove = ref<ChannelView | null>(null)
  async function confirmRemove() {
    if (!toRemove.value) return
    await invoke('remove_channel', { id: toRemove.value.id, list })
    toRemove.value = null
  }

  return { accounts, accountId, account, accountItems, channels, newLogin, adding, addChannel, setOptions, toRemove, confirmRemove }
}
