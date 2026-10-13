# Emulador de Android

La aplicación inicia QEMU automáticamente al abrirse. Busca la imagen en este orden:

1. `android.qcow2` en `$XDG_CONFIG_HOME/canaima-android-emu` o `~/.config/canaima-android-emu`.
2. La URL definida en `ANDROID_QCOW2_URL`; en ese caso descarga la imagen al directorio de configuración.
3. Como respaldo de desarrollo, `/home/canaima/vm/android/android.qcow2`, si está disponible.

Cuando se defina la URL de distribución, configura `ANDROID_QCOW2_URL` en el entorno de la aplicación. Se requieren `curl`, `qemu-system-x86_64`, `adb` y acceso a KVM.

Si ya hay una sesión escuchando en `127.0.0.1:5555`, la aplicación la reutiliza sin abrir ni descargar una imagen. Al cerrar la ventana, intenta apagar esa instancia con `adb shell poweroff` antes de cerrar.
