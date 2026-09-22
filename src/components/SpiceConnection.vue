<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { CheckCircle, RefreshCw, Wifi, XCircle } from '@lucide/vue'

type SpiceProbeResult = {
    transport: string
    majorVersion: number
    minorVersion: number
    messageSize: number
}

const endpoint = ref('127.0.0.1:5556')
const result = ref<SpiceProbeResult | null>(null)
const errorMessage = ref('')
const isConnecting = ref(false)
const isOpeningViewer = ref(false)

async function probeConnection() {
    isConnecting.value = true
    result.value = null
    errorMessage.value = ''

    try {
        result.value = await invoke<SpiceProbeResult>('spice_probe', {
            endpoint: endpoint.value.trim(),
            timeoutMs: 1500,
        })
    } catch (error) {
        errorMessage.value = String(error)
    } finally {
        isConnecting.value = false
    }
}

async function openViewer() {
    isOpeningViewer.value = true
    errorMessage.value = ''

    try {
        await invoke('spice_open_viewer', { endpoint: endpoint.value.trim() })
    } catch (error) {
        errorMessage.value = String(error)
    } finally {
        isOpeningViewer.value = false
    }
}
</script>

<template>
    <section class="canaima-card spice-connection">
        <div class="spice-heading">
            <div>
                <p class="eyebrow">Transporte</p>
                <h2>Conexión SPICE</h2>
            </div>
            <Wifi :size="22" aria-hidden="true" />
        </div>

        <form @submit.prevent="probeConnection">
            <label for="spice-endpoint">Endpoint SPICE</label>
            <input id="spice-endpoint" v-model="endpoint" class="input-canaima" type="text" required autocomplete="off"
                placeholder="127.0.0.1:5556 o unix:/tmp/android.spice" />
            <button class="btn-canaima" type="submit" :disabled="isConnecting">
                <RefreshCw v-if="isConnecting" :size="18" class="spin-anim" aria-hidden="true" />
                <Wifi v-else :size="18" aria-hidden="true" />
                {{ isConnecting ? 'Conectando...' : 'Probar conexión' }}
            </button>
            <button class="viewer-button" type="button" :disabled="isOpeningViewer" @click="openViewer">
                <RefreshCw v-if="isOpeningViewer" :size="18" class="spin-anim" aria-hidden="true" />
                <Wifi v-else :size="18" aria-hidden="true" />
                {{ isOpeningViewer ? 'Abriendo visor...' : 'Abrir visor SPICE' }}
            </button>
        </form>

        <div v-if="result" class="spice-status spice-status-success" role="status">
            <CheckCircle :size="20" aria-hidden="true" />
            <span>
                Conectado por {{ result.transport.toUpperCase() }} · SPICE
                {{ result.majorVersion }}.{{ result.minorVersion }}
            </span>
        </div>

        <div v-if="errorMessage" class="spice-status spice-status-error" role="alert">
            <XCircle :size="20" aria-hidden="true" />
            <span>{{ errorMessage }}</span>
        </div>
    </section>
</template>

<style scoped>
.spice-connection {
    max-width: 500px;
    margin: 2rem auto 0;
    text-align: left;
}

.spice-heading {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    margin-bottom: 1.5rem;
    color: var(--canaima-primary);
}

.spice-heading h2 {
    margin: 0.25rem 0 0;
    color: inherit;
    font-size: 1.5rem;
}

.eyebrow {
    margin: 0;
    color: inherit;
    font-size: 0.75rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
}

label {
    display: block;
    margin-bottom: 0.5rem;
    font-weight: 600;
}

.btn-canaima {
    width: 100%;
    justify-content: center;
}

.viewer-button {
    display: inline-flex;
    width: 100%;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    margin-top: 0.65rem;
    padding: 0.75rem 1.5rem;
    border: 1px solid var(--canaima-primary);
    border-radius: 4px;
    background: transparent;
    color: var(--canaima-primary);
    cursor: pointer;
    font-size: 1rem;
}

.viewer-button:hover {
    background: rgb(11 103 147 / 8%);
}

.spice-status {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    margin-top: 1.25rem;
    padding: 0.9rem;
    overflow-wrap: anywhere;
}

.spice-status-success {
    color: #137333;
    background: #eaf6ed;
}

.spice-status-error {
    color: #b42318;
    background: #fef3f2;
}

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
</style>
