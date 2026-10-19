<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  Activity,
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Ellipsis,
  EllipsisVertical,
  House,
  Moon,
  Monitor,
  PanelsTopLeft,
  Power,
  Sun,
  Volume1,
  Volume2,
  X,
} from '@lucide/vue'
import AdbConnection from '../components/AdbConnection.vue'
import { useAppStore } from '../stores/useAppStore'

const appStore = useAppStore()
const frameSize = ref({ width: 0, height: 0 })
const displayFrame = ref<HTMLDivElement | null>(null)
const displayCanvas = ref<HTMLCanvasElement | null>(null)
const inputError = ref('')
const frameError = ref('')
const emulatorStatus = ref('Iniciando el emulador Android...')
const emulatorError = ref('')
const isAndroidConnected = ref(false)
const isStatusDismissed = ref(false)
const isErrorDismissed = ref(false)
const isDownloadingImage = ref(false)
const imageDownloadPercentage = ref<number | null>(null)
const fps = ref(0)
const showHardwareControls = ref(false)
type HardwareControlsPosition = 'left' | 'right' | 'top' | 'bottom'
const savedHardwareControlsPosition = localStorage.getItem('hardware-controls-position')
const hardwareControlsPosition = ref<HardwareControlsPosition>(
  savedHardwareControlsPosition === 'left'
    || savedHardwareControlsPosition === 'right'
    || savedHardwareControlsPosition === 'top'
    || savedHardwareControlsPosition === 'bottom'
    ? savedHardwareControlsPosition
    : 'right',
)
let unlistenDownloadProgress: (() => void) | undefined
const pressedKeys = new Set<number>()
type ScreenPoint = { x: number; y: number }
let activePointerId: number | null = null
let gestureStart: ScreenPoint | null = null
let gestureEnd: ScreenPoint | null = null
let pendingPointerMove: ScreenPoint | null = null
let pointerMoveFrame = 0
let inputQueue: Promise<unknown> = Promise.resolve()
let frameRevision = 0
let reusableFrame: ImageData | undefined
let sourceFrameCanvas: HTMLCanvasElement | undefined
let sourceFrameContext: CanvasRenderingContext2D | null = null
let frameResizeObserver: ResizeObserver | undefined
let fpsWindowFrames = 0
let fpsWindowStartedAt = performance.now()
let statusDismissTimer: ReturnType<typeof setTimeout> | undefined
let errorDismissTimer: ReturnType<typeof setTimeout> | undefined
let dismissableTime = 10000

watch(() => appStore.isDarkMode, (isDark) => {
  queueInput('adb_set_dark_mode', { isDark })
})
watch(hardwareControlsPosition, (position) => {
  localStorage.setItem('hardware-controls-position', position)
}, { flush: 'sync' })
watch([emulatorStatus, isDownloadingImage], () => {
  if (statusDismissTimer) clearTimeout(statusDismissTimer)
  isStatusDismissed.value = false
  if (emulatorStatus.value && !isDownloadingImage.value) {
    statusDismissTimer = setTimeout(() => {
      isStatusDismissed.value = true
    }, dismissableTime)
  }
})
watch(emulatorError, () => {
  if (errorDismissTimer) clearTimeout(errorDismissTimer)
  isErrorDismissed.value = false
  if (emulatorError.value) {
    errorDismissTimer = setTimeout(() => {
      isErrorDismissed.value = true
    }, dismissableTime)
  }
})

const androidKeycodes: Record<string, number> = {
  Escape: 111, Backspace: 67, Tab: 61, Enter: 66,
  Space: 62, ShiftLeft: 59, ShiftRight: 60,
  ControlLeft: 113, ControlRight: 114, AltLeft: 57, AltRight: 58,
  ArrowLeft: 21, ArrowUp: 19, ArrowRight: 22, ArrowDown: 20,
  Home: 3, End: 7, PageUp: 92, PageDown: 93,
  Backquote: 47, Minus: 69, Equal: 70,
  BracketLeft: 71, BracketRight: 72, Backslash: 73,
  Semicolon: 74, Quote: 75, Comma: 55, Period: 56, Slash: 76,
  Digit0: 7, Digit1: 8, Digit2: 9, Digit3: 10, Digit4: 11,
  Digit5: 12, Digit6: 13, Digit7: 14, Digit8: 15, Digit9: 16,
  KeyA: 29, KeyB: 30, KeyC: 31, KeyD: 32, KeyE: 33, KeyF: 34,
  KeyG: 35, KeyH: 36, KeyI: 37, KeyJ: 38, KeyK: 39, KeyL: 40,
  KeyM: 41, KeyN: 42, KeyO: 43, KeyP: 44, KeyQ: 45, KeyR: 46,
  KeyS: 47, KeyT: 48, KeyU: 49, KeyV: 50, KeyW: 51, KeyX: 52,
  KeyY: 53, KeyZ: 54,
  F1: 131, F2: 132, F3: 133, F4: 134, F5: 135, F6: 136,
  F7: 137, F8: 138, F9: 139, F10: 140, F11: 141, F12: 142,
  Delete: 112, Insert: 124, CapsLock: 115,
  MetaLeft: 117, MetaRight: 118,
}

