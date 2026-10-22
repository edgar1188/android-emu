use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use spice_client::channels::MouseButton;
use spice_client::SpiceClientShared;
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    State,
};
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;

const SPICE_FRAME_INTERVAL: Duration = Duration::from_millis(16);

pub struct SpiceSession {
    client: SpiceClientShared,
    frame_task: JoinHandle<()>,
}

pub struct SpiceAppState {
    session: Mutex<Option<SpiceSession>>,
}

impl SpiceAppState {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }
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

fn append_normalized_surface_pixels(
    destination: &mut Vec<u8>,
    surface: &spice_client::channels::display::DisplaySurface,
) {
    let pixel_count = (surface.width * surface.height) as usize;
    let pixel_offset = destination.len();

    match surface.format {
        1 | 8 | 32 => {
            destination.extend_from_slice(&surface.data[..pixel_count * 4]);
            if matches!(surface.format, 1 | 32) {
                for alpha in destination[pixel_offset + 3..].iter_mut().step_by(4) {
                    *alpha = 255;
                }
            }
        }
        24 => {
            for source in surface.data.chunks_exact(3).take(pixel_count) {
                destination.extend_from_slice(&[source[2], source[1], source[0], 255]);
            }
        }
        16 => {
            for source in surface.data.chunks_exact(2).take(pixel_count) {
                let word = u16::from_le_bytes([source[0], source[1]]);
                let r5 = (word >> 11) & 0x1F;
                let g6 = (word >> 5) & 0x3F;
                let b5 = word & 0x1F;
                let r = (((r5 * 255) + 15) / 31) as u8;
                let g = (((g6 * 255) + 31) / 63) as u8;
                let b = (((b5 * 255) + 15) / 31) as u8;
                destination.extend_from_slice(&[r, g, b, 255]);
            }
        }
        _ => destination.extend_from_slice(&surface.data),
    }
}

#[cfg(test)]
fn normalize_surface_pixels(surface: &spice_client::channels::display::DisplaySurface) -> Vec<u8> {
    let pixel_count = (surface.width * surface.height) as usize;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    append_normalized_surface_pixels(&mut rgba, surface);
    rgba
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

    let (frame_tx, frame_rx) =
        watch::channel(None::<Arc<spice_client::channels::display::DisplaySurface>>);
    client
        .set_display_update_callback(0, move |surface| {
            frame_tx.send_replace(Some(Arc::new(surface.clone())));
        })
        .await
        .map_err(|error| format!("No se pudo registrar la actualización de vídeo: {error}"))?;

    let frame_task = tokio::spawn(async move {
        let mut frame_id = 0_u32;
        let mut frame_interval = tokio::time::interval(SPICE_FRAME_INTERVAL);
        frame_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            frame_interval.tick().await;

            let payload = {
                let snapshot = frame_rx.borrow().clone();
                let Some(surface) = snapshot.as_ref() else {
                    continue;
                };

                if validate_surface(surface).is_err() {
                    continue;
                }

                frame_id = frame_id.wrapping_add(1);
                let pixel_count = (surface.width * surface.height) as usize;
                let mut payload = Vec::with_capacity(12 + pixel_count * 4);
                payload.extend_from_slice(&frame_id.to_le_bytes());
                payload.extend_from_slice(&surface.width.to_le_bytes());
                payload.extend_from_slice(&surface.height.to_le_bytes());
                append_normalized_surface_pixels(&mut payload, surface);
                payload
            };

            if on_frame.send(InvokeResponseBody::Raw(payload)).is_err() {
                break;
            }
        }
    });

    {
        let mut session = state.session.lock().await;
        *session = Some(SpiceSession {
            client: client.clone(),
            frame_task,
        });
    }

    let event_loop_client = client.clone();
    if let Err(error) = event_loop_client.start_event_loop().await {
        if let Some(failed_session) = state.session.lock().await.take() {
            failed_session.frame_task.abort();
            failed_session.client.disconnect().await;
        }
        return Err(format!("No se pudo iniciar el vídeo SPICE: {error}"));
    }

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
    fn preserves_rgba_surface_pixels() {
        let surface = DisplaySurface {
            width: 1,
            height: 1,
            format: 8,
            data: vec![0xFF, 0x00, 0x00, 0xFF],
        };

        let pixels = normalize_surface_pixels(&surface);
        assert_eq!(pixels, vec![255, 0, 0, 255]);
    }

    #[test]
    fn forces_opaque_alpha_for_spice_xrgb_format_32() {
        let surface = DisplaySurface {
            width: 1,
            height: 1,
            format: 32,
            data: vec![0xFF, 0x00, 0x00, 0x00],
        };

        let pixels = normalize_surface_pixels(&surface);
        assert_eq!(pixels, vec![255, 0, 0, 255]);
    }

    #[test]
    fn converts_spice_xrgb_surface_to_opaque_rgba() {
        let surface = DisplaySurface {
            width: 1,
            height: 1,
            format: 1,
            data: vec![0xFF, 0x00, 0x00, 0x00],
        };

        let pixels = normalize_surface_pixels(&surface);
        assert_eq!(pixels, vec![255, 0, 0, 255]);
    }

    #[test]
    fn converts_spice_bgr24_surface_to_rgba() {
        let surface = DisplaySurface {
            width: 1,
            height: 1,
            format: 24,
            data: vec![3, 2, 1],
        };

        let pixels = normalize_surface_pixels(&surface);
        assert_eq!(pixels, vec![1, 2, 3, 255]);
    }

    #[test]
    fn converts_spice_rgb565_surface_to_rgba() {
        let surface = DisplaySurface {
            width: 1,
            height: 1,
            format: 16,
            data: 0xF800_u16.to_le_bytes().to_vec(),
        };

        let pixels = normalize_surface_pixels(&surface);
        assert_eq!(pixels, vec![255, 0, 0, 255]);
    }
}
