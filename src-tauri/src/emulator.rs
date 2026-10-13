use serde::Serialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, Emitter, State};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpStream,
    process::Command,
    sync::Mutex,
    time::{sleep, timeout},
};

const IMAGE_FILE_NAME: &str = "android.qcow2";
const IMAGE_URL_ENV: &str = "ANDROID_QCOW2_URL";
const DEFAULT_IMAGE_URL: &str = "http://localhost:8888/android.qcow2";
const IMAGE_PROGRESS_EVENT: &str = "emulator-download-progress";

#[derive(Clone, Serialize)]
struct DownloadProgress {
    percentage: Option<u8>,
}

pub struct EmulatorAppState {
    started: Mutex<bool>,
}

impl EmulatorAppState {
    pub fn new() -> Self {
        Self {
            started: Mutex::new(false),
        }
    }
}

fn config_image_path() -> Result<PathBuf, String> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| "No se encontró el directorio de configuración del usuario.".to_string())?;

    Ok(config_home
        .join("canaima-android-emu")
        .join(IMAGE_FILE_NAME))
}

fn validate_image(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("No se pudo consultar la imagen {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(format!(
            "La imagen {} no es un archivo válido o está vacía.",
            path.display()
        ));
    }
    Ok(())
}

async fn download_image(app: &AppHandle, url: &str, destination: &Path) -> Result<(), String> {
    let temporary_path = destination.with_extension("qcow2.part");
    app.emit(IMAGE_PROGRESS_EVENT, DownloadProgress { percentage: None })
        .map_err(|error| format!("No se pudo informar el inicio de la descarga: {error}"))?;

    let mut child = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--show-error",
            "--progress-bar",
            "--output",
        ])
        .arg(&temporary_path)
        .arg(url)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("No se pudo iniciar la descarga con curl: {error}"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "No se pudo leer el progreso de curl.".to_string())?;
    let mut stderr = BufReader::new(stderr);
    let mut output = Vec::new();
    let mut last_percentage = None;
    let mut progress_line = Vec::new();
    loop {
        progress_line.clear();
        let bytes_read = stderr
            .read_until(b'\r', &mut progress_line)
            .await
            .map_err(|error| format!("No se pudo leer el progreso de descarga: {error}"))?;
        if bytes_read == 0 {
            break;
        }
        output.extend_from_slice(&progress_line);
        let line = String::from_utf8_lossy(&progress_line);
        let percentage = line.split_whitespace().find_map(|part| {
            part.strip_suffix('%')
                .and_then(|value| value.parse::<f32>().ok())
                .map(|value| value.round().clamp(0.0, 100.0) as u8)
        });
        if percentage.is_some() && percentage != last_percentage {
            app.emit(IMAGE_PROGRESS_EVENT, DownloadProgress { percentage })
                .map_err(|error| format!("No se pudo informar el progreso de descarga: {error}"))?;
            last_percentage = percentage;
        }
    }
    let status = child
        .wait()
        .await
        .map_err(|error| format!("No se pudo finalizar curl: {error}"))?;

    if !status.success() {
        let _ = fs::remove_file(&temporary_path);
        let details = String::from_utf8_lossy(&output).trim().to_string();
        return Err(if details.is_empty() {
            format!("No se pudo descargar la imagen desde {url}.")
        } else {
            format!("No se pudo descargar la imagen: {details}")
        });
    }

    if let Err(error) = validate_image(&temporary_path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("La descarga no produjo una imagen válida: {error}"));
    }

    fs::rename(&temporary_path, destination).map_err(|error| {
        let _ = fs::remove_file(&temporary_path);
        format!(
            "No se pudo guardar la imagen en {}: {error}",
            destination.display()
        )
    })?;
    app.emit(
        IMAGE_PROGRESS_EVENT,
        DownloadProgress {
            percentage: Some(100),
        },
    )
    .map_err(|error| format!("No se pudo informar el fin de la descarga: {error}"))?;
    Ok(())
}

