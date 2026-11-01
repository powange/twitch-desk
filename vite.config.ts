import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import ui from '@nuxt/ui/vite'
import { FLAG_ICONS } from './src/utils/languages'

// Port fixe : tauri.conf.json (devUrl) pointe dessus
export default defineConfig({
  plugins: [
    vue(),
    ui({
      ui: { colors: { primary: 'violet', neutral: 'zinc' } },
      // Icônes embarquées : l'app fonctionne hors ligne
      // Drapeaux choisis dynamiquement : le scan ne les voit pas, ils sont listés
      icon: { clientBundle: { scan: true, icons: [...FLAG_ICONS, 'simple-icons:discord'] } }
    })
  ],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ['**/src-tauri/**'] } },
  build: { target: 'es2022' }
})
