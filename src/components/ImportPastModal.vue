<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'

// Import par lot d'anciens giveaways (JSON) : vérifié d'abord, giveaway par giveaway, puis importé
const open = defineModel<boolean>('open', { required: true })
const toast = useToast()

interface ImportLine { index: number, channel: string, date: string, ok: boolean, message: string | null }
interface ImportReport { ok: number, errors: number, imported: boolean, lines: ImportLine[] }

const json = ref('')
const report = ref<ImportReport | null>(null)
// JSON vérifié : l'import n'est proposé que pour lui (modifié : à revérifier)
const checkedJson = ref<string | null>(null)
const busy = ref(false)
const docOpen = ref(false)

watch(open, (o) => {
  if (!o) return
  report.value = null
  checkedJson.value = null
})

const canImport = computed(() => !!report.value && !report.value.imported && report.value.ok > 0 && checkedJson.value === json.value)

async function run(dryRun: boolean) {
  if (!json.value.trim()) return
  busy.value = true
  try {
    const r = await invoke<ImportReport>('import_past_giveaways', { json: json.value, dryRun })
    report.value = r
    checkedJson.value = json.value
    if (!dryRun) {
      toast.add({ title: `${r.ok} ancien(s) giveaway(s) importé(s)`, description: r.errors ? `${r.errors} non importé(s) : voir le détail` : undefined, color: r.errors ? 'warning' : 'success', icon: 'i-lucide-check' })
    }
  } catch (e) {
    report.value = null
    toast.add({ title: dryRun ? 'Vérification impossible' : 'Import impossible', description: String(e), color: 'error' })
  } finally {
    busy.value = false
  }
}

// Fichier .json choisi : son contenu remplace le texte
const fileInput = ref<HTMLInputElement | null>(null)
async function onFile(e: Event) {
  const file = (e.target as HTMLInputElement).files?.[0]
  if (!file) return
  json.value = await file.text()
  report.value = null
  ;(e.target as HTMLInputElement).value = ''
}

