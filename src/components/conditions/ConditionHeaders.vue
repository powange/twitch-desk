<!-- En-têtes des colonnes de conditions (une par condition), à côté de ConditionCells -->
<template>
  <th class="px-2 py-2">
    <div class="flex justify-center">
      <UTooltip text="Présence requise dans le chat au moment du tirage">
      <UIcon name="i-lucide-user-check" class="size-4 block" aria-label="Présence au tirage" />
      </UTooltip>
    </div>
  </th>
  <th class="px-2 py-2">
    <div class="flex justify-center">
      <UTooltip text="Être sur le Discord du streamer">
      <UIcon name="i-simple-icons-discord" class="size-4 block" aria-label="Discord requis" />
      </UTooltip>
    </div>
  </th>
  <th class="px-2 py-2">
    <div class="flex justify-center">
      <UTooltip text="Follow de la chaîne requis (vert : suivie par tous tes comptes ; clique pour suivre sinon)">
        <UIcon name="i-lucide-heart" class="size-4 block" aria-label="Follow requis" />
      </UTooltip>
    </div>
  </th>
  <th class="px-2 py-2">
    <div class="flex justify-center">
      <UTooltip text="Avoir dit bonjour dans le chat">
        <UIcon name="i-lucide-hand" class="size-4 block" aria-label="Avoir dit bonjour" />
      </UTooltip>
    </div>
  </th>
  <th class="px-2 py-2">
    <div class="flex justify-center">
      <UTooltip text="Temps de présence minimum dans le chat">
        <UIcon name="i-lucide-timer" class="size-4 block" aria-label="Temps de présence dans le chat" />
      </UTooltip>
    </div>
  </th>
  <th class="px-2 py-2">
    <div class="flex justify-center">
      <UTooltip text="Gain offrable à quelqu'un d'autre">
        <UIcon name="i-lucide-hand-heart" class="size-4 block" aria-label="Gain offrable à quelqu'un d'autre" />
      </UTooltip>
    </div>
  </th>
</template>
