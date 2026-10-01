use fadb_scrcpy::{
    decoder::VideoDecoder,
    server::{abstract_socket_name, SCRCPY_VERSION, SERVER_JAR},
    session::{demux_packets, read_stream_header},
};
use serde::Serialize;
use std::{
    io::Write,
    net::{TcpListener, TcpStream as StdTcpStream},
    process::Stdio,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    State,
};
use tokio::{
    net::TcpStream,
    process::{Child, Command},
    sync::Mutex,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

static NEXT_SCRCPY_ID: AtomicU32 = AtomicU32::new(1);

fn next_scrcpy_id() -> u32 {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u32;
    (timestamp
        ^ std::process::id().rotate_left(13)
        ^ NEXT_SCRCPY_ID.fetch_add(1, Ordering::Relaxed))
        & 0x7fff_ffff
}

pub struct AdbAppState {
    session: Mutex<Option<AdbSession>>,
}

impl AdbAppState {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }
}

struct AdbSession {
    serial: String,
    frame_task: JoinHandle<()>,
    cancellation: CancellationToken,
    server: Child,
    forward_port: u16,
    control: ScrcpyControl,
}

#[derive(Clone)]
struct ScrcpyControl {
    stream: Arc<StdMutex<StdTcpStream>>,
    width: u16,
    height: u16,
}

impl ScrcpyControl {
    fn send(&self, payload: &[u8]) -> Result<(), String> {
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| "El canal de control scrcpy está bloqueado.".to_string())?;
        stream
            .write_all(payload)
            .map_err(|error| format!("No se pudo enviar el control scrcpy: {error}"))
    }

    fn send_key(&self, keycode: u32, down: bool, meta_state: u32) -> Result<(), String> {
        let mut payload = Vec::with_capacity(14);
        payload.extend_from_slice(&[0, if down { 0 } else { 1 }]);
        payload.extend_from_slice(&keycode.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&meta_state.to_be_bytes());
        self.send(&payload)
    }

    fn send_touch(&self, x: u32, y: u32, action: u8) -> Result<(), String> {
        let mut payload = Vec::with_capacity(32);
        payload.push(2);
        payload.push(action);
        payload.extend_from_slice(&u64::MAX.to_be_bytes());
        payload.extend_from_slice(&x.to_be_bytes());
        payload.extend_from_slice(&y.to_be_bytes());
        payload.extend_from_slice(&self.width.to_be_bytes());
        payload.extend_from_slice(&self.height.to_be_bytes());
        payload.extend_from_slice(&(if action == 1 { 0u16 } else { u16::MAX }).to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        self.send(&payload)
    }

    fn send_scroll(&self, x: u32, y: u32, hscroll: f32, vscroll: f32) -> Result<(), String> {
        let normalized = |value: f32| ((value / 16.0).clamp(-1.0, 1.0) * 32767.0) as i16;
        let mut payload = Vec::with_capacity(21);
        payload.push(3);
        payload.extend_from_slice(&x.to_be_bytes());
        payload.extend_from_slice(&y.to_be_bytes());
        payload.extend_from_slice(&self.width.to_be_bytes());
        payload.extend_from_slice(&self.height.to_be_bytes());
        payload.extend_from_slice(&normalized(hscroll).to_be_bytes());
        payload.extend_from_slice(&normalized(vscroll).to_be_bytes());
        payload.extend_from_slice(&0u32.to_be_bytes());
        self.send(&payload)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdbDevice {
    serial: String,
    model: String,
}

async fn run_adb(args: &[&str]) -> Result<std::process::Output, String> {
    tokio::time::timeout(
        Duration::from_secs(5),
        Command::new("adb").args(args).kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| "ADB tardó demasiado en responder.".to_string())?
    .map_err(|error| format!("No se pudo ejecutar ADB: {error}"))
}

#[tauri::command]
pub async fn adb_devices() -> Result<Vec<AdbDevice>, String> {
    let output = run_adb(&["devices", "-l"]).await?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let devices = String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            if fields.next()? != "device" {
                return None;
            }
            let model = fields
                .find_map(|field| field.strip_prefix("model:"))
                .unwrap_or(serial)
                .replace('_', " ");
            Some(AdbDevice {
                serial: serial.to_string(),
                model,
            })
        })
        .collect();

    Ok(devices)
}

async fn run_adb_owned(args: &[String]) -> Result<std::process::Output, String> {
    tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("adb").args(args).kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| "El comando ADB tardó demasiado en responder.".to_string())?
    .map_err(|error| format!("No se pudo ejecutar ADB: {error}"))
}

async fn connect_scrcpy_socket(port: u16, server: &mut Child) -> Result<TcpStream, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = server
            .try_wait()
            .map_err(|error| format!("No se pudo iniciar el servidor scrcpy: {error}"))?
        {
            return Err(format!("El servidor scrcpy terminó con estado {status}."));
        }

        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(stream) => return Ok(stream),
            Err(error) if tokio::time::Instant::now() < deadline => {
                let _ = error;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(error) => {
                return Err(format!("No se pudo conectar al stream scrcpy: {error}"));
            }
        }
    }
}

