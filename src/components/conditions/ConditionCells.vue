<script setup lang="ts">
import type { ConditionActions, ConditionTarget } from '../../composables/useConditionActions'
import SolidIcon from '../SolidIcon.vue'

// Une cellule par condition : ? inconnu, picto si requis (cliquable pour agir), rien sinon.
// Les pictos non cliquables restent atteignables au clavier (tabindex) pour lire leur bulle.
defineProps<{ target: ConditionTarget, actions: ConditionActions }>()
</script>

<template>
  <td class="px-2 py-2">
    <div class="flex justify-center items-center">
      <UTooltip v-if="target.requirePresence === null" text="Présence au tirage : on ne sait pas">
        <span class="text-muted rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" role="img" aria-label="Présence au tirage : on ne sait pas">?</span>
      </UTooltip>
      <template v-else-if="target.requirePresence">
        <UTooltip text="Présence requise au moment du tirage">
          <span class="inline-flex rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" role="img" aria-label="Présence requise au moment du tirage">
            <UIcon name="i-lucide-user-check" class="size-4 text-emerald-600 dark:text-emerald-400 block" />
          </span>
        </UTooltip>
        <!-- Réponse écrite automatiquement si un de tes comptes est désigné gagnant -->
        <UTooltip
          v-if="target.winReply"
          :text="`Si tu gagnes, réponse dans le chat après ${target.winReplyDelay ?? 10} s : « ${target.winReply} »`"
          :ui="{ content: 'max-w-sm h-auto' }"
        >
          <span class="inline-flex ml-1 rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" role="img" :aria-label="`Réponse automatique si tu gagnes : ${target.winReply}`">
            <UIcon name="i-lucide-message-circle-reply" class="size-4 text-primary block" />
          </span>
        </UTooltip>
      </template>
    </div>
  </td>
  <td class="px-2 py-2">
    <div class="flex justify-center items-center">
      <UTooltip v-if="target.requireDiscord === null" text="Discord du streamer : on ne sait pas">
        <span class="text-muted rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" role="img" aria-label="Discord du streamer : on ne sait pas">?</span>
      </UTooltip>
      <UTooltip v-else-if="target.requireDiscord" text="Être sur le Discord du streamer : clique pour ouvrir son invitation">
        <button
          type="button"
          class="inline-flex text-indigo-600 dark:text-indigo-400 hover:text-indigo-400 disabled:opacity-50"
          :disabled="actions.openingDiscord === target.channel"
          :aria-label="`Ouvrir le Discord de ${target.channel}`"
          @click="actions.openDiscord(target.channel)"
        >
          <UIcon :name="actions.openingDiscord === target.channel ? 'i-lucide-loader-circle' : 'i-simple-icons-discord'" class="size-4" :class="actions.openingDiscord === target.channel ? 'animate-spin' : ''" />
        </button>
      </UTooltip>
    </div>
  </td>
  <td class="px-2 py-2">
    <div class="flex justify-center items-center">
      <UTooltip v-if="target.requireFollow === null" text="Follow requis : on ne sait pas">
        <span class="text-muted rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" role="img" aria-label="Follow requis : on ne sait pas">?</span>
      </UTooltip>
      <UTooltip v-else-if="target.requireFollow" :text="actions.followTooltip(target)">
        <button
          v-if="actions.notFollowing(target.channel).length"
          type="button"
          class="inline-flex text-warning hover:text-pink-500 disabled:opacity-50"
          :disabled="actions.following === target.channel"
          :aria-label="`${actions.followLabel(target)}. Suivre ${target.channel}`"
          @click="actions.followChannel(target.channel)"
        >
          <UIcon :name="actions.following === target.channel ? 'i-lucide-loader-circle' : 'i-lucide-heart'" class="size-4" :class="actions.following === target.channel ? 'animate-spin' : ''" />
        </button>
        <span v-else class="inline-flex rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" role="img" :aria-label="actions.followLabel(target)">
          <SolidIcon v-if="actions.followOk(target)" name="heart" class="size-4 block text-success" />
          <UIcon v-else name="i-lucide-heart" class="size-4 block" :class="actions.tooRecent(target).length ? 'text-warning' : 'text-pink-600 dark:text-pink-400'" />
        </span>
      </UTooltip>
      <span v-if="target.requireFollow && target.followMinDays" class="text-xs text-muted ml-1 whitespace-nowrap">{{ target.followMinDays }} j</span>
    </div>
  </td>
  <td class="px-2 py-2">
    <div class="flex justify-center items-center">
      <UTooltip v-if="target.requireGreeting === null" text="Avoir dit bonjour : on ne sait pas">
        <span class="text-muted rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" role="img" aria-label="Avoir dit bonjour : on ne sait pas">?</span>
      </UTooltip>
      <UTooltip v-else-if="target.requireGreeting" :text="actions.greetingTooltip(target.channel)">
        <span v-if="actions.greetedByAll(target.channel)" class="relative inline-flex text-success rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" role="img" aria-label="Bonjour déjà dit par tous tes comptes">
          <UIcon name="i-lucide-hand" class="size-4 block" />
          <UIcon name="i-lucide-check" class="absolute -bottom-1 -right-1.5 size-3 bg-default rounded-full" />
        </span>
        <button
          v-else
          type="button"
          class="inline-flex text-amber-600 dark:text-amber-400 hover:text-amber-500 disabled:opacity-50"
          :disabled="actions.greeting === target.channel"
          :aria-label="`Dire bonjour sur ${target.channel}`"
          @click="actions.sayHello(target.channel)"
        >
          <UIcon :name="actions.greeting === target.channel ? 'i-lucide-loader-circle' : 'i-lucide-hand'" class="size-4" :class="actions.greeting === target.channel ? 'animate-spin' : ''" />
        </button>
      </UTooltip>
    </div>
  </td>
  <td class="px-2 py-2">
    <div class="flex justify-center items-center">
      <UTooltip v-if="target.minChatMinutes === null" text="Temps de présence dans le chat : on ne sait pas">
        <span class="text-muted rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" role="img" aria-label="Temps de présence dans le chat : on ne sait pas">?</span>
      </UTooltip>
      <UTooltip v-else-if="target.minChatMinutes > 0" :text="`Présent dans le chat depuis au moins ${target.minChatMinutes} min`">
        <span class="inline-flex items-center gap-1 text-sky-600 dark:text-sky-400 whitespace-nowrap rounded focus-visible:outline-2 focus-visible:outline-primary" tabindex="0" :aria-label="`Présent dans le chat depuis au moins ${target.minChatMinutes} min`">
          <UIcon name="i-lucide-timer" class="size-4" />{{ target.minChatMinutes }} min
        </span>
      </UTooltip>
    </div>
  </td>
</template>
