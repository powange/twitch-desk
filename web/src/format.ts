// Repris de src/utils/format.ts de l'application (sans Tauri)
const numberFormat = new Intl.NumberFormat('fr-FR')

export function formatNumber(n: number | null | undefined): string {
  return n === null || n === undefined ? '—' : numberFormat.format(n)
}

// « 21:30 » aujourd'hui, sinon « lun. 3 nov., 21:30 »
export function formatWhen(iso: string, now = new Date()): string {
  const d = new Date(iso)
  if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' })
  return d.toLocaleString('fr-FR', { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}

// Heure du tirage ; date seulement (heure inconnue, enregistrée à 23 h 59) : le jour, sans heure
export function formatDraw(iso: string, now = new Date()): string {
  const d = new Date(iso)
  if (d.getHours() !== 23 || d.getMinutes() !== 59) return formatWhen(iso, now)
  if (d.toDateString() === now.toDateString()) return 'aujourd\'hui'
  return d.toLocaleDateString('fr-FR', { weekday: 'short', day: 'numeric', month: 'short' })
}

// « il y a 3 min », « il y a 2 h », sinon la date
export function timeAgo(iso: string, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - new Date(iso).getTime()) / 1000))
  if (s < 60) return 'à l\'instant'
  if (s < 3600) return `il y a ${Math.floor(s / 60)} min`
  if (s < 86400) return `il y a ${Math.floor(s / 3600)} h`
  return new Date(iso).toLocaleDateString('fr-FR', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}

// Nom proposé pour cet appareil, tiré du navigateur
export function defaultDeviceName(ua = navigator.userAgent): string {
  if (/iPhone/.test(ua)) return 'iPhone'
  if (/iPad/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) return 'iPad'
  const android = ua.match(/Android [\d.]+; ([^;)]+)/)
  if (android?.[1] && !/^(K|wv)$/.test(android[1].trim())) return android[1].trim()
  if (/Android/.test(ua)) return 'Android'
  if (/Macintosh/.test(ua)) return 'Mac'
  if (/Windows/.test(ua)) return 'PC Windows'
  if (/Linux/.test(ua)) return 'Linux'
  return 'Navigateur'
}
