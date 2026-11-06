import { inject, provide, type InjectionKey, type Ref } from 'vue'
import type { PhoneAction } from './types'

// Demandes au PC partagées avec les cartes et le scanner : jeton, demandes suivies, ajout d'une
// demande envoyée (suivie jusqu'à son résultat par App.vue)
export interface PhoneContext {
  token: Readonly<Ref<string | null>>
  actions: Ref<PhoneAction[]>
  requested: (a: PhoneAction) => void
}

const KEY: InjectionKey<PhoneContext> = Symbol('phone')

export function providePhone(ctx: PhoneContext) {
  provide(KEY, ctx)
}

export function usePhone(): PhoneContext {
  const ctx = inject(KEY)
  if (!ctx) throw new Error('providePhone manquant')
  return ctx
}

// Demande du même type encore en attente pour cet objet (giveaway ou chaîne)
export function pendingFor(actions: PhoneAction[], kind: string, target: string): boolean {
  return actions.some(a => a.status === 'pending' && a.kind === kind && (a.payload.item === target || a.payload.channel === target))
}

// Demande au PC depuis un composant : envoyée, suivie jusqu'au résultat (message), erreur affichée
export async function sendAction(ctx: PhoneContext, toast: ReturnType<typeof useToast>, body: Record<string, unknown>) {
  const token = ctx.token.value
  if (!token) return
  const { pcAction, ApiError } = await import('./api')
  try {
    ctx.requested(await pcAction<PhoneAction>(token, body))
  } catch (e) {
    toast.add({
      title: 'Demande non envoyée',
      description: e instanceof ApiError && e.status === 400 ? 'Mets à jour le serveur de synchronisation (version 0.8.11 ou plus).' : String(e),
      color: 'error'
    })
  }
}
