<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Activity, Monitor } from '@lucide/vue'
import Navbar from '../components/Navbar.vue'
import SpiceConnection from '../components/SpiceConnection.vue'

const frameSize = ref({ width: 0, height: 0 })
const displayFrame = ref<HTMLDivElement | null>(null)
const displayCanvas = ref<HTMLCanvasElement | null>(null)
const inputError = ref('')
const frameError = ref('')
const framesDrawn = ref(0)
const pressedKeys = new Set<number>()
const pressedButtons = new Set<number>()
let lastPointerPosition: { x: number; y: number } | null = null
let inputQueue: Promise<unknown> = Promise.resolve()
let frameRevision = 0
let pendingMotion: { x: number; y: number; absoluteX: number; absoluteY: number; buttons: number } | null = null
let motionFrame = 0

const keyScancodes: Record<string, number> = {
  Escape: 0x01,
  Digit1: 0x02, Digit2: 0x03, Digit3: 0x04, Digit4: 0x05, Digit5: 0x06,
  Digit6: 0x07, Digit7: 0x08, Digit8: 0x09, Digit9: 0x0a, Digit0: 0x0b,
  Minus: 0x0c, Equal: 0x0d, Backspace: 0x0e, Tab: 0x0f,
  KeyQ: 0x10, KeyW: 0x11, KeyE: 0x12, KeyR: 0x13, KeyT: 0x14,
  KeyY: 0x15, KeyU: 0x16, KeyI: 0x17, KeyO: 0x18, KeyP: 0x19,
  BracketLeft: 0x1a, BracketRight: 0x1b, Enter: 0x1c, ControlLeft: 0x1d,
  KeyA: 0x1e, KeyS: 0x1f, KeyD: 0x20, KeyF: 0x21, KeyG: 0x22,
  KeyH: 0x23, KeyJ: 0x24, KeyK: 0x25, KeyL: 0x26,
  Semicolon: 0x27, Quote: 0x28, Backquote: 0x29, ShiftLeft: 0x2a,
  Backslash: 0x2b, KeyZ: 0x2c, KeyX: 0x2d, KeyC: 0x2e, KeyV: 0x2f,
  KeyB: 0x30, KeyN: 0x31, KeyM: 0x32, Comma: 0x33, Period: 0x34,
  Slash: 0x35, ShiftRight: 0x36, AltLeft: 0x38, Space: 0x39, CapsLock: 0x3a,
  F1: 0x3b, F2: 0x3c, F3: 0x3d, F4: 0x3e, F5: 0x3f, F6: 0x40,
  F7: 0x41, F8: 0x42, F9: 0x43, F10: 0x44, F11: 0x57, F12: 0x58,
  ControlRight: 0xe01d, AltRight: 0xe038, MetaLeft: 0xe05b, MetaRight: 0xe05c,
  Insert: 0xe052, Delete: 0xe053, Home: 0xe047, End: 0xe04f,
  PageUp: 0xe049, PageDown: 0xe051, ArrowUp: 0xe048, ArrowLeft: 0xe04b,
  ArrowRight: 0xe04d, ArrowDown: 0xe050, NumpadEnter: 0xe01c,
}

function queueInput(command: string, payload: Record<string, number | string | boolean>) {
  inputQueue = inputQueue
    .then(() => invoke(command, payload))
    .then(() => { inputError.value = '' })
    .catch((error: unknown) => { inputError.value = String(error) })
}

function pointerCoordinates(event: PointerEvent) {
  const canvas = displayCanvas.value
  if (!canvas || !frameSize.value.width || !frameSize.value.height) return null

  const rect = canvas.getBoundingClientRect()
  if (!rect.width || !rect.height) return null

  return {
    x: Math.max(0, Math.min(frameSize.value.width - 1,
      Math.floor((event.clientX - rect.left) * frameSize.value.width / rect.width))),
    y: Math.max(0, Math.min(frameSize.value.height - 1,
      Math.floor((event.clientY - rect.top) * frameSize.value.height / rect.height))),
  }
}

function buttonMask() {
  return [...pressedButtons].reduce((mask, button) => mask | button, 0)
}

function sendPointerMotion(event: PointerEvent, force = false) {
  const point = pointerCoordinates(event)
  if (!point) return

  const previous = lastPointerPosition ?? {
    x: Math.floor(frameSize.value.width / 2),
    y: Math.floor(frameSize.value.height / 2),
  }
  const x = point.x - previous.x
  const y = point.y - previous.y
  if (x || y || force) {
    pendingMotion = {
      x: (pendingMotion?.x ?? 0) + x,
      y: (pendingMotion?.y ?? 0) + y,
      absoluteX: point.x,
      absoluteY: point.y,
      buttons: buttonMask(),
    }
    if (!motionFrame) motionFrame = requestAnimationFrame(flushPointerMotion)
  }

  lastPointerPosition = point
}

function flushPointerMotion() {
  motionFrame = 0
  if (!pendingMotion) return

  const motion = pendingMotion
  pendingMotion = null
  queueInput('spice_mouse_motion', motion)
}

function onPointerDown(event: PointerEvent) {
  const button = event.button === 1 ? 2 : event.button === 2 ? 4 : 1
  const buttonName = button === 1 ? 'left' : button === 2 ? 'middle' : 'right'
  event.preventDefault()
  displayFrame.value?.focus()
  try {
    displayFrame.value?.setPointerCapture(event.pointerId)
  } catch {
    // Pointer capture may be unavailable when the frame is being replaced.
  }
  sendPointerMotion(event, true)
  if (motionFrame) cancelAnimationFrame(motionFrame)
  flushPointerMotion()
  pressedButtons.add(button)
  queueInput('spice_mouse_button', { button: buttonName, pressed: true })
}

