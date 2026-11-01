// Langue de diffusion Twitch (ISO 639-1) → drapeau (pays le plus parlant) et nom.
// Les icônes sont listées dans vite.config.ts pour être embarquées dans l'app.
export const LANGUAGES: Record<string, { flag: string, name: string }> = {
  fr: { flag: 'fr', name: 'Français' },
  en: { flag: 'gb', name: 'Anglais' },
  es: { flag: 'es', name: 'Espagnol' },
  de: { flag: 'de', name: 'Allemand' },
  it: { flag: 'it', name: 'Italien' },
  pt: { flag: 'pt', name: 'Portugais' },
  nl: { flag: 'nl', name: 'Néerlandais' },
  pl: { flag: 'pl', name: 'Polonais' },
  ru: { flag: 'ru', name: 'Russe' },
  uk: { flag: 'ua', name: 'Ukrainien' },
  tr: { flag: 'tr', name: 'Turc' },
  sv: { flag: 'se', name: 'Suédois' },
  no: { flag: 'no', name: 'Norvégien' },
  da: { flag: 'dk', name: 'Danois' },
  fi: { flag: 'fi', name: 'Finnois' },
  cs: { flag: 'cz', name: 'Tchèque' },
  sk: { flag: 'sk', name: 'Slovaque' },
  hu: { flag: 'hu', name: 'Hongrois' },
  ro: { flag: 'ro', name: 'Roumain' },
  bg: { flag: 'bg', name: 'Bulgare' },
  el: { flag: 'gr', name: 'Grec' },
  ca: { flag: 'es-ct', name: 'Catalan' },
  ja: { flag: 'jp', name: 'Japonais' },
  ko: { flag: 'kr', name: 'Coréen' },
  zh: { flag: 'cn', name: 'Chinois' },
  th: { flag: 'th', name: 'Thaï' },
  vi: { flag: 'vn', name: 'Vietnamien' },
  id: { flag: 'id', name: 'Indonésien' },
  ms: { flag: 'my', name: 'Malais' },
  tl: { flag: 'ph', name: 'Tagalog' },
  hi: { flag: 'in', name: 'Hindi' },
  ar: { flag: 'sa', name: 'Arabe' },
  he: { flag: 'il', name: 'Hébreu' }
}

export const FLAG_ICONS = [...new Set(Object.values(LANGUAGES).map(l => `flag:${l.flag}-4x3`))]
