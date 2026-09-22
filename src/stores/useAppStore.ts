import { invoke } from '@tauri-apps/api/core'
import { defineStore } from 'pinia'
import { ref } from 'vue'

// 'useAppStore' actúa como el almacén global (Global State) principal
export const useAppStore = defineStore('app', () => {
  // === Estado (Variables Globales) ===
  const isDarkMode = ref(localStorage.getItem('theme') === 'dark')

  // Inicializar el tema actual basado en las preferencias guardadas
  function initTheme() {
    if (isDarkMode.value) {
      document.documentElement.classList.add('dark')
    }
    // Sincronizar tema con GTK nativo en Linux (también para el claro inicial)
    invoke('sync_theme', { isDark: isDarkMode.value }).catch(() => null)
  }

  // === Acciones (Mutaciones) ===
  function toggleDarkMode() {
    isDarkMode.value = !isDarkMode.value
    if (isDarkMode.value) {
      document.documentElement.classList.add('dark')
      localStorage.setItem('theme', 'dark')
    } else {
      document.documentElement.classList.remove('dark')
      localStorage.setItem('theme', 'light')
    }
    // Sincronizar tema con GTK nativo en Linux
    invoke('sync_theme', { isDark: isDarkMode.value }).catch(() => null)
  }

  // === Exportar ===
  return {
    isDarkMode,
    initTheme,
    toggleDarkMode,
  }
})
