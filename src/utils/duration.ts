// Durées écrites « 7j », « 2h 30min », « 1j12h », « 45s » (mêmes règles que parse_duration côté Rust)

const UNITS: Record<string, number> = { j: 86_400, d: 86_400, h: 3_600, min: 60, mn: 60, m: 60, s: 1, sec: 1 }

/** Secondes ; null : vide ou illisible (unité inconnue, nombre sans unité, nul) */
export function parseDuration(text: string): number | null {
  const s = text.trim().toLowerCase().replace(/\s+/g, '')
  if (!s) return null
  const re = /(\d+)([a-z]+)/y
  let total = 0
  let m: RegExpExecArray | null
  while (re.lastIndex < s.length && (m = re.exec(s))) {
    const factor = UNITS[m[2]!]
    if (!factor) return null
    total += Number(m[1]) * factor
  }
  return re.lastIndex === s.length && total > 0 ? total : null
}

/** « 7j », « 2h 30min », « 1j 12h », « 45s » */
export function formatDuration(secs: number): string {
  const parts: string[] = []
  let rest = Math.max(0, Math.round(secs))
  for (const [unit, size] of [['j', 86_400], ['h', 3_600], ['min', 60], ['s', 1]] as const) {
    const n = Math.floor(rest / size)
    if (n) parts.push(`${n}${unit}`)
    rest -= n * size
  }
  return parts.join(' ') || '0s'
}
