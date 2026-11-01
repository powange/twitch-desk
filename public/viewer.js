// Page multi-stream de Twitch Desk (viewer.html)
// Adresse : viewer.html?channels=a,b,c&chat=a,c&novideo=c&featured=b
// (chaînes affichées, chat ouvert, vidéo masquée, chaîne mise en avant)
const LOGIN = /^[a-z0-9_]{1,25}$/
const MAX = 9
const parent = location.hostname
const grid = document.getElementById('grid')
const params = new URLSearchParams(location.search)

function readList(key) {
  const value = params.get(key) || ''
  return [...new Set(value.toLowerCase().split(',').map(s => s.trim()).filter(s => LOGIN.test(s)))].slice(0, MAX)
}
let channels = readList('channels')
const chats = new Set(readList('chat'))
const noVideo = new Set(readList('novideo'))
let featured = readList('featured')[0] || null
// Chaîne → { el, player, setChat }
const tiles = new Map()

// L'adresse reflète l'état : l'application la relit pour rouvrir la fenêtre telle quelle
function saveState() {
  const p = new URLSearchParams()
  if (channels.length) p.set('channels', channels.join(','))
  const open = channels.filter(c => chats.has(c))
  if (open.length) p.set('chat', open.join(','))
  const hidden = channels.filter(c => noVideo.has(c))
  if (hidden.length) p.set('novideo', hidden.join(','))
  if (featured) p.set('featured', featured)
  history.replaceState(null, '', `${location.pathname}?${p}`)
  document.title = channels.length ? `Streams : ${channels.join(', ')}` : 'Streams'
  document.getElementById('hint').textContent = channels.length ? `${channels.length} stream(s)` : ''
}

// Placement par la grille seulement : les lecteurs ne sont jamais déplacés dans la page,
// sinon ils se rechargeraient
function layout() {
  const n = channels.length
  const wide = innerWidth >= innerHeight
  for (const c of channels) tiles.get(c)?.el.classList.toggle('featured', c === featured && n > 1)
  if (featured && n > 1 && channels.includes(featured)) {
    // Mis en avant : grand à gauche (ou en haut), les autres en colonne (ou en ligne)
    const others = channels.filter(c => c !== featured)
    if (wide) {
      grid.style.gridTemplateColumns = 'minmax(0, 3fr) minmax(0, 1fr)'
      grid.style.gridTemplateRows = `repeat(${others.length}, minmax(0, 1fr))`
      place(featured, '1', `1 / span ${others.length}`)
      others.forEach((c, i) => place(c, '2', String(i + 1)))
    } else {
      grid.style.gridTemplateRows = 'minmax(0, 3fr) minmax(0, 1fr)'
      grid.style.gridTemplateColumns = `repeat(${others.length}, minmax(0, 1fr))`
      place(featured, `1 / span ${others.length}`, '1')
      others.forEach((c, i) => place(c, String(i + 1), '2'))
    }
    return
  }
  // Grille au plus près d'un carré, en privilégiant la largeur
  const count = Math.max(1, n)
  let cols = Math.ceil(Math.sqrt(count))
  let rows = Math.ceil(count / cols)
  if (!wide) [cols, rows] = [rows, cols]
  grid.style.gridTemplateColumns = `repeat(${cols}, minmax(0, 1fr))`
  grid.style.gridTemplateRows = `repeat(${rows}, minmax(0, 1fr))`
  channels.forEach((c, i) => place(c, String(i % cols + 1), String(Math.floor(i / cols) + 1)))
}
function place(channel, column, row) {
  const t = tiles.get(channel)
  if (!t) return
  t.el.style.gridColumn = column
  t.el.style.gridRow = row
}

