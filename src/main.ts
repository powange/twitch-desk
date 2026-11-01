import { createApp } from 'vue'
import { createRouter, createWebHashHistory } from 'vue-router'
import ui from '@nuxt/ui/vue-plugin'
import App from './App.vue'
import './assets/main.css'

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', name: 'dashboard', component: () => import('./pages/DashboardPage.vue') },
    { path: '/points', name: 'points', component: () => import('./pages/PointsPage.vue') },
    { path: '/giveaways', name: 'giveaways', component: () => import('./pages/giveaways/AlertsPage.vue') },
    { path: '/giveaways/channels', name: 'giveaway-channels', component: () => import('./pages/giveaways/ChannelsPage.vue') },
    { path: '/giveaways/categories', name: 'giveaway-categories', component: () => import('./pages/giveaways/CategoriesPage.vue') },
    { path: '/giveaways/settings', name: 'giveaway-settings', component: () => import('./pages/giveaways/SettingsPage.vue') },
    { path: '/drops', name: 'drops', component: () => import('./pages/DropsPage.vue') },
    { path: '/streamelements', name: 'streamelements', component: () => import('./pages/StreamElementsPage.vue') },
    { path: '/accounts', name: 'accounts', component: () => import('./pages/AccountsPage.vue') },
    { path: '/settings', name: 'settings', component: () => import('./pages/SettingsPage.vue') }
  ]
})

createApp(App).use(router).use(ui).mount('#app')