function queueInput(command: string, payload: Record<string, number | string | boolean>) {
  if (!(window as any).__TAURI_INTERNALS__) {
    return
  }

  inputQueue = inputQueue
    .then(() => invoke(command, payload))
    .then(() => { inputError.value = '' })
    .catch((error: unknown) => { inputError.value = String(error) })
}

function pressAndroidKey(keycode: number) {
  queueInput('scrcpy_input_key', { keycode, down: true, metaState: 0 })
  queueInput('scrcpy_input_key', { keycode, down: false, metaState: 0 })
}

function pointerCoordinates(event: { clientX: number; clientY: number }) {
  const canvas = displayCanvas.value
  if (!canvas || !frameSize.value.width || !frameSize.value.height) return null

  const rect = canvas.getBoundingClientRect()
  if (!rect.width || !rect.height) return null

  const imageAspect = frameSize.value.width / frameSize.value.height
  const canvasAspect = rect.width / rect.height
  const contentWidth = canvasAspect > imageAspect ? rect.height * imageAspect : rect.width
  const contentHeight = canvasAspect > imageAspect ? rect.height : rect.width / imageAspect
  const contentLeft = rect.left + (rect.width - contentWidth) / 2
  const contentTop = rect.top + (rect.height - contentHeight) / 2
  const offsetX = event.clientX - contentLeft
  const offsetY = event.clientY - contentTop
  if (offsetX < 0 || offsetX >= contentWidth || offsetY < 0 || offsetY >= contentHeight) {
    return null
  }

  return {
    x: Math.max(0, Math.min(frameSize.value.width - 1,
      Math.floor(offsetX * frameSize.value.width / contentWidth))),
    y: Math.max(0, Math.min(frameSize.value.height - 1,
      Math.floor(offsetY * frameSize.value.height / contentHeight))),
  }
}

function onPointerMove(event: PointerEvent) {
  if (event.pointerId !== activePointerId || !gestureStart) return
  const point = pointerCoordinates(event)
  if (!point) return

  gestureEnd = point
  pendingPointerMove = point
  if (!pointerMoveFrame) pointerMoveFrame = requestAnimationFrame(flushPointerMove)
}

function flushPointerMove() {
  pointerMoveFrame = 0
  if (activePointerId === null || !pendingPointerMove) return
  const point = pendingPointerMove
  pendingPointerMove = null
  queueInput('scrcpy_input_touch', { x: point.x, y: point.y, action: 'move' })
}

function onPointerDown(event: PointerEvent) {
  if (event.button !== 0 || activePointerId !== null) return
  event.preventDefault()
  const point = pointerCoordinates(event)
  if (!point) return

  displayFrame.value?.focus()
  try {
    displayCanvas.value?.setPointerCapture(event.pointerId)
  } catch {
    // Pointer capture may be unavailable when the frame is being replaced.
  }

  activePointerId = event.pointerId
  gestureStart = point
  gestureEnd = point
  queueInput('scrcpy_input_touch', { x: point.x, y: point.y, action: 'down' })
}

function onPointerUp(event: PointerEvent) {
  if (event.pointerId !== activePointerId || !gestureStart) return
  event.preventDefault()

  const endPoint = pointerCoordinates(event) ?? gestureEnd ?? gestureStart
  if (pointerMoveFrame) cancelAnimationFrame(pointerMoveFrame)
  pointerMoveFrame = 0
  if (pendingPointerMove) {
    queueInput('scrcpy_input_touch', {
      x: pendingPointerMove.x,
      y: pendingPointerMove.y,
      action: 'move',
    })
  }
  pendingPointerMove = null
  gestureEnd = endPoint
  queueInput('scrcpy_input_touch', { x: endPoint.x, y: endPoint.y, action: 'up' })

  activePointerId = null
  gestureStart = null
  gestureEnd = null
}

