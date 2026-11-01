import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import ui from '@nuxt/ui/vite'

// Port fixe : tauri.conf.json (devUrl) pointe dessus
export default defineConfig({
  plugins: [
    vue(),
    ui({
      ui: { colors: { primary: 'violet', neutral: 'zinc' } },
      // Icônes embarquées : l'app fonctionne hors ligne
      icon: { clientBundle: { scan: true } }
    })
  ],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**'] } },
  build: { target: 'es2022' }
})
