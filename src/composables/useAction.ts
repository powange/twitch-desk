import { reactive } from 'vue'

// Action asynchrone d'un bouton : clé « en cours » (pour :loading / :disabled) et message
// d'erreur en cas d'échec. run() renvoie true si l'action a réussi (pour ne fermer une
// fenêtre de confirmation qu'en cas de succès).
export function useAction() {
  const toast = useToast()
  const running = reactive(new Set<string>())

  async function run(key: string, errorTitle: string, action: () => Promise<unknown>): Promise<boolean> {
    // Déjà en cours : un double clic ne relance pas l'action
    if (running.has(key)) return false
    running.add(key)
    try {
      await action()
      return true
    } catch (e) {
      toast.add({ title: errorTitle, description: String(e), color: 'error' })
      return false
    } finally {
      running.delete(key)
    }
  }

  const busy = (key: string) => running.has(key)
  return { run, busy }
}