function onWheel(event: WheelEvent) {
  event.preventDefault()
  const canvas = displayCanvas.value
  if (!canvas || !frameSize.value.width || !frameSize.value.height) return

  const point = pointerCoordinates(event)
  if (!point || (!event.deltaX && !event.deltaY)) return
  queueInput('scrcpy_input_scroll', {
    x: point.x,
    y: point.y,
    hscroll: event.deltaX,
    vscroll: event.deltaY,
  })
}

function onKeyDown(event: KeyboardEvent) {
  const keycode = androidKeycodes[event.code]
  if (keycode === undefined) return
  event.preventDefault()
  if (event.repeat || pressedKeys.has(keycode)) return
  pressedKeys.add(keycode)
  queueInput('scrcpy_input_key', { keycode, down: true, metaState: androidMetaState() })
}

function onKeyUp(event: KeyboardEvent) {
  const keycode = androidKeycodes[event.code]
  if (keycode === undefined) return
  event.preventDefault()
  if (!pressedKeys.delete(keycode)) return
  queueInput('scrcpy_input_key', { keycode, down: false, metaState: androidMetaState() })
}

function androidMetaState() {
  let metaState = 0
  const has = (...keycodes: number[]) => keycodes.some((keycode) => pressedKeys.has(keycode))

  if (has(59, 60)) metaState |= 0x00000001
  if (pressedKeys.has(59)) metaState |= 0x00000040
  if (pressedKeys.has(60)) metaState |= 0x00000080
  if (has(57, 58)) metaState |= 0x00000002
  if (pressedKeys.has(57)) metaState |= 0x00000010
  if (pressedKeys.has(58)) metaState |= 0x00000020
  if (has(113, 114)) metaState |= 0x00001000
  if (pressedKeys.has(113)) metaState |= 0x00002000
  if (pressedKeys.has(114)) metaState |= 0x00004000
  if (has(117, 118)) metaState |= 0x00010000
  if (pressedKeys.has(117)) metaState |= 0x00020000
  if (pressedKeys.has(118)) metaState |= 0x00040000
  return metaState
}

function releaseInputs() {
  for (const keycode of [...pressedKeys]) {
    pressedKeys.delete(keycode)
    queueInput('scrcpy_input_key', { keycode, down: false, metaState: androidMetaState() })
  }
  pressedKeys.clear()
  if (activePointerId !== null && gestureEnd) {
    queueInput('scrcpy_input_touch', { x: gestureEnd.x, y: gestureEnd.y, action: 'up' })
  }
  activePointerId = null
  gestureStart = null
  gestureEnd = null
  pendingPointerMove = null
  if (pointerMoveFrame) cancelAnimationFrame(pointerMoveFrame)
  pointerMoveFrame = 0
}

function recordFrame() {
  fpsWindowFrames += 1
  const now = performance.now()
  const elapsed = now - fpsWindowStartedAt
  if (elapsed >= 1000) {
    fps.value = Math.round(fpsWindowFrames * 1000 / elapsed)
    fpsWindowFrames = 0
    fpsWindowStartedAt = now
  }
}

function renderSourceFrame() {
  const canvas = displayCanvas.value
  const source = sourceFrameCanvas
  if (!canvas || !source || !source.width || !source.height) return

  const rect = canvas.getBoundingClientRect()
  if (!rect.width || !rect.height) return

  const pixelRatio = window.devicePixelRatio || 1
  const targetWidth = Math.max(1, Math.round(rect.width * pixelRatio))
  const targetHeight = Math.max(1, Math.round(rect.height * pixelRatio))
  if (canvas.width !== targetWidth || canvas.height !== targetHeight) {
    canvas.width = targetWidth
    canvas.height = targetHeight
  }

  const context = canvas.getContext('2d', { alpha: false })
  if (!context) {
    frameError.value = 'No se pudo inicializar el canvas de vídeo.'
    return
  }

  const scale = Math.min(targetWidth / source.width, targetHeight / source.height)
  const width = source.width * scale
  const height = source.height * scale
  context.imageSmoothingEnabled = true
  context.imageSmoothingQuality = 'high'
  context.clearRect(0, 0, targetWidth, targetHeight)
  context.drawImage(
    source,
    (targetWidth - width) / 2,
    (targetHeight - height) / 2,
    width,
    height,
  )
}

