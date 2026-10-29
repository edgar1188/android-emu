# Emulador de Android

La aplicación inicia QEMU automáticamente al abrirse. Busca la imagen en:

1. `android.qcow2` en `$XDG_CONFIG_HOME/canaima-android-emu` o `~/.config/canaima-android-emu`.
2. Si no existe, la descarga desde `http://localhost:8888/android.qcow2` al directorio de configuración.

La URL se puede sobrescribir con `ANDROID_QCOW2_URL`. Se requieren `curl`, `qemu-system-x86_64`, `adb` y acceso a KVM. El paquete `.deb` declara `adb` como dependencia; la aplicación también busca ADB en `PATH` y en las ubicaciones habituales de Android SDK (`ANDROID_HOME`, `ANDROID_SDK_ROOT` y `~/Android/Sdk/platform-tools`).

Si ya hay una sesión escuchando en `127.0.0.1:5555`, la aplicación la reutiliza sin abrir ni descargar una imagen. Al cerrar la ventana, intenta apagar esa instancia con `adb shell poweroff` antes de cerrar.

Durante la descarga se muestra su progreso. Después de iniciar QEMU, la aplicación espera a que Android esté disponible por ADB y se conecta automáticamente, sin requerir acciones en el panel de transporte.
