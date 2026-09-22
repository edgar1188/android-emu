<script setup lang="ts">
import { useAppStore } from '../stores/useAppStore'
import { Sun, Moon } from '@lucide/vue'

const appStore = useAppStore()
</script>
<template>
    <nav class="canaima-navbar">
        <a href="#" style="font-size: 1.25rem; font-weight: 600;">Canaima App</a>
        <ul class="canaima-nav-links">
            <li><a href="https://tauri.app" target="_blank">Tauri</a></li>
            <li><a href="https://vuejs.org" target="_blank">Vue 3</a></li>
            <li><a href="https://materializecss.com" target="_blank">Materialize</a></li>
            <li>
                <button @click.prevent="appStore.toggleDarkMode()"
                    style="background: none; border: none; color: white; cursor: pointer; display: flex; padding: 0;">
                    <Sun v-if="appStore.isDarkMode" :size="20" />
                    <Moon v-else :size="20" />
                </button>
            </li>
        </ul>
    </nav>
</template>