function ensureSourceFrameCanvas(width: number, height: number) {
  if (!sourceFrameCanvas) {
    sourceFrameCanvas = document.createElement('canvas')
    frameResizeObserver = new ResizeObserver(renderSourceFrame)
  }
  if (sourceFrameCanvas.width !== width || sourceFrameCanvas.height !== height) {
    sourceFrameCanvas.width = width
    sourceFrameCanvas.height = height
  }
  sourceFrameContext = sourceFrameCanvas.getContext('2d', { alpha: false })
  if (!displayCanvas.value) return
  frameResizeObserver?.observe(displayCanvas.value)
}

async function updateFrame(rawFrame: ArrayBuffer | Uint8Array) {
  const buffer = rawFrame instanceof ArrayBuffer
    ? rawFrame
    : rawFrame.buffer.slice(rawFrame.byteOffset, rawFrame.byteOffset + rawFrame.byteLength)
  const bytes = new Uint8Array(buffer)
  const pngSignature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]
  const isPngFrame = bytes.length > 8 && bytes.slice(0, 8).every((value, index) => pngSignature[index] === value)

  if (isPngFrame) {
    try {
      const blob = new Blob([new Uint8Array(bytes).buffer], { type: 'image/png' })
      const objectUrl = URL.createObjectURL(blob)
      const image = await new Promise<HTMLImageElement>((resolve, reject) => {
        const img = new Image()
        img.onload = () => resolve(img)
        img.onerror = () => reject(new Error('No se pudo cargar la imagen PNG de ADB'))
        img.src = objectUrl
      })

      const nextSize = { width: image.width, height: image.height }
      const revision = ++frameRevision
      const dimensionsChanged = frameSize.value.width !== nextSize.width || frameSize.value.height !== nextSize.height
      if (dimensionsChanged) {
        frameSize.value = nextSize
        await nextTick()
      }

      if (revision !== frameRevision) {
        URL.revokeObjectURL(objectUrl)
        return
      }

      ensureSourceFrameCanvas(nextSize.width, nextSize.height)
      if (!sourceFrameCanvas || !sourceFrameContext) {
        frameError.value = 'No se pudo inicializar el canvas de vídeo.'
        URL.revokeObjectURL(objectUrl)
        return
      }

      sourceFrameContext.drawImage(image, 0, 0)
      renderSourceFrame()
      recordFrame()
      frameError.value = ''
      URL.revokeObjectURL(objectUrl)
    } catch (error) {
      frameError.value = `No se pudo decodificar el PNG de ADB: ${String(error)}`
    }
    return
  }

  if (bytes.length < 12) {
    frameError.value = 'El frame recibido está incompleto.'
    return
  }

  const header = new DataView(buffer)
  const width = header.getUint32(4, true)
  const height = header.getUint32(8, true)
  const pixelLength = width * height * 4
  if (!width || !height || bytes.length !== 12 + pixelLength) {
    frameError.value = 'El frame tiene dimensiones o datos inválidos.'
    return
  }

  const revision = ++frameRevision
  const dimensionsChanged = frameSize.value.width !== width || frameSize.value.height !== height
  if (dimensionsChanged) {
    frameSize.value = { width, height }
    await nextTick()
  }

  if (revision !== frameRevision) return

  ensureSourceFrameCanvas(width, height)
  if (!sourceFrameCanvas || !sourceFrameContext) {
    frameError.value = 'No se pudo inicializar el canvas de vídeo.'
    return
  }
  if (!reusableFrame || reusableFrame.width !== width || reusableFrame.height !== height) {
    reusableFrame = new ImageData(width, height)
  }
  reusableFrame.data.set(new Uint8ClampedArray(buffer, 12, pixelLength))
  sourceFrameContext.putImageData(reusableFrame, 0, 0)
  renderSourceFrame()
  recordFrame()
  frameError.value = ''
}

function onPointerCancel() {
  releaseInputs()
}