async fn ensure_image(app: &AppHandle) -> Result<(PathBuf, String), String> {
    let config_image = config_image_path()?;
    let config_dir = config_image
        .parent()
        .ok_or_else(|| "No se pudo determinar el directorio de la imagen.".to_string())?;
    fs::create_dir_all(config_dir)
        .map_err(|error| format!("No se pudo crear {}: {error}", config_dir.display()))?;

    if config_image.exists() {
        validate_image(&config_image)?;
        return Ok((
            config_image.clone(),
            format!("Imagen: {}", config_image.display()),
        ));
    }

    let url = env::var(IMAGE_URL_ENV).unwrap_or_else(|_| DEFAULT_IMAGE_URL.to_string());
    if url.trim().is_empty() {
        return Err(format!(
            "No se encontró la imagen y {IMAGE_URL_ENV} está vacío; configura una URL."
        ));
    }

    download_image(app, &url, &config_image).await?;
    Ok((
        config_image.clone(),
        format!("Imagen descargada en {}", config_image.display()),
    ))
}

fn qemu_is_running(pid_file: &Path, image_path: &Path) -> Result<bool, String> {
    let pid = match fs::read_to_string(pid_file) {
        Ok(pid) => pid,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "No se pudo leer el PID de QEMU en {}: {error}",
                pid_file.display()
            ))
        }
    };
    let pid = pid.trim().parse::<u32>().map_err(|error| {
        format!(
            "El archivo PID de QEMU {} no contiene un PID válido: {error}",
            pid_file.display()
        )
    })?;
    let process_file = PathBuf::from(format!("/proc/{pid}/cmdline"));
    let command_line = match fs::read(&process_file) {
        Ok(command_line) => String::from_utf8_lossy(&command_line).into_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::remove_file(pid_file).map_err(|remove_error| {
                format!(
                    "No se pudo eliminar el PID obsoleto {}: {remove_error}",
                    pid_file.display()
                )
            })?;
            return Ok(false);
        }
        Err(error) => {
            return Err(format!(
                "No se pudo comprobar el proceso QEMU {pid}: {error}"
            ))
        }
    };

    let arguments: Vec<_> = command_line.split('\0').collect();
    let is_qemu = arguments
        .first()
        .and_then(|executable| Path::new(executable).file_name())
        .is_some_and(|name| name == "qemu-system-x86_64");
    if is_qemu
        && arguments
            .iter()
            .any(|argument| Path::new(argument) == image_path)
    {
        return Ok(true);
    }

    fs::remove_file(pid_file).map_err(|error| {
        format!(
            "No se pudo eliminar el PID obsoleto {}: {error}",
            pid_file.display()
        )
    })?;
    Ok(false)
}

async fn emulator_port_is_listening() -> bool {
    timeout(
        Duration::from_millis(500),
        TcpStream::connect("127.0.0.1:5555"),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

async fn connect_adb_to_emulator() -> Result<(), String> {
    let output = Command::new("adb")
        .args(["connect", "127.0.0.1:5555"])
        .output()
        .await
        .map_err(|error| format!("No se pudo conectar ADB con QEMU: {error}"))?;
    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("ADB no pudo conectarse con QEMU: {details}"));
    }

    let devices = Command::new("adb")
        .args(["devices"])
        .output()
        .await
        .map_err(|error| format!("No se pudo verificar la conexión ADB: {error}"))?;
    if !devices.status.success() {
        let details = String::from_utf8_lossy(&devices.stderr).trim().to_string();
        return Err(format!("No se pudo verificar la conexión ADB: {details}"));
    }

    let is_connected = String::from_utf8_lossy(&devices.stdout)
        .lines()
        .any(|line| line.split_whitespace().eq(["127.0.0.1:5555", "device"]));
    if !is_connected {
        let details = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(format!(
            "QEMU escucha en el puerto 5555, pero ADB no reconoce el dispositivo: {details}"
        ));
    }
    Ok(())
}

