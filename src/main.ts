import { getCurrentWindow } from '@tauri-apps/api/window'
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'

import '@materializecss/materialize/dist/css/materialize.min.css'
import '@materializecss/materialize'

import './css/style.css'

// Mostrar la ventana solo cuando el DOM de Vue esté completamente listo
document.addEventListener('DOMContentLoaded', () => {
  getCurrentWindow().show()
})

const app = createApp(App)

app.use(createPinia())
app.use(router)
app.mount('#app')