async function startAndroidEmulator() {
  if (!(window as any).__TAURI_INTERNALS__) {
    emulatorStatus.value = ''
    return
  }

  try {
    unlistenDownloadProgress = await listen<{ percentage: number | null }>(
      'emulator-download-progress',
      (event) => {
        imageDownloadPercentage.value = event.payload.percentage
        isDownloadingImage.value = event.payload.percentage !== 100
        emulatorStatus.value = isDownloadingImage.value
          ? 'Descargando la imagen de Android...'
          : 'Imagen descargada. Iniciando, esperando conexión...'
      },
    )
    emulatorStatus.value = 'Iniciando, esperando conexión...'
    const startupStatus = await invoke<string>('start_android_emulator')
    if (!isAndroidConnected.value) emulatorStatus.value = startupStatus
    emulatorError.value = ''
  } catch (error) {
    emulatorStatus.value = 'No se pudo iniciar el emulador.'
    emulatorError.value = String(error)
  } finally {
    isDownloadingImage.value = false
  }
}

function onAdbConnected() {
  isAndroidConnected.value = true
  emulatorStatus.value = 'Android conectado.'
  queueInput('adb_set_dark_mode', { isDark: appStore.isDarkMode })
}

onMounted(() => {
  window.addEventListener('blur', releaseInputs)
  emulatorStatus.value = 'Iniciando el emulador y esperando la conexión...'
  void startAndroidEmulator()
})
onUnmounted(() => {
  window.removeEventListener('blur', releaseInputs)
  frameResizeObserver?.disconnect()
  unlistenDownloadProgress?.()
  if (statusDismissTimer) clearTimeout(statusDismissTimer)
  if (errorDismissTimer) clearTimeout(errorDismissTimer)
  releaseInputs()
})
</script>