async fn wait_for_adb_connection() -> Result<(), String> {
    loop {
        if emulator_port_is_listening().await {
            match connect_adb_to_emulator().await {
                Ok(()) => return Ok(()),
                Err(error) if error.starts_with("QEMU escucha en el puerto 5555") => {}
                Err(error) => return Err(error),
            }
        }
        sleep(Duration::from_secs(2)).await;
    }
}

#[tauri::command]
pub async fn start_android_emulator(
    app: AppHandle,
    state: State<'_, EmulatorAppState>,
) -> Result<String, String> {
    let mut started = state.started.lock().await;
    if *started {
        return Ok("El emulador ya se inició en esta sesión.".to_string());
    }

    let qemu_status = if emulator_port_is_listening().await {
        "Sesión QEMU existente reutilizada en el puerto 5555.".to_string()
    } else {
        let (image_path, image_status) = ensure_image(&app).await?;
        let config_image = config_image_path()?;
        let pid_file = config_image
            .parent()
            .ok_or_else(|| "No se pudo determinar el directorio de configuración.".to_string())?
            .join("qemu.pid");

        let _ = Command::new("adb").arg("kill-server").output().await;

        let qemu_status = if qemu_is_running(&pid_file, &image_path)? {
            "QEMU ya estaba ejecutándose.".to_string()
        } else {
            let output = Command::new("qemu-system-x86_64")
                .args([
                    "-nodefaults",
                    "-cpu",
                    "host",
                    "-m",
                    "2G",
                    "--enable-kvm",
                    "-boot",
                    "cd",
                    "-smp",
                    "2,cores=2,sockets=1",
                    "-daemonize",
                    "-pidfile",
                ])
                .arg(&pid_file)
                .arg("-hda")
                .arg(&image_path)
                .args([
                    "-netdev",
                    "user,id=user0,hostfwd=tcp:127.0.0.1:5555-:5555",
                    "-device",
                    "virtio-net-pci,netdev=user0",
                    "-vga",
                    "qxl",
                    "-display",
                    "none",
                ])
                .output()
                .await
                .map_err(|error| format!("No se pudo iniciar QEMU: {error}"))?;

            if !output.status.success() {
                let details = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(if details.is_empty() {
                    format!("QEMU terminó con el código {}.", output.status)
                } else {
                    format!("QEMU no pudo iniciar el emulador: {details}")
                });
            }
            "Android iniciado.".to_string()
        };
        format!("{qemu_status} {image_status}")
    };

    let adb_output = Command::new("adb")
        .arg("start-server")
        .output()
        .await
        .map_err(|error| format!("No se pudo iniciar ADB: {error}"))?;
    if !adb_output.status.success() {
        let details = String::from_utf8_lossy(&adb_output.stderr)
            .trim()
            .to_string();
        return Err(format!("No se pudo iniciar: {details}"));
    }

    wait_for_adb_connection().await?;
    *started = true;
    Ok(format!("{qemu_status} Android conectado."))
}

pub async fn shutdown_android_emulator() -> Result<(), String> {
    if !emulator_port_is_listening().await {
        return Ok(());
    }

    let output = Command::new("adb")
        .args(["connect", "127.0.0.1:5555"])
        .output()
        .await
        .map_err(|error| format!("No se pudo conectar ADB para apagar Android: {error}"))?;
    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "No se pudo conectar ADB para apagar Android: {details}"
        ));
    }

    let output = timeout(
        Duration::from_secs(15),
        Command::new("adb")
            .args(["-s", "127.0.0.1:5555", "shell", "poweroff"])
            .output(),
    )
    .await
    .map_err(|_| "ADB tardó demasiado en apagar Android.".to_string())?
    .map_err(|error| format!("No se pudo ejecutar adb shell poweroff: {error}"))?;
    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("No se pudo ejecutar adb shell poweroff: {details}"));
    }

    Ok(())
}
