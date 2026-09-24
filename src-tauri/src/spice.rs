use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::Serialize;
use spice_client::SpiceClientShared;
use tauri::State;

use crate::SpiceAppState;

pub struct SpiceSession {
    client: SpiceClientShared,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpiceFrame {
    pub width: u32,
    pub height: u32,
    pub data_url: String,
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

#[tauri::command]
pub async fn spice_connect(
    endpoint: String,
    state: State<'_, SpiceAppState>,
) -> Result<(), String> {
    let (host, port) = parse_tcp_endpoint(&endpoint)?;
    let client = SpiceClientShared::new(host, port);
    client
        .connect()
        .await
        .map_err(|error| format!("No se pudo abrir los canales SPICE: {error}"))?;
    client
        .start_event_loop()
        .await
        .map_err(|error| format!("No se pudo iniciar el vídeo SPICE: {error}"))?;

    let mut session = state.session.lock().await;
    *session = Some(SpiceSession { client });
    Ok(())
}

#[tauri::command]
pub async fn spice_disconnect(state: State<'_, SpiceAppState>) -> Result<(), String> {
    let mut session = state.session.lock().await;
    if let Some(active) = session.take() {
        active.client.disconnect().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn spice_frame(state: State<'_, SpiceAppState>) -> Result<Option<SpiceFrame>, String> {
    let session = state.session.lock().await;
    let Some(active) = session.as_ref() else {
        return Ok(None);
    };

    let Some(surface) = active.client.get_display_surface(0).await else {
        return Ok(None);
    };

    if surface.data.iter().all(|byte| *byte == 0) {
        return Err(
            "QEMU conectó SPICE, pero no entregó píxeles decodificables para la pantalla.".into(),
        );
    }

    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, surface.width, surface.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("No se pudo preparar el frame SPICE: {error}"))?;
    writer
        .write_image_data(&surface.data)
        .map_err(|error| format!("No se pudo codificar el frame SPICE: {error}"))?;
    writer
        .finish()
        .map_err(|error| format!("No se pudo finalizar el frame SPICE: {error}"))?;

    Ok(Some(SpiceFrame {
        width: surface.width,
        height: surface.height,
        data_url: format!("data:image/png;base64,{}", BASE64.encode(encoded)),
    }))
}

#[cfg(test)]
mod tests {
    use super::read_header;
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
}