<template>
  <div class="home-shell">

    <main class="workspace">
      <div class="floating-messages" :class="{ 'floating-messages-dark': appStore.isDarkMode }" aria-live="polite">
        <div v-if="emulatorStatus && !isStatusDismissed" class="emulator-status" role="status">
          <span>{{ emulatorStatus }}</span>
          <progress v-if="isDownloadingImage" class="image-download-progress"
            :value="imageDownloadPercentage ?? undefined" :max="100"
            aria-label="Progreso de descarga de la imagen Android" />
          <span v-if="isDownloadingImage && imageDownloadPercentage !== null">
            {{ imageDownloadPercentage }}%
          </span>
          <button class="message-dismiss" type="button" aria-label="Descartar mensaje"
            @click="isStatusDismissed = true">
            <X :size="16" aria-hidden="true" />
          </button>
        </div>
        <div v-if="emulatorError && !isErrorDismissed" class="emulator-status emulator-status-error" role="alert">
          <span>{{ emulatorError }}</span>
          <button class="message-dismiss" type="button" aria-label="Descartar mensaje" @click="isErrorDismissed = true">
            <X :size="16" aria-hidden="true" />
          </button>
        </div>
      </div>

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
            <strong>Esperando conexión</strong>
            <span>La aplicación se conectará automáticamente cuando Android termine de iniciar.</span>
          </div>
          <div v-else class="display-shell">
            <div ref="displayFrame" class="display-frame" tabindex="0" aria-label="Pantalla interactiva de Android"
              @keydown="onKeyDown" @keyup="onKeyUp">
              <!-- <div class="screen-header">
                <span class="screen-pill">Android</span>
                <span class="screen-live"><i></i>LIVE</span>
              </div> -->
              <canvas ref="displayCanvas" :width="frameSize.width" :height="frameSize.height" role="img"
                aria-label="Pantalla de Android" @pointerdown="onPointerDown" @pointermove="onPointerMove"
                @pointerup="onPointerUp" @pointercancel="onPointerCancel" @wheel.prevent="onWheel"
                @contextmenu.prevent />
              <!--<span>{{ frameSize.width }} x {{ frameSize.height }} · {{ fps }} FPS</span>-->
              <span>{{ fps }} FPS</span>
              <small v-if="inputError" class="input-error" role="alert">{{ inputError }}</small>
              <small v-if="frameError" class="input-error" role="alert">{{ frameError }}</small>
            </div>
            <button class="hardware-toggle" type="button"
              :aria-label="showHardwareControls ? 'Ocultar controles físicos' : 'Mostrar controles físicos'"
              :aria-expanded="showHardwareControls" aria-controls="android-hardware-controls"
              :title="showHardwareControls ? 'Ocultar controles físicos' : 'Mostrar controles físicos'"
              @click="showHardwareControls = !showHardwareControls">
              <Ellipsis :size="21" aria-hidden="true" v-if="!showHardwareControls" />
              <EllipsisVertical :size="21" aria-hidden="true" v-else="showHardwareControls" />
            </button>
            <div v-if="showHardwareControls" id="android-hardware-controls" class="hardware-controls"
              :class="`hardware-controls-${hardwareControlsPosition}`" role="group"
              aria-label="Controles físicos de Android">
              <div class="hardware-position" role="group" aria-label="Elegir posición de los botones">
                <button class="position-arrow" :class="{ 'position-arrow-active': hardwareControlsPosition === 'top' }"
                  type="button" aria-label="Colocar arriba" title="Colocar arriba"
                  :aria-pressed="hardwareControlsPosition === 'top'" @click="hardwareControlsPosition = 'top'">
                  <ArrowUp :size="16" aria-hidden="true" />
                </button>
                <button class="position-arrow" :class="{ 'position-arrow-active': hardwareControlsPosition === 'left' }"
                  type="button" aria-label="Colocar a la izquierda" title="Colocar a la izquierda"
                  :aria-pressed="hardwareControlsPosition === 'left'" @click="hardwareControlsPosition = 'left'">
                  <ArrowLeft :size="16" aria-hidden="true" />
                </button>
                <span class="position-indicator" aria-hidden="true" />
                <button class="position-arrow"
                  :class="{ 'position-arrow-active': hardwareControlsPosition === 'right' }" type="button"
                  aria-label="Colocar a la derecha" title="Colocar a la derecha"
                  :aria-pressed="hardwareControlsPosition === 'right'" @click="hardwareControlsPosition = 'right'">
                  <ArrowRight :size="16" aria-hidden="true" />
                </button>
                <button class="position-arrow"
                  :class="{ 'position-arrow-active': hardwareControlsPosition === 'bottom' }" type="button"
                  aria-label="Colocar abajo" title="Colocar abajo" :aria-pressed="hardwareControlsPosition === 'bottom'"
                  @click="hardwareControlsPosition = 'bottom'">
                  <ArrowDown :size="16" aria-hidden="true" />
                </button>
              </div>
              <div class="hardware-control-buttons"
                :class="{ 'hardware-control-buttons-horizontal': hardwareControlsPosition === 'top' || hardwareControlsPosition === 'bottom' }">
                <button class="hardware-control" type="button" aria-label="Atrás" title="Atrás"
                  @click="pressAndroidKey(4)">
                  <ArrowLeft :size="19" aria-hidden="true" />
                  <span>Atrás</span>
                </button>
                <button class="hardware-control" type="button" aria-label="Inicio" title="Inicio"
                  @click="pressAndroidKey(3)">
                  <House :size="19" aria-hidden="true" />
                  <span>Inicio</span>
                </button>
                <button class="hardware-control" type="button" aria-label="Aplicaciones recientes"
                  title="Aplicaciones recientes" @click="pressAndroidKey(187)">
                  <PanelsTopLeft :size="19" aria-hidden="true" />
                  <span>Recientes</span>
                </button>
                <button class="hardware-control" type="button" aria-label="Encendido" title="Encendido"
                  @click="pressAndroidKey(26)">
                  <Power :size="19" aria-hidden="true" />
                  <span>Encendido</span>
                </button>
                <button class="hardware-control" type="button" aria-label="Subir volumen" title="Subir volumen"
                  @click="pressAndroidKey(24)">
                  <Volume2 :size="19" aria-hidden="true" />
                  <span>Volumen +</span>
                </button>
                <button class="hardware-control" type="button" aria-label="Bajar volumen" title="Bajar volumen"
                  @click="pressAndroidKey(25)">
                  <Volume1 :size="19" aria-hidden="true" />
                  <span>Volumen −</span>
                </button>
                <button class="hardware-control" type="button"
                  :aria-label="appStore.isDarkMode ? 'Activar modo claro' : 'Activar modo oscuro'"
                  :title="appStore.isDarkMode ? 'Activar modo claro' : 'Activar modo oscuro'"
                  :aria-pressed="appStore.isDarkMode" @click="appStore.toggleDarkMode()">
                  <Sun v-if="appStore.isDarkMode" :size="19" aria-hidden="true" />
                  <Moon v-else :size="19" aria-hidden="true" />
                  <span v-if="appStore.isDarkMode">Modo claro</span>
                  <span v-else>Modo oscuro</span>
                </button>
              </div>
            </div>
          </div>
        </section>

        <aside class="control-column" style="display:none">
          <AdbConnection @frame="updateFrame" @connected="onAdbConnected" />
        </aside>
      </div>
    </main>
  </div>
