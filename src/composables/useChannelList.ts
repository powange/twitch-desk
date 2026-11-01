import { computed, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useSelectedAccount } from './useSelectedAccount'
import type { ChannelList, ChannelView } from '../types'

// Une liste de chaînes (page Points de chaîne ou StreamElements) : compte
// affiché, chaînes de cette liste, ajout et retrait. Les deux listes sont
// indépendantes : retirer une chaîne d'une page ne la retire pas de l'autre.
export function useChannelList(list: ChannelList) {
  const { accounts, accountId, account, accountItems } = useSelectedAccount()
  const toast = useToast()
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

  // Options d'une chaîne : les changements pas encore visibles dans l'état du cœur sont gardés
  // ici (sinon deux clics rapprochés s'annulaient, le second relisant l'ancien état), et les
  // envois d'une même chaîne partent l'un après l'autre avec l'état local le plus récent
  type Options = Pick<ChannelView, 'enabled' | 'autoClaim' | 'chatPresence'>
  const overrides = reactive(new Map<number, Partial<Options>>())
  const queues = new Map<number, Promise<void>>()
  const inFlight = new Map<number, number>()
  const findChannel = (id: number) => account.value?.channels.find(c => c.id === id)
  function optionsOf(channel: ChannelView): Options {
    const o = overrides.get(channel.id)
    return {
      enabled: o?.enabled ?? channel.enabled,
      autoClaim: o?.autoClaim ?? channel.autoClaim,
      chatPresence: o?.chatPresence ?? channel.chatPresence
    }
  }
  // Changement local retiré dès que le cœur le reflète (et que plus rien n'est en cours d'envoi)
  function prune(id: number) {
    if (inFlight.get(id)) return
    const o = overrides.get(id)
    const c = findChannel(id)
    if (!o) return
    if (!c || (Object.keys(o) as (keyof Options)[]).every(k => o[k] === c[k])) overrides.delete(id)
  }
  watch(() => account.value?.channels, () => { for (const id of [...overrides.keys()]) prune(id) })

  function setOptions(channel: ChannelView, patch: Partial<Options>) {
    const id = channel.id
    overrides.set(id, { ...overrides.get(id), ...patch })
    inFlight.set(id, (inFlight.get(id) ?? 0) + 1)
    const send = async () => {
      const c = findChannel(id) ?? channel
      try {
        await invoke('set_channel_options', { id, ...optionsOf(c) })
      } catch (e) {
        // Retour à l'état du cœur
        overrides.delete(id)
        toast.add({ title: 'Option non enregistrée', description: String(e), color: 'error' })
      } finally {
        inFlight.set(id, (inFlight.get(id) ?? 1) - 1)
        prune(id)
      }
    }
    const next = (queues.get(id) ?? Promise.resolve()).then(send)
    queues.set(id, next)
    return next
  }

  const toRemove = ref<ChannelView | null>(null)
  const removing = ref(false)
  async function confirmRemove() {
    const c = toRemove.value
    if (!c || removing.value) return
    removing.value = true
    try {
      await invoke('remove_channel', { id: c.id, list })
      toRemove.value = null
    } catch (e) {
      toast.add({ title: 'Chaîne non retirée', description: String(e), color: 'error' })
    } finally {
      removing.value = false
    }
  }

  return { accounts, accountId, account, accountItems, channels, newLogin, adding, addChannel, setOptions, optionsOf, toRemove, removing, confirmRemove }
}
