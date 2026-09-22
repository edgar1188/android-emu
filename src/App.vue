<script setup lang="ts">
import { onMounted } from 'vue'
import { RouterView } from 'vue-router'
import { useAppStore } from './stores/useAppStore'
const appStore = useAppStore()

onMounted(() => {
  appStore.initTheme() // Aplica el tema seleccionado almacenado en cache
})
</script>

<template>
  <div class="app-layout">
    <main class="main-content">
      <RouterView />
    </main>
  </div>
</template>

<style>
.app-layout {
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
  
  color: var(--texto-oscuro, #333333);
  transition:
    background-color 0.3s ease,
    color 0.3s ease;
}

/* El store añade 'dark-mode' al body — usamos ese selector */
body.dark-mode .app-layout {
  background-color: var(--fondo-oscuro, #191919);
  
  color: var(--texto-claro, #ffffff);
}



.main-content {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  
}

/* El body hereda el fondo del sistema de colores */
body,
html {
  overflow: hidden !important;
  margin: 0;
  padding: 0;
  background-color: var(--fondo-claro, #ececec);
}

/* Ocultar barra de scroll global (pero mantener funcionamiento) */
::-webkit-scrollbar {
  display: none !important;
}

* {
  scrollbar-width: none !important;
  -ms-overflow-style: none !important;
}
</style>