</template>

<style scoped>
.home-shell {
  position: relative;
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  background: var(--canaima-bg-light);
}

.workspace {
  width: 100%;
  height: 100%;
  margin: 0;
  padding: 0;
}

.floating-messages {
  position: fixed;
  z-index: 10;
  right: 1rem;
  top: 1rem;
  display: flex;
  width: min(26rem, calc(100vw - 2rem));
  flex-direction: column;
  align-items: stretch;
  gap: 0.5rem;
  pointer-events: none;
}

.image-download-progress {
  display: block;
  width: 100%;
  height: 0.6rem;
  margin-top: 0.75rem;
  accent-color: var(--canaima-primary);
}

.emulator-status {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.25rem 0.75rem;
  padding: 0.75rem 1rem;
  border: 1px solid rgb(216 225 228 / 90%);
  border-radius: 8px;
  background: var(--canaima-surface-light);
  box-shadow: 0 4px 18px rgb(0 0 0 / 18%);
  color: var(--canaima-text-dark);
  overflow-wrap: anywhere;
  backdrop-filter: blur(8px);
}

.floating-messages-dark .emulator-status {
  border-color: #34444a;
  background: var(--canaima-surface-dark);
  color: var(--canaima-text-light);
}

.emulator-status-error {
  border-color: #d99a9a;
  background: rgb(255 245 245 / 96%);
  color: #8c2828;
}

.floating-messages-dark .emulator-status-error {
  border-color: #794b4b;
  background: #361e20;
  color: #ffcaca;
}

.message-dismiss {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  justify-content: center;
  width: 1.75rem;
  height: 1.75rem;
  margin-left: auto;
  padding: 0;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: currentColor;
  cursor: pointer;
  pointer-events: auto;
}

.message-dismiss:hover {
  background: rgb(0 0 0 / 8%);
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
  width: 100%;
  height: 100%;
}

.display-panel,
.settings-panel {
  border: 1px solid #dce5e8;
  border-radius: 8px;
  background: var(--canaima-surface-light);
}

.display-panel {
  position: relative;
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  border: 0;
  border-radius: 0;
  background: #0b1216;
}

.panel-heading {
  display: none;
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
  position: absolute;
  inset: 0;
  align-items: center;
  justify-content: center;
  flex-direction: column;
  gap: 0.7rem;
  padding: 2rem;
  background: #0b1216;
  color: #87a0a8;
  text-align: center;
}

.display-shell {
  position: relative;
  width: 100%;
  height: 100%;
}

.hardware-controls {
  position: absolute;
  z-index: 4;
  top: 50%;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  transform: translateY(-50%);
}

.hardware-controls-left {
  left: 1rem;
}

.hardware-controls-right {
  right: 1rem;
}

.hardware-controls-top,
.hardware-controls-bottom {
  left: 50%;
  align-items: center;
  flex-direction: row;
  transform: translateX(-50%);
}

.hardware-controls-top {
  top: 1rem;
}

.hardware-controls-bottom {
  top: auto;
  bottom: 1rem;
}

.hardware-position {
  display: grid;
  grid-template: repeat(3, 2rem) / repeat(3, 2rem);
  align-items: center;
  justify-items: center;
  padding: 0.25rem;
  border: 1px solid rgb(255 255 255 / 22%);
  border-radius: 0.75rem;
  background: rgb(12 22 27 / 88%);
  backdrop-filter: blur(8px);
}

.position-arrow {
  display: inline-flex;
  width: 1.9rem;
  height: 1.9rem;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 0;
  border-radius: 0.45rem;
  background: transparent;
  color: #f1f7f8;
  cursor: pointer;
  touch-action: manipulation;
}

.position-arrow:nth-child(1) {
  grid-area: 1 / 2;
}

.position-arrow:nth-child(2) {
  grid-area: 2 / 1;
}

.position-arrow:nth-child(4) {
  grid-area: 2 / 3;
}

.position-arrow:nth-child(5) {
  grid-area: 3 / 2;
}

.position-arrow:hover,
.position-arrow-active {
  background: rgb(255 255 255 / 18%);
}

