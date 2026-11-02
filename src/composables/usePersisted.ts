import { ref, watch } from 'vue'
import type { Ref } from 'vue'

// Choix d'affichage retenus sur ce poste (filtres, tris…), d'un changement de page et
// d'un lancement à l'autre. Stockés en JSON dans le stockage local de la fenêtre ; une
// valeur absente, illisible ou refusée par `sanitize` redonne la valeur par défaut.
const PREFIX = 'td:'

export interface PersistedOptions<T> {
  // Valeur relue : la renvoyer (éventuellement nettoyée) si elle convient, sinon undefined
  sanitize?: (value: unknown) => T | undefined
}

function read<T>(key: string, initial: T, sanitize?: (value: unknown) => T | undefined): T {
  try {
    const raw = localStorage.getItem(PREFIX + key)
    if (raw === null) return initial
    const value: unknown = JSON.parse(raw)
    if (!sanitize) return value as T
    return sanitize(value) ?? initial
  } catch {
    return initial
  }
}

export function usePersisted<T>(key: string, initial: T, options: PersistedOptions<T> = {}): Ref<T> {
  const state = ref(read(key, initial, options.sanitize)) as Ref<T>
  watch(state, (value) => {
    try {
      localStorage.setItem(PREFIX + key, JSON.stringify(value))
    } catch {
      // Stockage indisponible : le choix vaut pour la session
    }
  }, { deep: true })
  return state
}

// Validations courantes
export const isBoolean = (v: unknown): boolean | undefined => typeof v === 'boolean' ? v : undefined
export const oneOf = <T extends string>(allowed: readonly T[]) =>
  (v: unknown): T | undefined => allowed.includes(v as T) ? v as T : undefined
export const stringList = (v: unknown): string[] | undefined =>
  Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : undefined
export const numberList = (v: unknown): number[] | undefined =>
  Array.isArray(v) ? v.filter((x): x is number => typeof x === 'number' && Number.isFinite(x)) : undefined
