import { listen } from '@tauri-apps/api/event'

// Alertes sonores : sons générés (pas de fichier audio), un motif par type d'alerte.
// L'application les demande par l'événement « sound-alert » ; la fenêtre les joue, même cachée.
export type SoundAlertKind = 'giveaway' | 'win' | 'mention' | 'reminder' | 'scanner'

export const SOUND_ALERTS: { kind: SoundAlertKind, label: string, hint: string }[] = [
  { kind: 'giveaway', label: 'Giveaway détecté', hint: 'Commande reprise par le chat, giveaway StreamElements' },
  { kind: 'win', label: 'Victoire', hint: 'Un bot annonce l\'un de tes comptes comme gagnant' },
  { kind: 'mention', label: 'Mention', hint: 'Un de tes pseudos est cité dans un chat surveillé' },
  { kind: 'reminder', label: 'Rappel de tirage', hint: '5 min avant un tirage où la présence est requise' },
  { kind: 'scanner', label: 'Stream repéré', hint: 'Nouveau stream trouvé par le scan automatique' }
]

// Notes (Hz), durée (s) et timbre de chaque motif
const PATTERNS: Record<SoundAlertKind, { notes: number[], step: number, length: number, wave: OscillatorType }> = {
  giveaway: { notes: [659, 880], step: 0.14, length: 0.22, wave: 'triangle' },
  win: { notes: [523, 659, 784, 1047], step: 0.12, length: 0.3, wave: 'triangle' },
  mention: { notes: [1175, 1175], step: 0.11, length: 0.08, wave: 'sine' },
  reminder: { notes: [784, 784, 784], step: 0.28, length: 0.16, wave: 'square' },
  scanner: { notes: [587], step: 0.2, length: 0.25, wave: 'sine' }
}

let context: AudioContext | null = null

function audio(): AudioContext {
  context ??= new AudioContext()
  if (context.state === 'suspended') void context.resume()
  return context
}

export function playSoundAlert(kind: SoundAlertKind, volume: number) {
  const pattern = PATTERNS[kind]
  if (!pattern || volume <= 0) return
  const ctx = audio()
  const start = ctx.currentTime + 0.02
  // Volume perçu : progression au carré, plafonné pour rester supportable
  const peak = Math.min(1, volume / 100) ** 2 * 0.5
  pattern.notes.forEach((freq, i) => {
    const t = start + i * pattern.step
    const osc = ctx.createOscillator()
    const gain = ctx.createGain()
    osc.type = pattern.wave
    osc.frequency.value = freq
    // Attaque courte puis extinction, sans clic
    gain.gain.setValueAtTime(0.0001, t)
    gain.gain.exponentialRampToValueAtTime(pattern.wave === 'square' ? peak * 0.4 : peak, t + 0.01)
    gain.gain.exponentialRampToValueAtTime(0.0001, t + pattern.length)
    osc.connect(gain).connect(ctx.destination)
    osc.start(t)
    osc.stop(t + pattern.length + 0.02)
  })
}

let installed = false

// À appeler une fois au démarrage de l'interface
export function installSoundAlerts() {
  if (installed) return
  installed = true
  // Le moteur audio peut attendre une première action de l'utilisateur pour démarrer
  const unlock = () => {
    audio()
    window.removeEventListener('pointerdown', unlock)
    window.removeEventListener('keydown', unlock)
  }
  window.addEventListener('pointerdown', unlock)
  window.addEventListener('keydown', unlock)
  void listen<{ kind: SoundAlertKind, volume: number }>('sound-alert', (e) => {
    playSoundAlert(e.payload.kind, e.payload.volume)
  })
}
