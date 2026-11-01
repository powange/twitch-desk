<script setup lang="ts">
import { useSelectedAccount } from '../composables/useSelectedAccount'

// Compte dont la page montre les données : toujours visible, sélecteur s'il y en a plusieurs
const { accounts, accountId, account, accountItems } = useSelectedAccount()
</script>

<template>
  <USelect
    v-if="accounts.length > 1"
    v-model="accountId"
    :items="accountItems"
    class="w-52"
    icon="i-lucide-user"
    aria-label="Compte affiché"
  />
  <div v-else-if="account" class="inline-flex items-center gap-2 border border-default rounded-md px-2.5 py-1 text-sm" title="Compte affiché">
    <UAvatar :text="account.login.slice(0, 2).toUpperCase()" size="2xs" />
    <span class="font-medium">{{ account.login }}</span>
  </div>
</template>
