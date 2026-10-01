<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { Channel, invoke } from '@tauri-apps/api/core'
import { CheckCircle, RefreshCw, Smartphone, XCircle } from '@lucide/vue'

type AdbDevice = {
  serial: string
  model: string
}

const devices = ref<AdbDevice[]>([])
const selectedSerial = ref('')
const status = ref('')
const isLoading = ref(false)
const errorMessage = ref('')
let activeFrameChannel: Channel<ArrayBuffer> | undefined
const emit = defineEmits<{
  frame: [frame: ArrayBuffer]
}>()

async function refreshDevices() {
  if (!(window as any).__TAURI_INTERNALS__) {
    devices.value = []
    status.value = 'Debe abrirse desde Tauri para usar ADB.'
    return
  }

  try {
    const list = (await invoke('adb_devices')) as AdbDevice[]
    devices.value = list

    const preferred = list.find((device) => device.serial.includes('emulator') || device.serial.includes('5554'))
      ?? list[0]

    if (preferred) {
      selectedSerial.value = preferred.serial
    } else if (!selectedSerial.value) {
      selectedSerial.value = ''
    }

    if (!list.length) {
      status.value = 'No hay dispositivos ADB conectados.'
    }
  } catch (error) {
    errorMessage.value = String(error)
  }
}

async function connectDevice() {
  if (!(window as any).__TAURI_INTERNALS__) {
    errorMessage.value = 'La conexión ADB solo funciona dentro de la app Tauri.'
    return
  }

  if (!selectedSerial.value) {
    errorMessage.value = 'Selecciona un dispositivo ADB antes de conectar.'
    return
  }

  isLoading.value = true
  errorMessage.value = ''
  status.value = ''

  try {
    const frameChannel = new Channel<ArrayBuffer>()
    frameChannel.onmessage = (frame) => emit('frame', frame)
    activeFrameChannel = frameChannel
    await invoke('adb_connect', { serial: selectedSerial.value, onFrame: frameChannel })
    status.value = `Conectado a ${selectedSerial.value}`
  } catch (error) {
    activeFrameChannel = undefined
    errorMessage.value = String(error)
  } finally {
    isLoading.value = false
  }
}

onMounted(() => {
  void refreshDevices().then(() => {
    if (selectedSerial.value) {
      void connectDevice()
    }
  })
})

onUnmounted(() => {
  const channel = activeFrameChannel
  if (!(window as any).__TAURI_INTERNALS__) {
    return
  }

  void invoke('adb_disconnect')
    .catch(() => undefined)
    .finally(() => {
      if (activeFrameChannel === channel) activeFrameChannel = undefined
    })
})
</script>

<template>
  <section class="canaima-card adb-connection">
    <div class="adb-heading">
      <div>
        <p class="eyebrow">Transporte</p>
        <h2>Conexión ADB</h2>
      </div>
      <Smartphone :size="22" aria-hidden="true" />
    </div>

    <div class="device-selector">
      <label for="adb-device">Dispositivo</label>
      <select id="adb-device" v-model="selectedSerial" class="input-canaima" :disabled="isLoading || !devices.length">
        <option value="" disabled>Selecciona un dispositivo</option>
        <option v-for="device in devices" :key="device.serial" :value="device.serial">
          {{ device.model }} · {{ device.serial }}
        </option>
      </select>

      <button class="btn-canaima" type="button" :disabled="isLoading || !selectedSerial" @click="connectDevice">
        <RefreshCw v-if="isLoading" :size="18" class="spin-anim" aria-hidden="true" />
        <Smartphone v-else :size="18" aria-hidden="true" />
        {{ isLoading ? 'Conectando...' : 'Conectar pantalla ADB' }}
      </button>
    </div>

    <button class="btn-canaima btn-secondary" type="button" @click="refreshDevices">
      Actualizar dispositivos
    </button>

    <div v-if="status" class="adb-status adb-status-success" role="status">
      <CheckCircle :size="20" aria-hidden="true" />
      <span>{{ status }}</span>
    </div>

    <div v-if="errorMessage" class="adb-status adb-status-error" role="alert">
      <XCircle :size="20" aria-hidden="true" />
      <span>{{ errorMessage }}</span>
    </div>
  </section>
</template>

<style scoped>
.adb-connection {
  max-width: 500px;
  margin: 2rem auto 0;
  text-align: left;
}

.adb-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  margin-bottom: 1.5rem;
  color: var(--canaima-primary);
}

.adb-heading h2 {
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

.device-selector {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
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

.btn-secondary {
  margin-top: 0.75rem;
}

.adb-status {
  display: flex;
  align-items: flex-start;
  gap: 0.5rem;
  margin-top: 1.25rem;
  padding: 0.9rem;
  overflow-wrap: anywhere;
}

.adb-status-success {
  color: #137333;
  background: #eaf6ed;
}

.adb-status-error {
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
