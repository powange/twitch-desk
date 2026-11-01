<script setup lang="ts">
import { computed } from 'vue'
import { useSnapshot } from '../composables/useSnapshot'
import { LANGUAGES } from '../utils/languages'

// Drapeau de la langue de diffusion d'une chaîne (rien tant qu'elle n'est pas connue)
// `lang` : langue déjà connue (sinon, celle relevée pour les chaînes suivies)
const props = defineProps<{ login: string, lang?: string | null }>()
const { snapshot } = useSnapshot()

const code = computed(() => props.lang ?? snapshot.value?.languages[props.login.toLowerCase()])
const lang = computed(() => (code.value ? LANGUAGES[code.value] : undefined))
</script>

<template>
  <UTooltip v-if="lang" :text="lang.name">
    <UIcon :name="`i-flag-${lang.flag}-4x3`" class="w-4 h-3 rounded-[2px] shrink-0 shadow-[0_0_0_1px_rgb(0_0_0/0.12)]" />
  </UTooltip>
  <UTooltip v-else-if="code && code !== 'other'" :text="`Langue : ${code}`">
    <span class="text-[10px] uppercase font-semibold text-muted border border-default rounded px-0.5 leading-tight">{{ code }}</span>
  </UTooltip>
</template>