const EXAMPLE = `[
  {
    "channel": "saenz203",
    "date": "2026-10-20 21:30",
    "type": "streamelements",
    "url": "https://streamelements.com/saenz203/giveaway/6ae91894a0a183147ae56a71",
    "gifts": ["Gilded Phoenix Ship Crest"],
    "winners": [{ "login": "bilge_rat_bust3r267", "chance": "20.92%" }],
    "conditions": { "presence": true },
    "notes": "Tirage en fin de live"
  },
  {
    "channel": "lunarsea",
    "date": "2026-09-14",
    "command": "!coffre",
    "winners": ["pseudo1", "pseudo2"]
  }
]`
function useExample() {
  json.value = EXAMPLE
  report.value = null
}
const formatDate = (iso: string) => {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso || '—' : d.toLocaleString('fr-FR', { day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' })
}
</script>

<template>
  <UModal v-model:open="open" title="Importer d'anciens giveaways" description="Colle ou ouvre un fichier JSON, vérifie-le, puis importe" :ui="{ content: 'max-w-4xl' }">
    <template #body>
      <div class="flex flex-col gap-4">
        <!-- Format attendu -->
        <div class="border border-default rounded-lg">
          <button type="button" class="w-full flex items-center gap-2 px-4 py-2.5 text-sm font-medium" :aria-expanded="docOpen" @click="docOpen = !docOpen">
            <UIcon name="i-lucide-book-open" class="size-4 text-muted" />
            <span class="flex-1 text-left">Format attendu</span>
            <UIcon :name="docOpen ? 'i-lucide-chevron-up' : 'i-lucide-chevron-down'" class="size-4 text-muted" />
          </button>
          <div v-if="docOpen" class="px-4 pb-4 flex flex-col gap-3 text-sm">
            <p>Une liste d'objets, un par giveaway (500 au plus). Chaque giveaway est enregistré <strong>déjà terminé</strong>, sans alerte ni participation, sans toucher à la fiche du streamer, et partagé avec tes amis. Un giveaway déjà présent (même chaîne, même date) n'est pas importé deux fois.</p>
            <table class="w-full text-xs">
              <thead class="text-muted uppercase">
                <tr><th class="text-left py-1 pr-3">Champ</th><th class="text-left py-1 pr-3">Obligatoire</th><th class="text-left py-1">Contenu</th></tr>
              </thead>
              <tbody class="align-top">
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">channel</td><td class="py-1 pr-3">oui</td><td class="py-1">Nom de la chaîne Twitch</td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">date</td><td class="py-1 pr-3">oui</td><td class="py-1">Date du giveaway, passée : « 2026-10-20 », « 2026-10-20 21:30 » (heure locale ; jour seul : midi) ou « 2026-10-20T19:30:00Z »</td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">type</td><td class="py-1 pr-3">non</td><td class="py-1">« command » (par défaut), « ticket », « streamelements » ou « channelpoints »</td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">command</td><td class="py-1 pr-3">non</td><td class="py-1">Commande à taper, ex. « !coffre »</td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">url</td><td class="py-1 pr-3">non</td><td class="py-1">Lien du giveaway StreamElements</td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">drawAt</td><td class="py-1 pr-3">non</td><td class="py-1">Heure du tirage (même format que « date ») ; sinon la date du giveaway</td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">winners</td><td class="py-1 pr-3">non</td><td class="py-1">Gagnants : des pseudos, ou des objets <span class="font-mono">{ "login": "pseudo", "chance": "20.92%" }</span></td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">gifts</td><td class="py-1 pr-3">non</td><td class="py-1">Cadeaux : titres exacts du catalogue (majuscules ignorées) ou numéros ; inconnus : ignorés et signalés</td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">conditions</td><td class="py-1 pr-3">non</td><td class="py-1"><span class="font-mono">{ "presence", "discord", "follow", "greeting" : true/false, "followDays", "chatMinutes" : nombre }</span></td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">ticketUnique, ticketMax</td><td class="py-1 pr-3">non</td><td class="py-1">Type ticket ou StreamElements : un seul ticket par personne (true/false), maximum par personne</td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">rewardName, rewardCost</td><td class="py-1 pr-3">non</td><td class="py-1">Type points de chaîne : récompense et son coût</td></tr>
                <tr class="border-t border-default"><td class="py-1 pr-3 font-mono">notes</td><td class="py-1 pr-3">non</td><td class="py-1">Notes du giveaway</td></tr>
              </tbody>
            </table>
            <div class="flex items-center gap-2">
              <span class="text-muted flex-1">Exemple</span>
              <UButton label="Utiliser l'exemple" size="xs" color="neutral" variant="soft" icon="i-lucide-clipboard-paste" @click="useExample" />
            </div>
            <pre class="text-xs font-mono bg-elevated rounded p-3 overflow-x-auto">{{ EXAMPLE }}</pre>
          </div>
        </div>

        <div class="flex items-center gap-2">
          <span class="text-sm font-medium flex-1">JSON</span>
          <input ref="fileInput" type="file" accept=".json,application/json" class="hidden" @change="onFile">
          <UButton label="Ouvrir un fichier" icon="i-lucide-file-json" size="xs" color="neutral" variant="soft" @click="fileInput?.click()" />
        </div>
        <UTextarea v-model="json" :rows="10" autoresize :maxrows="18" placeholder="[ { &quot;channel&quot;: &quot;…&quot;, &quot;date&quot;: &quot;2026-10-20&quot; } ]" class="w-full font-mono text-xs" />

        <!-- Résultat de la vérification (ou de l'import), giveaway par giveaway -->
        <div v-if="report" class="flex flex-col gap-2">
          <UAlert
            :color="report.errors ? 'warning' : 'success'"
            variant="subtle"
            :icon="report.errors ? 'i-lucide-triangle-alert' : 'i-lucide-check'"
            :title="report.imported ? `${report.ok} giveaway(s) importé(s)${report.errors ? `, ${report.errors} non importé(s)` : ''}` : `${report.ok} giveaway(s) prêt(s) à importer${report.errors ? `, ${report.errors} en erreur (ignorés)` : ''}`"
          />
          <div class="border border-default rounded-lg overflow-auto max-h-72">
            <table class="w-full text-sm">
              <thead class="bg-elevated/60 text-xs uppercase text-muted sticky top-0">
                <tr>
                  <th class="text-right px-3 py-2">N°</th>
                  <th class="text-left px-3 py-2">Chaîne</th>
                  <th class="text-left px-3 py-2">Date</th>
                  <th class="text-left px-3 py-2">État</th>
                  <th class="text-left px-3 py-2">Détail</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="l in report.lines" :key="l.index" class="border-t border-default align-top">
                  <td class="px-3 py-1.5 text-right tabular-nums text-muted">{{ l.index }}</td>
                  <td class="px-3 py-1.5 font-medium">{{ l.channel || '—' }}</td>
                  <td class="px-3 py-1.5 whitespace-nowrap">{{ formatDate(l.date) }}</td>
                  <td class="px-3 py-1.5">
                    <UBadge :label="l.ok ? (report.imported ? 'Importé' : 'Valide') : 'Ignoré'" :color="l.ok ? 'success' : 'error'" variant="subtle" size="sm" />
                  </td>
                  <td class="px-3 py-1.5 text-muted">{{ l.message ?? '' }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>
    </template>
    <template #footer>
      <div class="flex justify-end gap-2 w-full">
        <UButton label="Fermer" color="neutral" variant="ghost" @click="open = false" />
        <UButton label="Vérifier" icon="i-lucide-list-checks" color="neutral" variant="soft" :loading="busy" :disabled="!json.trim()" @click="run(true)" />
        <UButton :label="canImport ? `Importer ${report?.ok} giveaway(s)` : 'Importer'" icon="i-lucide-upload" :loading="busy" :disabled="!canImport" @click="run(false)" />
      </div>
    </template>
  </UModal>
</template>
