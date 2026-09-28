use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::Serialize;
use spice_client::channels::MouseButton;
use spice_client::SpiceClientShared;
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    State,
};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::SpiceAppState;

pub struct SpiceSession {
    client: SpiceClientShared,
    frame_task: JoinHandle<()>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpiceProbeResult {
    pub endpoint: String,
    pub transport: String,
    pub magic: String,
    pub major_version: u32,
    pub minor_version: u32,
    pub message_size: u32,
}

fn read_header<R: Read>(
    mut stream: R,
    endpoint: &str,
    transport: &str,
) -> Result<SpiceProbeResult, String> {
    let mut header = [0_u8; 16];
    stream
        .read_exact(&mut header)
        .map_err(|error| format!("No se pudo leer la cabecera SPICE: {error}"))?;

    let magic = String::from_utf8_lossy(&header[..4]).to_string();
    if magic != "REDQ" {
        return Err(format!(
            "El endpoint no respondió con una cabecera SPICE válida (magic: {magic:?})"
        ));
    }

    Ok(SpiceProbeResult {
        endpoint: endpoint.to_string(),
        transport: transport.to_string(),
        magic,
        major_version: u32::from_le_bytes(header[4..8].try_into().unwrap()),
        minor_version: u32::from_le_bytes(header[8..12].try_into().unwrap()),
        message_size: u32::from_le_bytes(header[12..16].try_into().unwrap()),
    })
}

fn send_client_header<W: Write>(mut stream: W) -> Result<(), String> {
    let mut header = Vec::with_capacity(16);
    header.extend_from_slice(b"REDQ");
    header.extend_from_slice(&2_u32.to_le_bytes());
    header.extend_from_slice(&2_u32.to_le_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());

    stream
        .write_all(&header)
        .map_err(|error| format!("No se pudo enviar la cabecera SPICE: {error}"))
}

fn tcp_endpoint(endpoint: &str) -> &str {
    endpoint.strip_prefix("tcp://").unwrap_or(endpoint)
}

#[tauri::command]
pub fn spice_probe(endpoint: String, timeout_ms: Option<u64>) -> Result<SpiceProbeResult, String> {
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(1500));

    if let Some(path) = endpoint.strip_prefix("unix:") {
        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;

            let stream = UnixStream::connect(path)
                .map_err(|error| format!("No se pudo conectar al socket Unix SPICE: {error}"))?;
            stream
                .set_read_timeout(Some(timeout))
                .map_err(|error| format!("No se pudo configurar el timeout SPICE: {error}"))?;
            send_client_header(&stream)?;
            return read_header(stream, &endpoint, "unix");
        }

