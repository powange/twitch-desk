<script setup lang="ts">
import { computed } from 'vue'
import { LANGUAGES } from '../languages'

// Drapeau de la langue de diffusion d'une chaîne (comme sur le PC) ; rien tant qu'elle n'est pas
// connue. `langs` : langues publiées par le PC (login → code)
const props = defineProps<{ login: string, langs?: Record<string, string> | null }>()
const code = computed(() => props.langs?.[props.login.toLowerCase()])
const lang = computed(() => (code.value ? LANGUAGES[code.value] : undefined))
</script>

<template>
  <UIcon v-if="lang" :name="`i-flag-${lang.flag}-4x3`" class="w-4 h-3 rounded-[2px] shrink-0 shadow-[0_0_0_1px_rgb(0_0_0/0.12)]" :aria-label="lang.name" role="img" />
  <span v-else-if="code && code !== 'other'" class="text-[10px] uppercase font-semibold text-muted border border-default rounded px-0.5 leading-tight shrink-0">{{ code }}</span>
</template>
