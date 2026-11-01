import { invoke } from '@tauri-apps/api/core'

// Ajoute un stream à la fenêtre multi-stream (ouverte au besoin) ; à appeler dans setup
export function useWatchStream() {
  const toast = useToast()
  return async (login: string) => {
    try {
      await invoke('watch_stream', { login })
    } catch (e) {
      toast.add({ title: 'Stream non ouvert', description: String(e), color: 'error' })
    }
  }
}
