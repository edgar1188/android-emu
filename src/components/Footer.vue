    <script setup lang="ts">
    import { Code, Lock, Brush, Database, Star } from '@lucide/vue'

</script>

    <template>
        <footer>
            <div class="footer-container">
                <div style="flex: 1; min-width: 300px;">
                    <h5 style="margin-top: 0;">Garantía Full-Stack</h5>
                    <p style="opacity: 0.7; font-size: 0.9rem;">
                        Añade vistas en <code>src/views/</code>, maneja estado en <code>src/stores/</code> y crea
                        componentes en
                        <code>src/components/</code>.
                    </p>
                </div>
                <div style="flex: 1; min-width: 200px;">
                    <h5 style="margin-top: 0;">Stack Moderno</h5>
                    <ul class="footer-list">
                        <li><Code :size="16" /> Vite + Vue 3</li>
                        <li>
                            <Lock :size="16" /> Tauri / Rust API
                        </li>
                        <li>
                            <Brush :size="16" /> Materialize CSS
                        </li>
                        <li>
                            <Database :size="16" /> Pinia Store
                        </li>
                        <li>
                            <Star :size="16" /> Lucide Icons
                        </li>
                    </ul>
                </div>
            </div>
            <div style="text-align: center; margin-top: 2rem; opacity: 0.5; font-size: 0.8rem;">
                © 2026 Plantilla Canaima App
            </div>
        </footer>
    </template>

<style scoped>
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
</style>