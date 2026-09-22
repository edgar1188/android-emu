<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useAppStore } from '../stores/useAppStore'
import imageLogo from '../assets/image.png'
import { Sun, Moon, RefreshCw, CheckCircle, Code, Lock, Brush, Database, Star } from '@lucide/vue'

const appStore = useAppStore()
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
    <nav class="canaima-navbar">
      <a href="#" style="font-size: 1.25rem; font-weight: 600;">Canaima App</a>
      <ul class="canaima-nav-links">
        <li><a href="https://tauri.app" target="_blank">Tauri</a></li>
        <li><a href="https://vuejs.org" target="_blank">Vue 3</a></li>
        <li><a href="https://materializecss.com" target="_blank">Materialize</a></li>
        <li>
          <button @click.prevent="appStore.toggleDarkMode()" style="background: none; border: none; color: white; cursor: pointer; display: flex; padding: 0;">
            <Sun v-if="appStore.isDarkMode" :size="20" />
            <Moon v-else :size="20" />
          </button>
        </li>
      </ul>
    </nav>

    <!-- Main Hero Section -->
    <main style="flex: 1; padding: 4rem 1rem; max-width: 800px; margin: 0 auto; width: 100%; text-align: center;">
      <img :src="imageLogo" alt="Canaima Logo" class="logo-canaima" style="max-height: 100px; margin-bottom: 2rem; transition: filter 0.3s;" />
      
      <h1 style="font-size: 2.5rem; font-weight: 300; margin: 0 0 1rem 0;">
        Arquitectura Profesional
      </h1>
      
      <p style="font-size: 1.1rem; opacity: 0.8; margin-bottom: 3rem;">
        Esta plantilla implementa Vue 3, Pinia, Vue Router y Materialize CSS de forma nativa e impulsada por el rendimiento de Tauri.
      </p>

      <!-- Test IPC Section -->
      <section id="test-ipc" class="canaima-card" style="max-width: 500px; margin: 0 auto; text-align: left;">
        <h3 style="margin-top: 0; font-size: 1.5rem; margin-bottom: 1.5rem;">Prueba IPC</h3>
        <form @submit.prevent="greet">
          <input
            class="input-canaima"
            type="text"
            v-model="name"
            required
            placeholder="Escribe tu nombre..."
          />
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
    <footer>
      <div class="footer-container">
        <div style="flex: 1; min-width: 300px;">
          <h5 style="margin-top: 0;">Garantía Full-Stack</h5>
          <p style="opacity: 0.7; font-size: 0.9rem;">
            Añade vistas en <code>src/views/</code>, maneja estado en <code>src/stores/</code> y crea componentes en <code>src/components/</code>.
          </p>
        </div>
        <div style="flex: 1; min-width: 200px;">
          <h5 style="margin-top: 0;">Stack Moderno</h5>
          <ul class="footer-list">
            <li><Code :size="16" /> Vite + Vue 3</li>
            <li><Lock :size="16" /> Tauri / Rust API</li>
            <li><Brush :size="16" /> Materialize CSS</li>
            <li><Database :size="16" /> Pinia Store</li>
            <li><Star :size="16" /> Lucide Icons</li>
          </ul>
        </div>
      </div>
      <div style="text-align: center; margin-top: 2rem; opacity: 0.5; font-size: 0.8rem;">
        © 2026 Plantilla Canaima App
      </div>
    </footer>
  </div>
</template>

<style scoped>
.spin-anim {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
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

footer {
  padding: 2rem;
  background-color: var(--canaima-surface-light);
  border-top: 1px solid #e2e8f0;
  margin-top: auto;
}

.footer-container {
  max-width: 1000px;
  margin: 0 auto;
  display: flex;
  flex-wrap: wrap;
  gap: 2rem;
  justify-content: space-between;
}

.footer-list {
  list-style: none;
  padding: 0;
  margin: 0;
  opacity: 0.8;
  font-size: 0.9rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.footer-list li {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

html.dark footer {
  background-color: var(--canaima-surface-dark);
  border-top-color: #333;
}

html.dark .alert-success {
  background-color: #2a2a2a;
}
</style>
