use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;

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

#[tauri::command]
pub fn spice_open_viewer(endpoint: String) -> Result<(), String> {
    let address = tcp_endpoint(endpoint.trim());
    if address.is_empty() || !address.contains(':') {
        return Err("El endpoint SPICE debe tener el formato host:puerto".into());
    }

    Command::new("remote-viewer")
        .arg(format!("spice://{address}"))
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("No se pudo iniciar remote-viewer: {error}"))
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
