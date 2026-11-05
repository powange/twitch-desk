import { ref } from 'vue'

// Menu développeur : débloqué par 14 clics rapides sur le numéro de version, pour la session
// seulement (rien n'est gardé : à refaire après avoir fermé l'application)
const CLICKS = 14
const WINDOW_MS = 6000
const unlocked = ref(false)
let clicks: number[] = []

export function useDevMode() {
  // Vrai au clic qui débloque le menu
  function tap(): boolean {
    if (unlocked.value) return false
    const now = Date.now()
    clicks = [...clicks.filter(t => now - t < WINDOW_MS), now]
    if (clicks.length < CLICKS) return false
    clicks = []
    unlocked.value = true
    return true
  }
  return { unlocked, tap }
}
