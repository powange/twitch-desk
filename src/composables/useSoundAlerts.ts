import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

// Alertes sonores : sons générés (pas de fichier audio) au choix, ou fichier de l'utilisateur.
// L'application les demande par l'événement « sound-alert » ; la fenêtre les joue, même cachée.
export type SoundAlertKind = 'giveaway' | 'win' | 'mention' | 'reminder' | 'scanner'

// Son d'une alerte : identifiant d'un son prédéfini, ou « custom » (fichier choisi)
export const CUSTOM_SOUND = 'custom'

export const SOUND_ALERTS: { kind: SoundAlertKind, label: string, hint: string }[] = [
  { kind: 'giveaway', label: 'Giveaway détecté', hint: 'Commande reprise par le chat, giveaway StreamElements' },
  { kind: 'win', label: 'Victoire', hint: 'Un bot annonce l\'un de tes comptes comme gagnant' },
  { kind: 'mention', label: 'Mention', hint: 'Un de tes pseudos est cité dans un chat surveillé' },
  { kind: 'reminder', label: 'Rappel de tirage', hint: '5 min avant un tirage où la présence est requise' },
  { kind: 'scanner', label: 'Stream repéré', hint: 'Nouveau stream trouvé par le scan automatique' }
]

// Notes (Hz), écart entre les notes et durée de chacune (s), timbre
interface Pattern { label: string, notes: number[], step: number, length: number, wave: OscillatorType }
export const SOUND_PRESETS: Record<string, Pattern> = {
  chime: { label: 'Carillon', notes: [659, 880], step: 0.14, length: 0.22, wave: 'triangle' },
  fanfare: { label: 'Fanfare', notes: [523, 659, 784, 1047], step: 0.12, length: 0.3, wave: 'triangle' },
  blip: { label: 'Double bip', notes: [1175, 1175], step: 0.11, length: 0.08, wave: 'sine' },
  alarm: { label: 'Alarme', notes: [784, 784, 784], step: 0.28, length: 0.16, wave: 'square' },
  soft: { label: 'Note douce', notes: [587], step: 0.2, length: 0.25, wave: 'sine' },
  bell: { label: 'Cloche', notes: [1319, 988], step: 0.18, length: 0.6, wave: 'sine' },
  rise: { label: 'Montée', notes: [392, 494, 587, 784], step: 0.07, length: 0.12, wave: 'triangle' },
  fall: { label: 'Descente', notes: [784, 587, 494, 392], step: 0.07, length: 0.12, wave: 'triangle' },
  pop: { label: 'Pop', notes: [1568], step: 0.1, length: 0.05, wave: 'sine' },
  retro: { label: 'Rétro', notes: [523, 784, 1047], step: 0.09, length: 0.09, wave: 'square' }
}

// Son par défaut de chaque alerte
export const DEFAULT_SOUND: Record<SoundAlertKind, string> = {
  giveaway: 'chime',
  win: 'fanfare',
  mention: 'blip',
  reminder: 'alarm',
  scanner: 'soft'
}

let context: AudioContext | null = null
let output: AudioNode | null = null

function audio(): AudioContext {
  context ??= new AudioContext()
  if (context.state === 'suspended') void context.resume()
  return context
}

// Sortie commune : limiteur qui laisse pousser le niveau au maximum sans saturer
function master(ctx: AudioContext): AudioNode {
  if (!output) {
    const limiter = ctx.createDynamicsCompressor()
    limiter.threshold.value = -6
    limiter.knee.value = 3
    limiter.ratio.value = 20
    limiter.attack.value = 0.002
    limiter.release.value = 0.1
    limiter.connect(ctx.destination)
    output = limiter
  }
  return output
}

// Volume perçu : progression au carré ; à 100 %, le son est poussé contre le limiteur (×3)
const gainFor = (volume: number) => 3 * Math.min(1, volume / 100) ** 2

function playPattern(pattern: Pattern, volume: number) {
  const ctx = audio()
  const start = ctx.currentTime + 0.02
  const peak = gainFor(volume)
  const out = master(ctx)
  pattern.notes.forEach((freq, i) => {
    const t = start + i * pattern.step
    const osc = ctx.createOscillator()
    const gain = ctx.createGain()
    osc.type = pattern.wave
    osc.frequency.value = freq
    // Attaque courte, note tenue, puis extinction, sans clic
    const level = pattern.wave === 'square' ? peak * 0.6 : peak
    const length = Math.max(pattern.length, 0.1) * 1.4
    gain.gain.setValueAtTime(0.0001, t)
    gain.gain.exponentialRampToValueAtTime(level, t + 0.01)
    gain.gain.setValueAtTime(level, t + length * 0.5)
    gain.gain.exponentialRampToValueAtTime(0.0001, t + length)
    osc.connect(gain).connect(out)
    osc.start(t)
    osc.stop(t + length + 0.02)
  })
}

// Fichiers de l'utilisateur décodés, par alerte (vidé quand un fichier est remplacé)
const buffers = new Map<SoundAlertKind, AudioBuffer>()

export function forgetCustomSound(kind: SoundAlertKind) {
  buffers.delete(kind)
}

async function playCustom(kind: SoundAlertKind, volume: number) {
  const ctx = audio()
  let buffer = buffers.get(kind)
  if (!buffer) {
    const bytes = await invoke<ArrayBuffer>('read_sound', { kind })
    buffer = await ctx.decodeAudioData(bytes)
    buffers.set(kind, buffer)
  }
  const source = ctx.createBufferSource()
  const gain = ctx.createGain()
  source.buffer = buffer
  gain.gain.value = gainFor(volume)
  source.connect(gain).connect(master(ctx))
  source.start()
}

// Joue le son choisi pour une alerte (son par défaut si le choix est vide ou introuvable)
export async function playSoundAlert(kind: SoundAlertKind, volume: number, sound?: string | null) {
  if (volume <= 0) return
  if (sound === CUSTOM_SOUND) {
    try {
      await playCustom(kind, volume)
      return
    } catch {
      // Fichier absent ou illisible : son par défaut
    }
  }
  const pattern = SOUND_PRESETS[sound ?? ''] ?? SOUND_PRESETS[DEFAULT_SOUND[kind]]
  if (pattern) playPattern(pattern, volume)
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
  void listen<{ kind: SoundAlertKind, volume: number, sound: string | null }>('sound-alert', (e) => {
    void playSoundAlert(e.payload.kind, e.payload.volume, e.payload.sound)
  })
}
