<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import imageLogo from '../assets/image.png'
import { RefreshCw, CheckCircle } from '@lucide/vue'
import Navbar from './Navbar.vue'
import Footer from './Footer.vue'

const greetMsg = ref('')
const name = ref('')
const isLoading = ref(false)

async function greet() {
  if (name.value) {
    isLoading.value = true
    try {
      greetMsg.value = await invoke('greet', { name: name.value })
      name.value = ''
    } finally {
      isLoading.value = false
    }
  }
}
</script>

<template>
  <div style="min-height: 100vh; display: flex; flex-direction: column;">
    <!-- Navbar Simple -->
    <Navbar />

    <!-- Main Hero Section -->
    <main style="flex: 1; padding: 4rem 1rem; max-width: 800px; margin: 0 auto; width: 100%; text-align: center;">
      <img :src="imageLogo" alt="Canaima Logo" class="logo-canaima"
        style="max-height: 100px; margin-bottom: 2rem; transition: filter 0.3s;" />

      <h1 style="font-size: 2.5rem; font-weight: 300; margin: 0 0 1rem 0;">
        Arquitectura Profesional
      </h1>

      <p style="font-size: 1.1rem; opacity: 0.8; margin-bottom: 3rem;">
        Esta plantilla implementa Vue 3, Pinia, Vue Router y Materialize CSS de forma nativa e impulsada por el
        rendimiento de Tauri.
      </p>

      <!-- Test IPC Section -->
      <section id="test-ipc" class="canaima-card" style="max-width: 500px; margin: 0 auto; text-align: left;">
        <h3 style="margin-top: 0; font-size: 1.5rem; margin-bottom: 1.5rem;">Prueba IPC</h3>
        <form @submit.prevent="greet">
          <input class="input-canaima" type="text" v-model="name" required placeholder="Escribe tu nombre..." />
          <button class="btn-canaima" type="submit" :disabled="isLoading" style="width: 100%; justify-content: center;">
            <RefreshCw v-if="isLoading" :size="18" class="spin-anim" />
            Probar Respuesta
          </button>
        </form>

        <div v-if="greetMsg" class="alert-success">
          <CheckCircle :size="20" color="var(--canaima-primary)" />
          <span>{{ greetMsg }}</span>
        </div>
      </section>
    </main>

    <!-- Footer Simple -->
    <Footer />
  </div>
</template>

<style scoped>
.spin-anim {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}

/* Invertir logo blanco a negro en modo claro */
html:not(.dark) .logo-canaima {
  filter: brightness(0);
}

/* Forzar colores de texto en modo oscuro para sobreescribir Materialize */
html.dark .canaima-card {
  color: var(--canaima-text-light) !important;
}

html.dark footer {
  background-color: var(--canaima-surface-dark) !important;
  border-top-color: #333 !important;
  color: var(--canaima-text-light) !important;
}

html.dark .alert-success {
  background-color: #2a2a2a !important;
  color: var(--canaima-text-light) !important;
}

.alert-success {
  margin-top: 1.5rem;
  padding: 1rem;
  background-color: var(--canaima-bg-light);
  border-left: 4px solid var(--canaima-primary);
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

html.dark .alert-success {
  background-color: #2a2a2a;
}
</style>