async fn stop_session(mut session: AdbSession) {
    session.cancellation.cancel();
    session.frame_task.abort();
    let _ = session.server.kill().await;
    let _ = session.server.wait().await;
    let args = vec![
        "forward".to_string(),
        "--remove".to_string(),
        format!("tcp:{}", session.forward_port),
    ];
    let _ = run_adb_owned(&args).await;
}

#[tauri::command]
pub async fn adb_connect(
    serial: String,
    on_frame: Channel<InvokeResponseBody>,
    state: State<'_, AdbAppState>,
) -> Result<(), String> {
    if let Some(session) = state.session.lock().await.take() {
        stop_session(session).await;
    }

    let devices = adb_devices().await?;
    if !devices.iter().any(|device| device.serial == serial) {
        return Err(format!("El dispositivo ADB {serial} no está conectado."));
    }
    let decoder = VideoDecoder::new()
        .map_err(|error| format!("No se pudo inicializar el decoder H.264: {error}"))?;

    let local_jar = std::env::temp_dir().join("canaima-scrcpy-server-v3.3.4.jar");
    std::fs::write(&local_jar, SERVER_JAR)
        .map_err(|error| format!("No se pudo preparar scrcpy-server: {error}"))?;
    let local_jar = local_jar.to_string_lossy().into_owned();
    let push_args = vec![
        "-s".to_string(),
        serial.clone(),
        "push".to_string(),
        local_jar,
        "/data/local/tmp/scrcpy-server.jar".to_string(),
    ];
    let output = run_adb_owned(&push_args).await?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let scid = next_scrcpy_id();
    let socket_name = abstract_socket_name(scid);
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("No se pudo reservar un puerto para scrcpy: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("No se pudo leer el puerto local de scrcpy: {error}"))?
        .port();
    drop(listener);

    let forward_args = vec![
        "-s".to_string(),
        serial.clone(),
        "forward".to_string(),
        format!("tcp:{port}"),
        format!("localabstract:{socket_name}"),
    ];
    let output = run_adb_owned(&forward_args).await?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let server_args = [
        SCRCPY_VERSION.to_string(),
        "log_level=warn".to_string(),
        format!("scid={scid:08x}"),
        "tunnel_forward=true".to_string(),
        "audio=false".to_string(),
        "control=true".to_string(),
        "video_codec=h264".to_string(),
        "send_device_meta=false".to_string(),
        "max_size=1920".to_string(),
        "video_bit_rate=8000000".to_string(),
        "max_fps=60".to_string(),
    ];
    let mut server_command = Command::new("adb");
    server_command
        .args([
            "-s",
            &serial,
            "shell",
            "CLASSPATH=/data/local/tmp/scrcpy-server.jar",
            "app_process",
            "/",
            "com.genymobile.scrcpy.Server",
        ])
        .args(server_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut server = match server_command.spawn() {
        Ok(server) => server,
        Err(error) => {
            let _ = run_adb_owned(&[
                "-s".to_string(),
                serial,
                "forward".to_string(),
                "--remove".to_string(),
                format!("tcp:{port}"),
            ])
            .await;
            return Err(format!("No se pudo lanzar scrcpy-server: {error}"));
        }
    };

    tokio::time::sleep(Duration::from_millis(300)).await;

    let setup_result = async {
        let mut video = connect_scrcpy_socket(port, &mut server).await?;
        let control_socket = connect_scrcpy_socket(port, &mut server).await?;
        let stream_header =
            tokio::time::timeout(Duration::from_secs(5), read_stream_header(&mut video))
                .await
                .map_err(|_| {
                    "El servidor scrcpy no envió el encabezado de vídeo a tiempo.".to_string()
                })?
                .map_err(|error| format!("No se pudo leer el stream scrcpy: {error}"))?;
        let width = u16::try_from(stream_header.metadata.width)
            .map_err(|_| "El ancho de pantalla excede el protocolo scrcpy.".to_string())?;
        let height = u16::try_from(stream_header.metadata.height)
            .map_err(|_| "El alto de pantalla excede el protocolo scrcpy.".to_string())?;
        let control_socket = control_socket
            .into_std()
            .map_err(|error| format!("No se pudo preparar el canal de control: {error}"))?;
        control_socket
            .set_nonblocking(false)
            .map_err(|error| format!("No se pudo configurar el canal de control: {error}"))?;
        control_socket
            .set_write_timeout(Some(Duration::from_millis(500)))
            .map_err(|error| format!("No se pudo configurar el timeout de control: {error}"))?;
        Ok::<_, String>((
            video,
            ScrcpyControl {
                stream: Arc::new(StdMutex::new(control_socket)),
                width,
                height,
            },
        ))
    }
    .await;

    let (mut video, control) = match setup_result {
        Ok(result) => result,
        Err(error) => {
            let _ = server.kill().await;
            let _ = server.wait().await;
            let _ = run_adb_owned(&[
                "-s".to_string(),
                serial,
                "forward".to_string(),
                "--remove".to_string(),
                format!("tcp:{port}"),
            ])
            .await;
            return Err(error);
        }
    };

    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let frame_task = tokio::spawn(async move {
        let mut decoder = decoder;
        let mut frame_id = 0u32;
        let mut send_frame =
            |_: &fadb_scrcpy::protocol::PacketHeader,
             _: &[u8],
             frame: Option<fadb_scrcpy::decoder::RgbaFrame>| {
                let Some(frame) = frame else { return };
                let Ok(width) = u32::try_from(frame.width) else {
                    return;
                };
                let Ok(height) = u32::try_from(frame.height) else {
                    return;
                };
                let mut bytes = Vec::with_capacity(12 + frame.rgba.len());
                bytes.extend_from_slice(&frame_id.to_le_bytes());
                bytes.extend_from_slice(&width.to_le_bytes());
                bytes.extend_from_slice(&height.to_le_bytes());
                bytes.extend_from_slice(&frame.rgba);
                frame_id = frame_id.wrapping_add(1);
                if on_frame.send(InvokeResponseBody::Raw(bytes)).is_err() {
                    task_cancellation.cancel();
                }
            };
        let _ = demux_packets(
            &mut video,
            &mut decoder,
            &task_cancellation,
            &mut send_frame,
        )
        .await;
    });

    *state.session.lock().await = Some(AdbSession {
        serial,
        frame_task,
        cancellation,
        server,
        forward_port: port,
        control,
    });
    Ok(())
}

#[tauri::command]
pub async fn adb_disconnect(state: State<'_, AdbAppState>) -> Result<(), String> {
    if let Some(session) = state.session.lock().await.take() {
        stop_session(session).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn scrcpy_input_key(
    keycode: u32,
    down: bool,
    meta_state: u32,
    state: State<'_, AdbAppState>,
) -> Result<(), String> {
    let control = state
        .session
        .lock()
        .await
        .as_ref()
        .map(|session| session.control.clone())
        .ok_or_else(|| "No hay un dispositivo scrcpy conectado.".to_string())?;
    control.send_key(keycode, down, meta_state)
}

#[tauri::command]
pub async fn scrcpy_input_touch(
    x: u32,
    y: u32,
    action: String,
    state: State<'_, AdbAppState>,
) -> Result<(), String> {
    let control = state
        .session
        .lock()
        .await
        .as_ref()
        .map(|session| session.control.clone())
        .ok_or_else(|| "No hay un dispositivo scrcpy conectado.".to_string())?;
    let action = match action.as_str() {
        "down" => 0,
        "up" => 1,
        "move" => 2,
        _ => return Err("Acción táctil scrcpy desconocida.".to_string()),
    };
    control.send_touch(x, y, action)
}

#[tauri::command]
pub async fn scrcpy_input_scroll(
    x: u32,
    y: u32,
    hscroll: f32,
    vscroll: f32,
    state: State<'_, AdbAppState>,
) -> Result<(), String> {
    let control = state
        .session
        .lock()
        .await
        .as_ref()
        .map(|session| session.control.clone())
        .ok_or_else(|| "No hay un dispositivo scrcpy conectado.".to_string())?;
    control.send_scroll(x, y, hscroll, vscroll)
}

async fn run_input(state: State<'_, AdbAppState>, input: &[String]) -> Result<(), String> {
    let session = state.session.lock().await;
    let serial = session
        .as_ref()
        .map(|session| session.serial.clone())
        .ok_or_else(|| "No hay un dispositivo ADB conectado.".to_string())?;
    drop(session);

    let mut args = vec![
        "-s".to_string(),
        serial,
        "shell".to_string(),
        "input".to_string(),
    ];
    args.extend_from_slice(input);
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new("adb").args(args).kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| "El comando de entrada ADB expiró.".to_string())?
    .map_err(|error| format!("No se pudo ejecutar la entrada ADB: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[tauri::command]
pub async fn adb_input_tap(x: u32, y: u32, state: State<'_, AdbAppState>) -> Result<(), String> {
    run_input(state, &["tap".into(), x.to_string(), y.to_string()]).await
}

#[tauri::command]
pub async fn adb_input_swipe(
    start_x: u32,
    start_y: u32,
    end_x: u32,
    end_y: u32,
    duration_ms: u32,
    state: State<'_, AdbAppState>,
) -> Result<(), String> {
    run_input(
        state,
        &[
            "swipe".into(),
            start_x.to_string(),
            start_y.to_string(),
            end_x.to_string(),
            end_y.to_string(),
            duration_ms.clamp(1, 1000).to_string(),
        ],
    )
    .await
}

#[tauri::command]
pub async fn adb_input_key(keycode: u32, state: State<'_, AdbAppState>) -> Result<(), String> {
    run_input(state, &["keyevent".into(), keycode.to_string()]).await
}
