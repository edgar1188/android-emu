<script setup lang="ts">
import { ref } from 'vue'
import { Activity, Monitor, Settings2 } from '@lucide/vue'
import Navbar from '../components/Navbar.vue'
import SpiceConnection from '../components/SpiceConnection.vue'

const frameUrl = ref('')
const frameSize = ref({ width: 0, height: 0 })

function updateFrame(frame: { width: number; height: number; dataUrl: string }) {
  frameUrl.value = frame.dataUrl
  frameSize.value = { width: frame.width, height: frame.height }
}
</script>

<template>
  <div class="home-shell">
    <Navbar />

    <main class="workspace">
      <header class="workspace-header">
        <div>
          <p class="eyebrow">Canaima Android Emulator</p>
          <h1>Sesión de emulación</h1>
          <p class="workspace-description">
            Configura el transporte SPICE y prepara la pantalla virtual de Android.
          </p>
        </div>
        <div class="session-state">
          <span class="state-dot"></span>
          Sin sesión activa
        </div>
      </header>

      <div class="workspace-grid">
        <section class="display-panel">
          <div class="panel-heading">
            <div class="panel-title">
              <Monitor :size="18" />
              <span>Pantalla virtual</span>
            </div>
            <span class="panel-badge">SPICE</span>
          </div>

          <div v-if="!frameUrl" class="display-placeholder">
            <Activity :size="34" />
            <strong>Esperando conexión de vídeo</strong>
            <span>Conecta SPICE para recibir la pantalla de Android dentro de esta ventana.</span>
          </div>
          <div v-else class="display-frame">
            <img :src="frameUrl" alt="Pantalla del dispositivo Android emulado" />
            <span>{{ frameSize.width }} x {{ frameSize.height }}</span>
          </div>
        </section>

        <aside class="control-column">
          <SpiceConnection @frame="updateFrame" />

          <section class="settings-panel">
            <div class="panel-title">
              <Settings2 :size="18" />
              <span>Configuración rápida</span>
            </div>
            <dl>
              <div>
                <dt>Resolución</dt>
                <dd>1280 x 720</dd>
              </div>
              <div>
                <dt>Arquitectura</dt>
                <dd>x86_64</dd>
              </div>
              <div>
                <dt>Entrada</dt>
                <dd>Teclado y puntero</dd>
              </div>
            </dl>
          </section>
        </aside>
      </div>
    </main>
  </div>
</template>

<style scoped>
.home-shell {
  min-height: 100vh;
  background: var(--canaima-bg-light);
}

.workspace {
  width: min(1440px, 100%);
  margin: 0 auto;
  padding: 2.5rem clamp(1rem, 4vw, 3.5rem);
}

.workspace-header {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 2rem;
  margin-bottom: 2rem;
}

.eyebrow {
  margin: 0 0 0.5rem;
  color: var(--canaima-primary);
  font-size: 0.72rem;
  font-weight: 700;
  letter-spacing: 0.1em;
  text-transform: uppercase;
}

h1 {
  margin: 0;
  color: var(--canaima-text-dark);
  font-size: clamp(1.8rem, 4vw, 2.8rem);
  font-weight: 650;
}

.workspace-description {
  max-width: 550px;
  margin: 0.65rem 0 0;
  color: #60747d;
}

.session-state {
  display: inline-flex;
  align-items: center;
  gap: 0.55rem;
  padding: 0.55rem 0.75rem;
  border: 1px solid #d8e1e4;
  border-radius: 5px;
  color: #60747d;
  font-size: 0.8rem;
  white-space: nowrap;
}

.state-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #96a5aa;
}

.workspace-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 360px;
  gap: 1.25rem;
  align-items: start;
}

.display-panel,
.settings-panel {
  border: 1px solid #dce5e8;
  border-radius: 8px;
  background: var(--canaima-surface-light);
}

.display-panel {
  min-height: 590px;
  overflow: hidden;
}

.panel-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 1rem 1.25rem;
  border-bottom: 1px solid #e5ecee;
}

.panel-title {
  display: flex;
  align-items: center;
  gap: 0.55rem;
  color: var(--canaima-text-dark);
  font-weight: 650;
}

.panel-badge {
  padding: 0.25rem 0.45rem;
  border-radius: 4px;
  background: #e6f3f5;
  color: #087a72;
  font-size: 0.68rem;
  font-weight: 700;
  letter-spacing: 0.06em;
}

.display-placeholder {
  display: flex;
  min-height: 535px;
  align-items: center;
  justify-content: center;
  flex-direction: column;
  gap: 0.7rem;
  padding: 2rem;
  background: #f7fafb;
  color: #87a0a8;
  text-align: center;
}

.display-frame {
  position: relative;
  display: grid;
  min-height: 535px;
  place-items: center;
  padding: 1.5rem;
  background: #0b1216;
}

.display-frame img {
  display: block;
  width: 100%;
  max-height: 500px;
  object-fit: contain;
  image-rendering: auto;
}

.display-frame span {
  position: absolute;
  right: 1rem;
  bottom: 1rem;
  padding: 0.25rem 0.45rem;
  border-radius: 4px;
  background: rgb(0 0 0 / 55%);
  color: #d6e4e7;
  font-size: 0.72rem;
}

.display-placeholder strong {
  color: #49616a;
}

.display-placeholder span {
  max-width: 330px;
  font-size: 0.82rem;
  line-height: 1.5;
}

.control-column {
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
}

.control-column :deep(.spice-connection) {
  width: 100%;
  margin: 0;
}

.settings-panel {
  padding: 1.25rem;
}

dl {
  margin: 1rem 0 0;
}

dl div {
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  padding: 0.7rem 0;
  border-top: 1px solid #edf1f2;
  font-size: 0.82rem;
}

dt {
  color: #71858c;
}

dd {
  margin: 0;
  color: var(--canaima-text-dark);
  font-weight: 600;
  text-align: right;
}

html.dark & .home-shell {
  background: var(--canaima-bg-dark);
}

html.dark & h1,
html.dark & .panel-title,
html.dark & dd {
  color: var(--canaima-text-light);
}

html.dark & .workspace-description,
html.dark & dt {
  color: #a9bdc3;
}

html.dark & .display-panel,
html.dark & .settings-panel {
  border-color: #34444a;
  background: var(--canaima-surface-dark);
}

html.dark & .panel-heading {
  border-color: #34444a;
}

html.dark & .display-placeholder {
  background: #182329;
}

html.dark & .session-state {
  border-color: #34444a;
  color: #a9bdc3;
}

html.dark & dl div {
  border-color: #34444a;
}

@media (max-width: 900px) {
  .workspace-header {
    align-items: flex-start;
    flex-direction: column;
    gap: 1rem;
  }

  .workspace-grid {
    grid-template-columns: 1fr;
  }

  .display-panel {
    min-height: 420px;
  }

  .display-placeholder {
    min-height: 365px;
  }
}
</style>