function createTile(channel) {
  const el = document.createElement('div')
  el.className = 'tile'

  const bar = document.createElement('div')
  bar.className = 'bar'
  const name = document.createElement('span')
  name.className = 'name'
  name.textContent = channel
  const star = document.createElement('button')
  star.className = 'star'
  star.textContent = '★'
  star.title = 'Mettre en avant (agrandir)'
  const videoButton = document.createElement('button')
  videoButton.textContent = 'Vidéo'
  videoButton.title = 'Afficher ou masquer la vidéo (le son continue)'
  const chatButton = document.createElement('button')
  chatButton.textContent = 'Chat'
  chatButton.title = 'Afficher ou masquer le chat'
  const close = document.createElement('button')
  close.textContent = '✕'
  close.title = 'Retirer ce stream'
  close.setAttribute('aria-label', `Retirer ${channel}`)
  bar.append(name, star, videoButton, chatButton, close)

  const body = document.createElement('div')
  body.className = 'body'
  const holder = document.createElement('div')
  holder.className = 'player'
  const note = document.createElement('div')
  note.className = 'hidden-note'
  note.textContent = 'Vidéo et chat masqués'
  body.append(holder, note)
  el.append(bar, body)
  grid.append(el)

  // Lecteur officiel : son coupé sauf pour le premier stream ouvert
  const player = new Twitch.Player(holder, {
    channel,
    parent: [parent],
    width: '100%',
    height: '100%',
    autoplay: true,
    muted: tiles.size > 0
  })

  let chatFrame = null
  function setChat(open) {
    if (open && !chatFrame) {
      chatFrame = document.createElement('iframe')
      chatFrame.className = 'chat'
      chatFrame.title = `Chat de ${channel}`
      chatFrame.src = `https://www.twitch.tv/embed/${channel}/chat?parent=${parent}&darkpopout`
      body.append(chatFrame)
    } else if (!open && chatFrame) {
      chatFrame.remove()
      chatFrame = null
    }
    chatButton.classList.toggle('on', open)
    el.classList.toggle('no-chat', !open)
    if (open) chats.add(channel)
    else chats.delete(channel)
  }
  setChat(chats.has(channel))

  function setVideo(show) {
    videoButton.classList.toggle('on', show)
    el.classList.toggle('no-video', !show)
    if (show) noVideo.delete(channel)
    else noVideo.add(channel)
  }
  setVideo(!noVideo.has(channel))
  videoButton.addEventListener('click', () => { setVideo(el.classList.contains('no-video')); saveState() })
  chatButton.addEventListener('click', () => { setChat(!chatFrame); saveState() })
  star.addEventListener('click', () => {
    featured = featured === channel ? null : channel
    refreshStars()
    layout()
    applyPause()
    saveState()
  })
  close.addEventListener('click', () => removeChannel(channel))

  tiles.set(channel, { el, player, star })
  // Ajouté pendant une mise en avant avec l'option : en pause dès qu'il démarre
  // (une seule fois : un stream relancé à la main n'est pas remis en pause)
  let started = false
  player.addEventListener(Twitch.Player.PLAYING, () => {
    if (started) return
    started = true
    if (pauseOthers && featured !== null && channel !== featured) player.pause()
  })
}

function refreshStars() {
  for (const [c, t] of tiles) t.star.classList.toggle('on', c === featured)
}

function showEmpty() {
  grid.querySelector('.empty')?.remove()
  if (channels.length) return
  const empty = document.createElement('div')
  empty.className = 'empty'
  empty.textContent = 'Aucun stream. Ouvre-en un depuis Twitch Desk (bouton « Regarder »).'
  grid.append(empty)
}

function addChannel(channel) {
  channel = String(channel).toLowerCase()
  if (!LOGIN.test(channel) || channels.includes(channel) || channels.length >= MAX) return
  channels.push(channel)
  createTile(channel)
  showEmpty()
  layout()
  saveState()
}

function removeChannel(channel) {
  const t = tiles.get(channel)
  if (!t) return
  t.el.remove()
  tiles.delete(channel)
  channels = channels.filter(c => c !== channel)
  chats.delete(channel)
  noVideo.delete(channel)
  if (featured === channel) {
    featured = null
    applyPause()
  }
  showEmpty()
  layout()
  saveState()
}

// --- Commandes pour tous les streams ---
const players = () => [...tiles.values()].map(t => t.player)
document.getElementById('mute-all').addEventListener('click', () => {
  for (const p of players()) p.setMuted(true)
})
document.getElementById('low-volume').addEventListener('click', () => {
  for (const p of players()) {
    p.setMuted(false)
    p.setVolume(0.01)
  }
})
// Qualité la plus basse proposée par chaque stream (hors « audio seul »)
document.getElementById('low-quality').addEventListener('click', () => {
  for (const p of players()) {
    const list = (p.getQualities() || []).filter(q => q.group && q.group !== 'auto' && q.group !== 'audio_only')
    const lowest = list.sort((a, b) => (parseInt(a.name) || 0) - (parseInt(b.name) || 0))[0]
    if (lowest) p.setQuality(lowest.group)
  }
})
document.getElementById('auto-quality').addEventListener('click', () => {
  for (const p of players()) p.setQuality('auto')
})

// --- Pause des autres streams quand l'un est mis en avant (option retenue) ---
const PAUSE_KEY = 'viewer-pause-others'
let pauseOthers = false
try { pauseOthers = localStorage.getItem(PAUSE_KEY) === '1' } catch { /* stockage indisponible */ }
const pauseButton = document.getElementById('pause-others')
function applyPause() {
  pauseButton.classList.toggle('on', pauseOthers)
  for (const [c, t] of tiles) {
    const paused = pauseOthers && featured !== null && c !== featured
    if (paused) t.player.pause()
    else if (t.player.isPaused()) t.player.play()
  }
}
pauseButton.addEventListener('click', () => {
  pauseOthers = !pauseOthers
  try { localStorage.setItem(PAUSE_KEY, pauseOthers ? '1' : '0') } catch { /* pas grave */ }
  applyPause()
})

// Twitch Desk ajoute un stream sans recharger les autres
window.twitchDeskAdd = addChannel

addEventListener('resize', layout)
for (const c of channels) createTile(c)
if (featured && !channels.includes(featured)) featured = null
refreshStars()
pauseButton.classList.toggle('on', pauseOthers)
showEmpty()
layout()
saveState()