        #[cfg(not(unix))]
        {
            let _ = path;
            return Err("Los sockets Unix SPICE no están disponibles en esta plataforma".into());
        }
    }

    let address = tcp_endpoint(&endpoint);
    let socket_address = address
        .to_socket_addrs()
        .map_err(|error| format!("Endpoint TCP inválido: {error}"))?
        .next()
        .ok_or_else(|| "El endpoint TCP no tiene una dirección resoluble".to_string())?;
    let stream = TcpStream::connect_timeout(&socket_address, timeout)
        .map_err(|error| format!("No se pudo conectar al endpoint TCP SPICE: {error}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| format!("No se pudo configurar el timeout SPICE: {error}"))?;

    send_client_header(&stream)?;
    read_header(stream, &endpoint, "tcp")
}

fn parse_tcp_endpoint(endpoint: &str) -> Result<(String, u16), String> {
    let address = tcp_endpoint(endpoint.trim());
    let (host, port) = address
        .rsplit_once(':')
        .ok_or_else(|| "El endpoint SPICE debe tener el formato host:puerto".to_string())?;
    let port = port
        .parse::<u16>()
        .map_err(|_| "El puerto SPICE no es válido".to_string())?;
    Ok((host.to_string(), port))
}

fn validate_surface(
    surface: &spice_client::channels::display::DisplaySurface,
) -> Result<(), String> {
    if surface.width == 0 || surface.height == 0 {
        return Err("La superficie SPICE no tiene dimensiones válidas.".into());
    }

    if surface.data.is_empty() {
        return Err("La superficie SPICE está vacía y no contiene píxeles.".into());
    }

    let expected_len = match surface.format {
        1 | 8 | 32 => (surface.width * surface.height * 4) as usize,
        24 => (surface.width * surface.height * 3) as usize,
        16 => (surface.width * surface.height * 2) as usize,
        _ => (surface.width * surface.height) as usize,
    };

    if surface.data.len() < expected_len {
        return Err(
            "La superficie SPICE no tiene suficientes bytes de píxeles para la resolución actual."
                .into(),
        );
    }

    Ok(())
}

fn normalize_surface_pixels(surface: &spice_client::channels::display::DisplaySurface) -> Vec<u8> {
    let pixel_count = (surface.width * surface.height) as usize;

    match surface.format {
        1 | 8 | 32 => {
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for chunk in surface.data.chunks_exact(4) {
                let [b, g, r, a] = [chunk[0], chunk[1], chunk[2], chunk[3]];
                rgba.extend_from_slice(&[r, g, b, a]);
            }
            rgba
        }
        24 => {
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for chunk in surface.data.chunks_exact(3) {
                let b = chunk[0];
                let g = chunk[1];
                let r = chunk[2];
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
            rgba
        }
        16 => {
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for chunk in surface.data.chunks_exact(2) {
                let word = u16::from_le_bytes([chunk[0], chunk[1]]);
                let r5 = (word >> 11) & 0x1F;
                let g6 = (word >> 5) & 0x3F;
                let b5 = word & 0x1F;
                let r = (((r5 * 255) + 15) / 31) as u8;
                let g = (((g6 * 255) + 31) / 63) as u8;
                let b = (((b5 * 255) + 15) / 31) as u8;
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
            rgba
        }
        _ => surface.data.clone(),
    }
}

#[tauri::command]
pub async fn spice_connect(
    endpoint: String,
    on_frame: Channel<InvokeResponseBody>,
    state: State<'_, SpiceAppState>,
) -> Result<(), String> {
    {
        let mut session = state.session.lock().await;
        if let Some(previous) = session.take() {
            previous.frame_task.abort();
            previous.client.disconnect().await;
        }
    }

    let (host, port) = parse_tcp_endpoint(&endpoint)?;
    let client = SpiceClientShared::new(host, port);
    client
        .connect()
        .await
        .map_err(|error| format!("No se pudo abrir los canales SPICE: {error}"))?;

    let latest_frame = Arc::new(StdMutex::new(None));
    let frame_notify = Arc::new(Notify::new());
    let callback_frame = Arc::clone(&latest_frame);
    let callback_notify = Arc::clone(&frame_notify);
    client
        .set_display_update_callback(0, move |surface| {
            let Ok(mut latest) = callback_frame.lock() else {
                return;
            };
            *latest = Some(surface.clone());
            drop(latest);
            callback_notify.notify_one();
        })
        .await
        .map_err(|error| format!("No se pudo registrar la actualización de vídeo: {error}"))?;

    let worker_frame = Arc::clone(&latest_frame);
    let worker_notify = Arc::clone(&frame_notify);
    let frame_task = tokio::spawn(async move {
        loop {
            let notified = worker_notify.notified();
            let surface = match worker_frame.lock() {
                Ok(mut latest) => latest.take(),
                Err(poisoned) => poisoned.into_inner().take(),
            };
            let Some(surface) = surface else {
                notified.await;
                continue;
            };

            if validate_surface(&surface).is_err() {
                continue;
            }

            let rgba_pixels = normalize_surface_pixels(&surface);
            let mut payload = Vec::with_capacity(8 + rgba_pixels.len());
            payload.extend_from_slice(&surface.width.to_le_bytes());
            payload.extend_from_slice(&surface.height.to_le_bytes());
            payload.extend_from_slice(&rgba_pixels);

            if on_frame.send(InvokeResponseBody::Raw(payload)).is_err() {
                break;
            }
        }
    });

    client.start_event_loop().await.map_err(|error| {
        frame_task.abort();
        format!("No se pudo iniciar el vídeo SPICE: {error}")
    })?;

    let mut session = state.session.lock().await;
    *session = Some(SpiceSession { client, frame_task });
    Ok(())
}

#[tauri::command]
pub async fn spice_disconnect(state: State<'_, SpiceAppState>) -> Result<(), String> {
    let mut session = state.session.lock().await;
    if let Some(active) = session.take() {
        active.frame_task.abort();
        active.client.disconnect().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn spice_mouse_motion(
    x: i32,
    y: i32,
    absolute_x: u32,
    absolute_y: u32,
    buttons: u32,
    state: State<'_, SpiceAppState>,
) -> Result<(), String> {
    let session = state.session.lock().await;
    let active = session
        .as_ref()
        .ok_or_else(|| "No hay una conexión SPICE activa.".to_string())?;
    active
        .client
        .send_mouse_motion_adaptive(0, x, y, absolute_x, absolute_y, buttons)
        .await
        .map_err(|error| format!("No se pudo enviar el movimiento del puntero: {error}"))
}

#[tauri::command]
pub async fn spice_mouse_button(
    button: String,
    pressed: bool,
    state: State<'_, SpiceAppState>,
) -> Result<(), String> {
    let button = match button.as_str() {
        "left" => MouseButton::Left,
        "middle" => MouseButton::Middle,
        "right" => MouseButton::Right,
        _ => return Err("Botón del mouse SPICE no válido.".into()),
    };
    let session = state.session.lock().await;
    let active = session
        .as_ref()
        .ok_or_else(|| "No hay una conexión SPICE activa.".to_string())?;
    active
        .client
        .send_mouse_button(0, button, pressed)
        .await
        .map_err(|error| format!("No se pudo enviar el botón del mouse: {error}"))
}

#[tauri::command]
pub async fn spice_mouse_wheel(
    delta_y: i32,
    state: State<'_, SpiceAppState>,
) -> Result<(), String> {
    let session = state.session.lock().await;
    let active = session
        .as_ref()
        .ok_or_else(|| "No hay una conexión SPICE activa.".to_string())?;
    active
        .client
        .send_mouse_wheel(0, 0, delta_y)
        .await
        .map_err(|error| format!("No se pudo enviar la rueda del mouse: {error}"))
}

#[tauri::command]
pub async fn spice_key(
    scancode: u32,
    pressed: bool,
    state: State<'_, SpiceAppState>,
) -> Result<(), String> {
    let session = state.session.lock().await;
    let active = session
        .as_ref()
        .ok_or_else(|| "No hay una conexión SPICE activa.".to_string())?;
    let result = if pressed {
        active.client.send_key_down(0, scancode).await
    } else {
        active.client.send_key_up(0, scancode).await
    };
    result.map_err(|error| format!("No se pudo enviar la tecla SPICE: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{normalize_surface_pixels, read_header, validate_surface};
    use spice_client::channels::display::DisplaySurface;
    use std::io::Cursor;

    #[test]
    fn parses_spice_link_header() {
        let mut header = Vec::from(*b"REDQ");
        header.extend(2_u32.to_le_bytes());
        header.extend(1_u32.to_le_bytes());
        header.extend(42_u32.to_le_bytes());

        let result = read_header(Cursor::new(header), "test", "tcp").unwrap();

        assert_eq!(result.magic, "REDQ");
        assert_eq!(result.major_version, 2);
        assert_eq!(result.minor_version, 1);
        assert_eq!(result.message_size, 42);
    }

    #[test]
    fn rejects_non_spice_header() {
        let result = read_header(Cursor::new([0_u8; 16]), "test", "tcp");

        assert!(result.is_err());
    }

    #[test]
    fn accepts_black_surface_pixels() {
        let surface = DisplaySurface {
            width: 2,
            height: 2,
            format: 32,
            data: vec![0_u8; 16],
        };

        assert!(validate_surface(&surface).is_ok());
    }

    #[test]
    fn rejects_empty_surface() {
        let surface = DisplaySurface {
            width: 0,
            height: 0,
            format: 32,
            data: vec![],
        };

        assert!(validate_surface(&surface).is_err());
    }

    #[test]
    fn converts_bgra_to_rgba() {
        let surface = DisplaySurface {
            width: 1,
            height: 1,
            format: 32,
            data: vec![0x00, 0x00, 0xFF, 0xFF],
        };

        let pixels = normalize_surface_pixels(&surface);
        assert_eq!(pixels, vec![255, 0, 0, 255]);
    }

    #[test]
    fn converts_spice_xrgb_surface_to_rgba() {
        let surface = DisplaySurface {
            width: 1,
            height: 1,
            format: 1,
            data: vec![0x00, 0x00, 0xFF, 0x00],
        };

        let pixels = normalize_surface_pixels(&surface);
        assert_eq!(pixels, vec![255, 0, 0, 0]);
    }
}
