import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'

import '@materializecss/materialize/dist/css/materialize.min.css'
import '@materializecss/materialize'

import './css/style.css'

const hasTauriWindow = typeof window !== 'undefined' && Boolean((window as any).__TAURI_INTERNALS__)

if (hasTauriWindow) {
  import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
    document.addEventListener('DOMContentLoaded', () => {
      getCurrentWindow().show()
    })
  }).catch(() => undefined)
}

const app = createApp(App)

app.use(createPinia())
app.use(router)
app.mount('#app')
