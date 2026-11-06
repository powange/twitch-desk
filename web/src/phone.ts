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
