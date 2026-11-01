import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'

// Liste réordonnable par une poignée : glisser-déposer (la ligne n'est déplaçable que pendant
// qu'on tient la poignée, pour ne pas gêner les interrupteurs et la sélection de texte) ou
// clavier (Alt+flèche haut/bas sur la poignée).
// L'ordre choisi reste affiché jusqu'à ce que le cœur renvoie le même : sans ça, la ligne
// revenait à sa place le temps que l'état soit rafraîchi.
export function useDragReorder<T extends { id: number }>(
  items: () => T[],
  save: (ids: number[]) => Promise<unknown>,
  errorTitle: string
) {
  const toast = useToast()
  const grabbed = ref<number | null>(null)
  const dragging = ref<number | null>(null)
  // Ordre affiché pendant le glisser (aperçu), null hors glisser
  const preview = ref<number[] | null>(null)
  // Ordre envoyé, en attente de confirmation par le cœur
  const pending = ref<number[] | null>(null)
  let pendingTimer: ReturnType<typeof setTimeout> | null = null

  const ordered = computed(() => {
    const list = items()
    const order = preview.value ?? pending.value
    if (!order) return list
    const byId = new Map(list.map(x => [x.id, x]))
    const kept = order.map(id => byId.get(id)).filter((x): x is T => !!x)
    // Élément ajouté entre-temps : à la fin
    return [...kept, ...list.filter(x => !order.includes(x.id))]
  })

  watch(() => items().map(x => x.id).join(), (ids) => {
    if (pending.value && ids === pending.value.join()) clearPending()
  })

  function clearPending() {
    pending.value = null
    if (pendingTimer) clearTimeout(pendingTimer)
    pendingTimer = null
  }

  async function commit(ids: number[]) {
    if (!ids.some((id, i) => id !== items()[i]?.id)) return
    pending.value = ids
    // Filet de sécurité : si le cœur ne confirme pas, on revient à son état
    if (pendingTimer) clearTimeout(pendingTimer)
    pendingTimer = setTimeout(clearPending, 5000)
    try {
      await save(ids)
    } catch (e) {
      clearPending()
      toast.add({ title: errorTitle, description: String(e), color: 'error' })
    }
  }

  function onDragStart(event: DragEvent, item: T, label: string) {
    dragging.value = item.id
    preview.value = ordered.value.map(x => x.id)
    event.dataTransfer?.setData('text/plain', label)
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
  }

  function onDragOver(event: DragEvent, target: T) {
    if (dragging.value === null || !preview.value) return
    event.preventDefault()
    if (target.id === dragging.value) return
    const ids = [...preview.value]
    ids.splice(ids.indexOf(dragging.value), 1)
    ids.splice(ids.indexOf(target.id), 0, dragging.value)
    preview.value = ids
  }

  function onDrop(event: DragEvent) {
    event.preventDefault()
    const ids = preview.value
    endDrag()
    if (ids) void commit(ids)
  }

  function endDrag() {
    dragging.value = null
    grabbed.value = null
    preview.value = null
  }

  // Clavier : Alt+flèche déplace d'un cran, la poignée garde le focus
  function onHandleKeydown(event: KeyboardEvent, item: T) {
    if (!event.altKey || (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')) return
    event.preventDefault()
    const ids = ordered.value.map(x => x.id)
    const from = ids.indexOf(item.id)
    const to = from + (event.key === 'ArrowUp' ? -1 : 1)
    if (from < 0 || to < 0 || to >= ids.length) return
    ids.splice(from, 1)
    ids.splice(to, 0, item.id)
    const handle = event.currentTarget as HTMLElement | null
    void commit(ids)
    void nextTick(() => handle?.focus())
  }

  // Bouton relâché ailleurs que sur la poignée, ou glisser abandonné hors de la liste :
  // la ligne n'est plus déplaçable
  function release() {
    if (dragging.value === null) grabbed.value = null
  }
  onMounted(() => {
    window.addEventListener('pointerup', release)
    window.addEventListener('dragend', endDrag)
  })
  onBeforeUnmount(() => {
    window.removeEventListener('pointerup', release)
    window.removeEventListener('dragend', endDrag)
    if (pendingTimer) clearTimeout(pendingTimer)
  })

  return { grabbed, dragging, ordered, onDragStart, onDragOver, onDrop, endDrag, onHandleKeydown }
}
