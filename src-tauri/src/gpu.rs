use std::fs;
use std::path::Path;

#[cfg(target_os = "linux")]
fn is_nvidia_active() -> bool {
    let pci_dir = Path::new("/sys/bus/pci/devices");
    
    let Ok(entries) = fs::read_dir(pci_dir) else { return false; };

    // Escanea los dispositivos buscando una GPU Nvidia que esté usando su driver privativo
    entries.flatten().any(|entry| {
        let path = entry.path();
        
        // 1. Verificar si es controlador gráfico (PCI class 0x03)
        let is_gpu = fs::read_to_string(path.join("class"))
            .map(|c| c.trim().starts_with("0x03"))
            .unwrap_or(false);

        // 2. Verificar si el fabricante es Nvidia (Vendor ID: 0x10de)
        let is_nvidia = fs::read_to_string(path.join("vendor"))
            .map(|v| v.trim() == "0x10de")
            .unwrap_or(false);

        // 3. Verificar si realmente está vinculado al driver privativo "nvidia" (y no apagado o en nouveau)
        let uses_nvidia_driver = fs::read_link(path.join("driver"))
            .map(|p| p.file_name().unwrap_or_default() == "nvidia")
            .unwrap_or(false);

        is_gpu && is_nvidia && uses_nvidia_driver
    })
}

#[cfg(target_os = "linux")]
pub fn setup_workarounds() {
    if is_nvidia_active() {
        #[cfg(debug_assertions)]
        println!("🔧 NVIDIA GPU activo detectado. Aplicando parches de compatibilidad de WebKit...");
        
        // Se respeta si el usuario ya configuró estas variables manualmente
        unsafe {
            if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
                std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
            }
            if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
                std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            }
        }
    } else {
        #[cfg(debug_assertions)]
        println!("🚀 Aceleración por hardware habilitada de forma segura.");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn setup_workarounds() {}