function onPointerUp(event: PointerEvent) {
  const button = event.button === 1 ? 2 : event.button === 2 ? 4 : 1
  const buttonName = button === 1 ? 'left' : button === 2 ? 'middle' : 'right'
  event.preventDefault()
  sendPointerMotion(event, true)
  if (motionFrame) cancelAnimationFrame(motionFrame)
  flushPointerMotion()
  pressedButtons.delete(button)
  queueInput('spice_mouse_button', { button: buttonName, pressed: false })
}

function onWheel(event: WheelEvent) {
  event.preventDefault()
  const deltaY = event.deltaY < 0 ? 1 : event.deltaY > 0 ? -1 : 0
  if (deltaY) queueInput('spice_mouse_wheel', { deltaY })
}

function onKeyDown(event: KeyboardEvent) {
  const scancode = keyScancodes[event.code]
  if (scancode === undefined) return
  event.preventDefault()
  if (event.repeat || pressedKeys.has(scancode)) return
  pressedKeys.add(scancode)
  queueInput('spice_key', { scancode, pressed: true })
}

function onKeyUp(event: KeyboardEvent) {
  const scancode = keyScancodes[event.code]
  if (scancode === undefined) return
  event.preventDefault()
  if (!pressedKeys.delete(scancode)) return
  queueInput('spice_key', { scancode, pressed: false })
}

function releaseInputs() {
  for (const scancode of pressedKeys) {
    queueInput('spice_key', { scancode, pressed: false })
  }
  pressedKeys.clear()

  for (const button of pressedButtons) {
    const buttonName = button === 1 ? 'left' : button === 2 ? 'middle' : 'right'
    queueInput('spice_mouse_button', { button: buttonName, pressed: false })
  }
  pressedButtons.clear()
}

async function updateFrame(buffer: ArrayBuffer) {
  if (buffer.byteLength < 8) {
    frameError.value = 'El frame SPICE recibido está incompleto.'
    return
  }

  const header = new DataView(buffer)
  const width = header.getUint32(0, true)
  const height = header.getUint32(4, true)
  const pixelLength = width * height * 4
  if (!width || !height || buffer.byteLength !== 8 + pixelLength) {
    frameError.value = 'El frame SPICE tiene dimensiones o datos inválidos.'
    return
  }

  const revision = ++frameRevision
  frameSize.value = { width, height }
  await nextTick()

  const canvas = displayCanvas.value
  const context = canvas?.getContext('2d', { alpha: false })
  if (!canvas || !context) {
    frameError.value = 'No se pudo inicializar el canvas de vídeo.'
    return
  }

  if (revision !== frameRevision) return

  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width
    canvas.height = height
  }
  const pixels = new Uint8ClampedArray(buffer, 8, pixelLength)
  context.putImageData(new ImageData(pixels, width, height), 0, 0)
  framesDrawn.value += 1
  frameError.value = ''
}

function onPointerCancel() {
  releaseInputs()
  lastPointerPosition = null
}

onMounted(() => window.addEventListener('blur', releaseInputs))
onUnmounted(() => {
  window.removeEventListener('blur', releaseInputs)
  if (motionFrame) cancelAnimationFrame(motionFrame)
  releaseInputs()
})
</script>

<template>
  <div class="home-shell">
    <Navbar />

    <main class="workspace">

      <div class="workspace-grid">
        <section class="display-panel">
          <div class="panel-heading">
            <div class="panel-title">
              <Monitor :size="18" />
              <span>Android</span>
            </div>
          </div>

          <div v-if="!frameSize.width" class="display-placeholder">
            <Activity :size="34" />
            <strong>Esperando conexión de vídeo</strong>
            <span>Conecta SPICE para recibir la pantalla de Android dentro de esta ventana.</span>
          </div>
          <div v-else ref="displayFrame" class="display-frame" tabindex="0" aria-label="Pantalla interactiva de Android"
            @pointerdown="onPointerDown" @pointermove="sendPointerMotion" @pointerup="onPointerUp"
            @pointercancel="onPointerCancel" @wheel.prevent="onWheel" @contextmenu.prevent @keydown="onKeyDown"
            @keyup="onKeyUp">
            <canvas ref="displayCanvas" :width="frameSize.width" :height="frameSize.height" role="img"
              aria-label="Pantalla de Android" />
            <span>{{ frameSize.width }} x {{ frameSize.height }} · {{ framesDrawn }} frames</span>
            <small v-if="inputError" class="input-error" role="alert">{{ inputError }}</small>
            <small v-if="frameError" class="input-error" role="alert">{{ frameError }}</small>
          </div>
        </section>

        <aside class="control-column">
          <SpiceConnection @frame="updateFrame" />
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
  display: block;
  width: 100%;
  padding: 0;
  background: #0b1216;
  cursor: crosshair;
  outline: none;
  touch-action: none;
  user-select: none;
}

.display-frame:focus-visible {
  outline: 2px solid var(--canaima-primary);
  outline-offset: -2px;
}

.display-frame canvas {
  display: block;
  width: 100%;
  height: auto;
  pointer-events: none;
}

.input-error {
  position: absolute;
  top: 0.5rem;
  left: 0.5rem;
  max-width: calc(100% - 1rem);
  padding: 0.4rem 0.6rem;
  background: #7b2525;
  color: #fff;
  overflow-wrap: anywhere;
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
