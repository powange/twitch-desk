import { ref } from 'vue'

// Cadeau affiché en grand (vignette cliquée dans Alertes, Chaînes, Scanner) : une fenêtre
// commune, installée dans App.vue
const previewId = ref<number | null>(null)

export function useGiftPreview() {
  return {
    previewId,
    openGift: (id: number) => { previewId.value = id },
    closeGift: () => { previewId.value = null }
  }
}