.position-arrow:focus-visible {
  outline: 2px solid var(--canaima-primary);
  outline-offset: 1px;
}

.position-indicator {
  grid-area: 2 / 2;
  width: 0.55rem;
  height: 0.55rem;
  border-radius: 50%;
  background: rgb(241 247 248 / 72%);
}

.hardware-control-buttons {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.hardware-control-buttons-horizontal {
  flex-direction: row;
}

.hardware-toggle {
  position: absolute;
  z-index: 4;
  top: 1rem;
  right: 1rem;
  display: inline-flex;
  width: 3.25rem;
  height: 3.25rem;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid rgb(255 255 255 / 24%);
  border-radius: 50%;
  background: rgb(12 22 27 / 88%);
  color: #f1f7f8;
  cursor: pointer;
  touch-action: manipulation;
  backdrop-filter: blur(8px);
}

.hardware-toggle:hover {
  background: rgb(29 49 57 / 96%);
}

.hardware-toggle:focus-visible {
  outline: 2px solid var(--canaima-primary);
  outline-offset: 2px;
}

.hardware-control {
  display: flex;
  min-width: 3.25rem;
  min-height: 3.25rem;
  align-items: center;
  justify-content: center;
  flex-direction: column;
  gap: 0.2rem;
  padding: 0.45rem;
  border: 1px solid rgb(255 255 255 / 22%);
  border-radius: 0.6rem;
  background: rgb(12 22 27 / 82%);
  color: #f1f7f8;
  cursor: pointer;
  touch-action: manipulation;
  backdrop-filter: blur(8px);
}

.hardware-control:hover {
  background: rgb(29 49 57 / 94%);
}

.hardware-control:focus-visible {
  outline: 2px solid var(--canaima-primary);
  outline-offset: 2px;
}

.hardware-control span {
  font-size: 0.62rem;
  line-height: 1;
  white-space: nowrap;
}

html:not(.dark) & .hardware-control,
html:not(.dark) & .hardware-toggle,
html:not(.dark) & .hardware-position {
  border-color: rgb(255 255 255 / 82%);
  background: rgb(239 246 248 / 94%);
  color: #263b43;
}

html:not(.dark) & .hardware-control:hover,
html:not(.dark) & .hardware-toggle:hover,
html:not(.dark) & .position-arrow:hover,
html:not(.dark) & .position-arrow-active {
  background: #d8e8ec;
}

html:not(.dark) & .position-arrow {
  color: #263b43;
}

html:not(.dark) & .position-indicator {
  background: #526a72;
}

.display-frame {
  position: relative;
  display: flex;
  width: 100%;
  height: 100%;
  align-items: center;
  justify-content: center;
  padding: 0;
  background: #0b1216;
  cursor: crosshair;
  outline: none;
  touch-action: none;
  user-select: none;
  border: 0;
  border-radius: 0;
  box-shadow: none;
  overflow: hidden;
}

.screen-header {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  z-index: 2;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.55rem 0.7rem;
  background: linear-gradient(180deg, rgba(17, 26, 30, 0.75), rgba(17, 26, 30, 0.2));
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  pointer-events: none;
}

.screen-pill,
.screen-live {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.2rem 0.5rem;
  border-radius: 999px;
  letter-spacing: 0.08em;
  font-size: 0.6rem;
  font-weight: 700;
  text-transform: uppercase;
}

.screen-pill {
  background: rgba(255, 255, 255, 0.08);
  color: #edf5f6;
}

.screen-live {
  background: rgba(47, 180, 117, 0.12);
  color: #a7f3c7;
}

.screen-live i {
  display: inline-block;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: #4ade80;
  box-shadow: 0 0 12px rgba(74, 222, 128, 0.7);
}

.display-frame:focus-visible {
  outline: 2px solid var(--canaima-primary);
  outline-offset: -2px;
}

.display-frame canvas {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
  image-rendering: auto;
  pointer-events: auto;
  background: #0b1216;
}

.input-error {
  position: absolute;
  z-index: 3;
  bottom: 1rem;
  left: 1rem;
  max-width: calc(100% - 1rem);
  padding: 0.4rem 0.6rem;
  background: #7b2525;
  color: #fff;
  overflow-wrap: anywhere;
}

.display-frame>span {
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
  color: #d3e0e4;
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
  background: #0b1216;
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

}
</style>
