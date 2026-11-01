import { openUrl } from '@tauri-apps/plugin-opener'

// À appeler dans setup : renvoie une fonction qui ouvre une adresse dans le
// navigateur par défaut, et affiche l'échec au lieu de le laisser passer inaperçu
export function useOpenExternal() {
  const toast = useToast()
  return async (url: string) => {
    try {
      await openUrl(url)
    } catch (e) {
      toast.add({ title: 'Impossible d\'ouvrir le navigateur', description: `${url} : ${String(e)}`, color: 'error' })
    }
  }
}
