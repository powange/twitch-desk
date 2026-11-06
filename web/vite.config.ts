import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import ui from '@nuxt/ui/vite'
import { FLAG_ICONS } from './src/languages'

// Web app mobile, servie par le serveur de synchronisation sous /app (dossier dist/ copié dans
// l'image Docker). En développement : `npm run dev`, API relayée vers un serveur local
export default defineConfig({
  base: '/app/',
  plugins: [
    vue(),
    ui({
      router: false,
      ui: { colors: { primary: 'violet', neutral: 'zinc' } },
      // Icônes embarquées : rien n'est chargé ailleurs que sur le serveur
      // Drapeaux choisis selon la langue : le scan ne les voit pas, ils sont listés
      icon: { clientBundle: { scan: true, icons: FLAG_ICONS } }
    })
  ],
  clearScreen: false,
  server: { port: 1421, proxy: { '/api': process.env.SYNC_SERVER ?? 'http://127.0.0.1:8787' } },
  build: { target: 'es2022' }
})
