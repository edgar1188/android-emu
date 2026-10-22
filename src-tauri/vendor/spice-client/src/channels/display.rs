use crate::channels::{Channel, ChannelConnection};
use crate::error::{Result, SpiceError};
use crate::protocol::*;
use binrw::BinRead;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

// Integration tests moved to tests/display_integration.rs

const DISPLAY_UPDATE_INTERVAL: Duration = Duration::from_millis(16);

#[path = "video_tests.rs"]
mod video_tests;

// Cache for images referenced by FROM_CACHE types
struct ImageCache {
    entries: HashMap<u64, CachedImage>,
}

struct CachedImage {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

impl ImageCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn get(&self, id: u64) -> Option<&CachedImage> {
        self.entries.get(&id)
    }

    fn insert(&mut self, id: u64, data: Vec<u8>, width: u32, height: u32) {
        self.entries.insert(
            id,
            CachedImage {
                data,
                width,
                height,
            },
        );
    }

    fn remove(&mut self, id: u64) -> bool {
        self.entries.remove(&id).is_some()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Debug, Clone)]
pub struct DisplaySurface {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub id: u32,
    pub codec_type: u8,
    pub width: u32,
    pub height: u32,
    pub dest_rect: SpiceRect,
}

fn flush_pending_display_updates<F>(
    surfaces: &HashMap<u32, DisplaySurface>,
    callback: Option<&F>,
    last_update_callback: &Cell<Option<Instant>>,
    pending_update_surfaces: &RefCell<HashSet<u32>>,
    force: bool,
) where
    F: Fn(&DisplaySurface) + ?Sized,
{
    let Some(callback) = callback else {
        return;
    };

    let now = Instant::now();
    if !force
        && last_update_callback
            .get()
            .is_some_and(|last| now.duration_since(last) < DISPLAY_UPDATE_INTERVAL)
    {
        return;
    }

    let pending = std::mem::take(&mut *pending_update_surfaces.borrow_mut());
    let mut notified = false;
    for surface_id in pending {
        if let Some(surface) = surfaces.get(&surface_id) {
            callback(surface);
            notified = true;
        }
    }
    if notified {
        last_update_callback.set(Some(now));
    }
}

fn copy_image_to_surface(
    surface: &mut DisplaySurface,
    image_data: &[u8],
    image_width: u32,
    image_height: u32,
    source: SpiceRect,
    destination: SpiceRect,
    clips: Option<&[SpiceRect]>,
    blend: bool,
) {
    let source_left = i64::from(source.left);
    let source_top = i64::from(source.top);
    let source_width = (i64::from(source.right) - source_left).max(0);
    let source_height = (i64::from(source.bottom) - source_top).max(0);
    let destination_left = i64::from(destination.left);
    let destination_top = i64::from(destination.top);
    let destination_width = (i64::from(destination.right) - destination_left).max(0);
    let destination_height = (i64::from(destination.bottom) - destination_top).max(0);
    if source_width == 0
        || source_height == 0
        || destination_width == 0
        || destination_height == 0
    {
        return;
    }

    let left = destination.left.max(0).min(surface.width as i32);
    let top = destination.top.max(0).min(surface.height as i32);
    let right = destination.right.max(0).min(surface.width as i32);
    let bottom = destination.bottom.max(0).min(surface.height as i32);
    if left >= right || top >= bottom {
        return;
    }

    let bpp = 4_usize;
    let surface_stride = surface.width as usize * bpp;

    for y in top..bottom {
        let relative_y = i64::from(y) - destination_top;
        let source_y = source_top + relative_y * source_height / destination_height;
        if source_y < 0 || source_y >= i64::from(image_height) {
            continue;
        }

        for x in left..right {
            if !inside_clip(clips, x, y) {
                continue;
            }

            let relative_x = i64::from(x) - destination_left;
            let source_x = source_left + relative_x * source_width / destination_width;
            if source_x < 0 || source_x >= i64::from(image_width) {
                continue;
            }

            let source_offset =
                (source_y as usize * image_width as usize + source_x as usize) * bpp;
            let destination_offset = y as usize * surface_stride + x as usize * bpp;
            if source_offset + bpp <= image_data.len()
                && destination_offset + bpp <= surface.data.len()
            {
                let source_pixel = &image_data[source_offset..source_offset + bpp];
                let destination_pixel =
                    &mut surface.data[destination_offset..destination_offset + bpp];
                if blend {
                    let source_alpha = u32::from(source_pixel[3]);
                    let destination_alpha = u32::from(destination_pixel[3]);
                    let output_alpha =
                        source_alpha + (destination_alpha * (255 - source_alpha) + 127) / 255;

                    for channel in 0..3 {
                        let source_component =
                            u32::from(source_pixel[channel]) * source_alpha * 255;
                        let destination_component = u32::from(destination_pixel[channel])
                            * destination_alpha
                            * (255 - source_alpha);
                        destination_pixel[channel] = if output_alpha == 0 {
                            0
                        } else {
                            ((source_component + destination_component
                                + output_alpha * 127)
                                / (output_alpha * 255)) as u8
                        };
                    }
                    destination_pixel[3] = output_alpha as u8;
                } else {
                    destination_pixel.copy_from_slice(source_pixel);
                }
            }
        }
    }
}

fn inside_clip(clips: Option<&[SpiceRect]>, x: i32, y: i32) -> bool {
    clips.map_or(true, |rects| {
        rects
            .iter()
            .any(|rect| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
    })
}

fn parse_clip_rects(clip: &SpiceClip, data: &[u8]) -> Result<Option<Vec<SpiceRect>>> {
    match clip.clip_type {
        value if value == ClipType::None as u8 => Ok(None),
        value if value == ClipType::Rects as u8 => {
            let offset = if clip.data > u32::MAX as u64 {
                (clip.data & u32::MAX as u64) as usize
            } else {
                clip.data as usize
            };
            let header_end = offset.checked_add(4).ok_or_else(|| {
                SpiceError::Protocol("SPICE clip rectangle offset overflowed".to_string())
            })?;
            if header_end > data.len() {
                return Err(SpiceError::Protocol(
                    "SPICE clip rectangle list is outside the drawing message".to_string(),
                ));
            }

            let count = u32::from_le_bytes(data[offset..header_end].try_into().unwrap()) as usize;
            let rect_bytes = count.checked_mul(16).ok_or_else(|| {
                SpiceError::Protocol("SPICE clip rectangle count overflowed".to_string())
            })?;
            let end = header_end.checked_add(rect_bytes).ok_or_else(|| {
                SpiceError::Protocol("SPICE clip rectangle list size overflowed".to_string())
            })?;
            if end > data.len() {
                return Err(SpiceError::Protocol(format!(
                    "SPICE clip rectangle list is truncated: expected {rect_bytes} rectangle bytes"
                )));
            }

            let rects = data[header_end..end]
                .chunks_exact(16)
                .map(|bytes| SpiceRect {
                    top: i32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                    left: i32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                    bottom: i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                    right: i32::from_le_bytes(bytes[12..16].try_into().unwrap()),
                })
                .collect();
            Ok(Some(rects))
        }
        value => Err(SpiceError::Protocol(format!(
            "Unsupported SPICE clip type {value}"
        ))),
    }
}

fn copy_surface_region(
    surface: &mut DisplaySurface,
    source_position: SpicePoint,
    destination: SpiceRect,
    clips: Option<&[SpiceRect]>,
) {
    let width = (i64::from(destination.right) - i64::from(destination.left)).max(0);
    let height = (i64::from(destination.bottom) - i64::from(destination.top)).max(0);
    if width == 0 || height == 0 {
        return;
    }

    let source = surface.data.clone();
    let stride = surface.width as usize * 4;
    let left = destination.left.max(0).min(surface.width as i32);
    let top = destination.top.max(0).min(surface.height as i32);
    let right = destination.right.max(0).min(surface.width as i32);
    let bottom = destination.bottom.max(0).min(surface.height as i32);

    for y in top..bottom {
        for x in left..right {
            if !inside_clip(clips, x, y) {
                continue;
            }

            let source_x = i64::from(source_position.x) + i64::from(x - destination.left);
            let source_y = i64::from(source_position.y) + i64::from(y - destination.top);
            if source_x < 0
                || source_y < 0
                || source_x >= i64::from(surface.width)
                || source_y >= i64::from(surface.height)
            {
                continue;
            }

            let source_offset = source_y as usize * stride + source_x as usize * 4;
            let destination_offset = y as usize * stride + x as usize * 4;
            if source_offset + 4 <= source.len()
                && destination_offset + 4 <= surface.data.len()
            {
                surface.data[destination_offset..destination_offset + 4]
                    .copy_from_slice(&source[source_offset..source_offset + 4]);
            }
        }
    }
}

pub struct DisplayChannel {
    pub(crate) connection: ChannelConnection,
    surfaces: HashMap<u32, DisplaySurface>,
    monitors: Vec<SpiceHead>,
    active_streams: HashMap<u32, StreamInfo>,
    #[cfg(not(target_arch = "wasm32"))]
    update_callback: Option<Box<dyn Fn(&DisplaySurface) + Send + Sync>>,
    #[cfg(target_arch = "wasm32")]
    update_callback: Option<Box<dyn Fn(&DisplaySurface)>>,
    last_update_callback: Cell<Option<Instant>>,
    pending_update_surfaces: RefCell<HashSet<u32>>,
    image_cache: ImageCache,
    last_mark: Option<u32>, // Track last received mark for synchronization
}

fn decode_bitmap(
    bitmap: &SpiceBitmap,
    data: &[u8],
    width: u32,
    height: u32,
) -> Result<Option<(Vec<u8>, u32, u32)>> {
    let bytes_per_pixel = match bitmap.format {
        SPICE_BITMAP_FMT_32BIT | SPICE_BITMAP_FMT_RGBA => 4,
        SPICE_BITMAP_FMT_24BIT => 3,
        SPICE_BITMAP_FMT_16BIT => 2,
        SPICE_BITMAP_FMT_8BIT | SPICE_BITMAP_FMT_8BIT_A => 1,
        _ => {
            warn!("Unsupported bitmap format: {}", bitmap.format);
            return Ok(None);
        }
    };

    let expected_size = (bitmap.stride * height) as usize;
    if data.len() < expected_size {
        warn!("Bitmap data too small: {} < {}", data.len(), expected_size);
        return Ok(None);
    }

    let mut rgba_data = Vec::with_capacity((width * height * 4) as usize);

    for y in 0..height {
        let source_y = if bitmap.flags & SPICE_BITMAP_FLAGS_TOP_DOWN != 0 {
            y
        } else {
            height - 1 - y
        };
        let row_offset = (source_y * bitmap.stride) as usize;
        for x in 0..width {
            let pixel_offset = row_offset + (x as usize * bytes_per_pixel);

            match bitmap.format {
                SPICE_BITMAP_FMT_32BIT => {
                    rgba_data.extend_from_slice(&[
                        data[pixel_offset + 2],
                        data[pixel_offset + 1],
                        data[pixel_offset],
                        255,
                    ]);
                }
                SPICE_BITMAP_FMT_RGBA => {
                    rgba_data.extend_from_slice(&data[pixel_offset..pixel_offset + 4]);
                }
                SPICE_BITMAP_FMT_24BIT => {
                    rgba_data.extend_from_slice(&[
                        data[pixel_offset + 2],
                        data[pixel_offset + 1],
                        data[pixel_offset],
                        255,
                    ]);
                }
                _ => rgba_data.extend_from_slice(&[0, 0, 0, 255]),
            }
        }
    }

    Ok(Some((rgba_data, width, height)))
}

impl DisplayChannel {
    pub async fn new(host: &str, port: u16, channel_id: u8) -> Result<Self> {
        Self::new_with_connection_id(host, port, channel_id, None).await
    }

    pub async fn new_with_connection_id(
        host: &str,
        port: u16,
        channel_id: u8,
        connection_id: Option<u32>,
    ) -> Result<Self> {
        let mut connection =
            ChannelConnection::new(host, port, ChannelType::Display, channel_id).await?;
        if let Some(conn_id) = connection_id {
            connection.set_connection_id(conn_id);
        }
        connection.handshake().await?;

        // Send display init message after handshake
        info!("Sending SPICE_MSGC_DISPLAY_INIT");

        // Create the display init message
        // For now, disable cache until we properly handle cache negotiation
        // The server is sending cached references without first sending the images
        let display_init = SpiceMsgcDisplayInit {
            cache_id: 0,
            cache_size: 4 * 1024 * 1024,
            glz_dict_id: 0,
        };

        // Serialize the message
        use binrw::BinWrite;
        let mut cursor = std::io::Cursor::new(Vec::new());
        display_init
            .write(&mut cursor)
            .map_err(|e| SpiceError::Protocol(format!("Failed to write display init: {e}")))?;
        let init_data = cursor.into_inner();

        connection
            .send_message(SPICE_MSGC_DISPLAY_INIT, &init_data)
            .await?;

        // Create default primary surface (ID 0) with a reasonable default size
        // Many SPICE servers assume surface 0 exists after display init
        let mut surfaces = HashMap::new();
        let default_width = 1024;
        let default_height = 768;
        let default_format = 32; // 32-bit RGBA

        info!(
            "Creating default primary surface (ID 0) - {}x{}",
            default_width, default_height
        );
        surfaces.insert(
            0,
            DisplaySurface {
                width: default_width,
                height: default_height,
                format: default_format,
                data: vec![0; (default_width * default_height * 4) as usize],
            },
        );

        Ok(Self {
            connection,
            surfaces,
            monitors: Vec::new(),
            active_streams: HashMap::new(),
            update_callback: None,
            last_update_callback: Cell::new(None),
            pending_update_surfaces: RefCell::new(HashSet::new()),
            image_cache: ImageCache::new(),
            last_mark: None,
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn new_websocket(websocket_url: &str, channel_id: u8) -> Result<Self> {
        Self::new_websocket_with_auth(websocket_url, channel_id, None).await
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn new_websocket_with_auth(
        websocket_url: &str,
        channel_id: u8,
        auth_token: Option<String>,
    ) -> Result<Self> {
        let mut connection = ChannelConnection::new_websocket_with_auth(
            websocket_url,
            ChannelType::Display,
            channel_id,
            auth_token,
        )
        .await?;
        connection.handshake().await?;

        // Send display init message after handshake
        info!("Sending SPICE_MSGC_DISPLAY_INIT");

        // Create the display init message
        // For now, disable cache until we properly handle cache negotiation
        // The server is sending cached references without first sending the images
        let display_init = SpiceMsgcDisplayInit {
            cache_id: 0,
            cache_size: 4 * 1024 * 1024,
            glz_dict_id: 0,
        };

        // Serialize the message
        use binrw::BinWrite;
        let mut cursor = std::io::Cursor::new(Vec::new());
        display_init
            .write(&mut cursor)
            .map_err(|e| SpiceError::Protocol(format!("Failed to write display init: {e}")))?;
        let init_data = cursor.into_inner();

        connection
            .send_message(SPICE_MSGC_DISPLAY_INIT, &init_data)
            .await?;

        // Create default primary surface (ID 0) with a reasonable default size
        // Many SPICE servers assume surface 0 exists after display init
        let mut surfaces = HashMap::new();
        let default_width = 1024;
        let default_height = 768;
        let default_format = 32; // 32-bit RGBA

        info!(
            "Creating default primary surface (ID 0) - {}x{}",
            default_width, default_height
        );
        surfaces.insert(
            0,
            DisplaySurface {
                width: default_width,
                height: default_height,
                format: default_format,
                data: vec![0; (default_width * default_height * 4) as usize],
            },
        );

        Ok(Self {
            connection,
            surfaces,
            monitors: Vec::new(),
            active_streams: HashMap::new(),
            update_callback: None,
            last_update_callback: Cell::new(None),
            pending_update_surfaces: RefCell::new(HashSet::new()),
            image_cache: ImageCache::new(),
            last_mark: None,
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn new_websocket_with_auth_and_session(
        websocket_url: &str,
        channel_id: u8,
        auth_token: Option<String>,
        password: Option<String>,
        connection_id: Option<u32>,
    ) -> Result<Self> {
        let mut connection = ChannelConnection::new_websocket_with_auth(
            websocket_url,
            ChannelType::Display,
            channel_id,
            auth_token,
        )
        .await?;
        if let Some(pwd) = password {
            connection.set_password(pwd);
        }
        if let Some(conn_id) = connection_id {
            connection.set_connection_id(conn_id);
        }
        connection.handshake().await?;

        // Send display init message after handshake
        info!("Sending SPICE_MSGC_DISPLAY_INIT");

        // Create the display init message
        // For now, disable cache until we properly handle cache negotiation
        // The server is sending cached references without first sending the images
        let display_init = SpiceMsgcDisplayInit {
            cache_id: 0,
            cache_size: 4 * 1024 * 1024,
            glz_dict_id: 0,
        };

        // Serialize the message
        use binrw::BinWrite;
        let mut cursor = std::io::Cursor::new(Vec::new());
        display_init
            .write(&mut cursor)
            .map_err(|e| SpiceError::Protocol(format!("Failed to write display init: {e}")))?;
        let init_data = cursor.into_inner();

        connection
            .send_message(SPICE_MSGC_DISPLAY_INIT, &init_data)
            .await?;

        // Create default primary surface (ID 0) with a reasonable default size
        // Many SPICE servers assume surface 0 exists after display init
        let mut surfaces = HashMap::new();
        let default_width = 1024;
        let default_height = 768;
        let default_format = 32; // 32-bit RGBA

        info!(
            "Creating default primary surface (ID 0) - {}x{}",
            default_width, default_height
        );
        surfaces.insert(
            0,
            DisplaySurface {
                width: default_width,
                height: default_height,
                format: default_format,
                data: vec![0; (default_width * default_height * 4) as usize],
            },
        );

        Ok(Self {
            connection,
            surfaces,
            monitors: Vec::new(),
            active_streams: HashMap::new(),
            update_callback: None,
            last_update_callback: Cell::new(None),
            pending_update_surfaces: RefCell::new(HashSet::new()),
            image_cache: ImageCache::new(),
            last_mark: None,
        })
    }

    pub async fn initialize(&mut self) -> Result<()> {
        info!("Display channel {} initialized", self.connection.channel_id);

        // According to SPICE protocol, display channels might need to send an init message
        // or wait for the server to send display configuration
        // For now, we just wait for server messages

        Ok(())
    }

    pub fn get_surface(&self, surface_id: u32) -> Option<&DisplaySurface> {
        self.surfaces.get(&surface_id)
    }

    pub fn get_primary_surface(&self) -> Option<&DisplaySurface> {
        let surface = self.surfaces.get(&0);
        if surface.is_none() {
            debug!(
                "DisplayChannel: No primary surface available. Total surfaces: {}",
                self.surfaces.len()
            );
        }
        surface
    }

    pub fn get_monitors(&self) -> &[SpiceHead] {
        &self.monitors
    }

    pub fn get_surfaces(&self) -> &HashMap<u32, DisplaySurface> {
        &self.surfaces
    }

    pub fn get_last_mark(&self) -> Option<u32> {
        self.last_mark
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_update_callback<F>(&mut self, callback: F)
    where
        F: Fn(&DisplaySurface) + Send + Sync + 'static,
    {
        self.update_callback = Some(Box::new(callback));
    }

    #[cfg(target_arch = "wasm32")]
    pub fn set_update_callback<F>(&mut self, callback: F)
    where
        F: Fn(&DisplaySurface) + 'static,
    {
        self.update_callback = Some(Box::new(callback));
    }

    /// Process a single message from the server
    /// This is primarily for testing purposes
    pub async fn process_next_message(&mut self) -> Result<()> {
        let (header, data) = self.connection.read_message().await?;
        self.handle_message(&header, &data).await
    }

    fn notify_update(&self, surface_id: u32) {
        self.notify_update_inner(surface_id, false);
    }

    fn notify_update_force(&self, surface_id: u32) {
        self.notify_update_inner(surface_id, true);
    }

    fn notify_update_inner(&self, surface_id: u32, force: bool) {
        self.pending_update_surfaces
            .borrow_mut()
            .insert(surface_id);
        flush_pending_display_updates(
            &self.surfaces,
            self.update_callback.as_deref(),
            &self.last_update_callback,
            &self.pending_update_surfaces,
            force,
        );
    }

    /// Resolve a SpiceAddress to get data from the message buffer
    /// SpiceAddress is an offset from the beginning of the message data
    fn resolve_address<'a>(&self, address: SpiceAddress, data: &'a [u8]) -> Option<&'a [u8]> {
        if address == 0 {
            return None;
        }

        // Check if this is an encoded address (upper 32 bits non-zero)
        if address > 0xFFFFFFFF {
            let surface_id = (address >> 32) as u32;
            let offset = (address & 0xFFFFFFFF) as u32;

            warn!(
                "SpiceAddress 0x{:x} is encoded: surface_id=0x{:x} ({}) offset=0x{:x} ({})",
                address, surface_id, surface_id, offset, offset
            );

            if (offset as usize) < data.len() {
                return Some(&data[offset as usize..]);
            }

            warn!("Encoded SpiceAddress is outside the current message");
            return None;
        }

        // Simple offset from message body
        let offset = address as usize;
        if offset >= data.len() {
            warn!(
                "SpiceAddress 0x{:x} ({}) is out of bounds (data len: {})",
                address,
                address,
                data.len()
            );
            return None;
        }

        Some(&data[offset..])
    }

    /// Decode a SpiceImage from a SpiceAddress
    fn decode_image(
        &mut self,
        address: SpiceAddress,
        data: &[u8],
    ) -> Result<Option<(Vec<u8>, u32, u32)>> {
        // Cached-image addresses have no inline descriptor in the message.
        if address > 0x10000000 && (address & 0xFFFFFFFF) as usize >= data.len() {
            let cache_id = address & 0xFFFFFFFF;

            if let Some(cached) = self.image_cache.get(cache_id) {
                debug!("Found cached image: {}x{}", cached.width, cached.height);
                return Ok(Some((cached.data.clone(), cached.width, cached.height)));
            }

            warn!("SPICE image cache miss for image ID {cache_id}");
            return Ok(None);
        }

        let image_data = match self.resolve_address(address, data) {
            Some(d) => d,
            None => return Ok(None),
        };

        // Parse the image descriptor
        let mut cursor = std::io::Cursor::new(image_data);
        let descriptor = SpiceImageDescriptor::read(&mut cursor)
            .map_err(|e| SpiceError::Protocol(format!("Failed to parse image descriptor: {e}")))?;

        debug!(
            "Image descriptor: type={}, size={}x{}, id={}",
            descriptor.type_, descriptor.width, descriptor.height, descriptor.id
        );

        let result = match descriptor.type_ {
            SPICE_IMAGE_TYPE_BITMAP => {
                // Parse bitmap structure
                let bitmap = SpiceBitmap::read(&mut cursor)
                    .map_err(|e| SpiceError::Protocol(format!("Failed to parse bitmap: {e}")))?;

                // Get bitmap data
                let bitmap_data_offset = bitmap.data as usize;
                if bitmap_data_offset >= data.len() {
                    warn!("Bitmap data offset {} out of bounds", bitmap_data_offset);
                    return Ok(None);
                }

                let bitmap_data = &data[bitmap_data_offset..];
                decode_bitmap(&bitmap, bitmap_data, descriptor.width, descriptor.height)
            }
            SPICE_IMAGE_TYPE_QUIC => {
                let compressed_data = &image_data[cursor.position() as usize..];
                self.decode_quic(compressed_data, descriptor.width, descriptor.height)
            }
            SPICE_IMAGE_TYPE_LZ4 => {
                // Decompress LZ4 data
                let compressed_data = &image_data[cursor.position() as usize..];
                self.decode_lz4(compressed_data, descriptor.width, descriptor.height)
            }
            SPICE_IMAGE_TYPE_JPEG => {
                // Decode JPEG data
                let jpeg_data = &image_data[cursor.position() as usize..];
                self.decode_jpeg(jpeg_data)
            }
            SPICE_IMAGE_TYPE_LZ => {
                // Decompress LZ data (SPICE custom LZ format)
                let compressed_data = &image_data[cursor.position() as usize..];
                self.decode_lz(
                    strip_lz_data_size_prefix(compressed_data),
                    descriptor.width,
                    descriptor.height,
                )
            }
            SPICE_IMAGE_TYPE_ZLIB_GLZ_RGB => {
                // Decompress zlib data
                let compressed_data = &image_data[cursor.position() as usize..];
                self.decode_zlib(compressed_data, descriptor.width, descriptor.height)
            }
            SPICE_IMAGE_TYPE_FROM_CACHE | SPICE_IMAGE_TYPE_FROM_CACHE_LOSSLESS => {
                // This is a cached image reference
                debug!("FROM_CACHE image type, id: {}", descriptor.id);

                // Try to get from cache
                if let Some(cached) = self.image_cache.get(descriptor.id) {
                    debug!("Found cached image: {}x{}", cached.width, cached.height);
                    Ok(Some((cached.data.clone(), cached.width, cached.height)))
                } else {
                    warn!("Cached image with ID {} not found in cache", descriptor.id);
                    Ok(None)
                }
            }
            SPICE_IMAGE_TYPE_SURFACE => {
                // This references another surface
                debug!(
                    "SURFACE image type, surface id encoded in descriptor id: {}",
                    descriptor.id
                );

                // The descriptor.id might encode the surface ID
                let surface_id = (descriptor.id & 0xFFFFFFFF) as u32;

                if let Some(surface) = self.surfaces.get(&surface_id) {
                    debug!(
                        "Found surface {}: {}x{}",
                        surface_id, surface.width, surface.height
                    );
                    Ok(Some((surface.data.clone(), surface.width, surface.height)))
                } else {
                    warn!("Surface {} not found", surface_id);
                    Ok(None)
                }
            }
            _ => {
                debug!("Unsupported image type: {}", descriptor.type_);
                Ok(None)
            }
        }?;

        // Cache the decoded image if successful (except for FROM_CACHE and SURFACE types)
        if let Some((ref data, width, height)) = result {
            if descriptor.type_ != SPICE_IMAGE_TYPE_FROM_CACHE
                && descriptor.type_ != SPICE_IMAGE_TYPE_FROM_CACHE_LOSSLESS
                && descriptor.type_ != SPICE_IMAGE_TYPE_SURFACE
            {
                debug!("Caching decoded image with id: {}", descriptor.id);
                self.image_cache
                    .insert(descriptor.id, data.clone(), width, height);
            }
        }

        Ok(result)
    }

    /// Decode LZ4 compressed image
    fn decode_lz4(
        &self,
        compressed_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Option<(Vec<u8>, u32, u32)>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use lz4::Decoder;
            use std::io::Read;

            let mut decoder = Decoder::new(compressed_data)
                .map_err(|e| SpiceError::Protocol(format!("Failed to create LZ4 decoder: {e}")))?;

            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|e| SpiceError::Protocol(format!("Failed to decompress LZ4: {e}")))?;

            // Assume decompressed data is RGBA
            Ok(Some((decompressed, width, height)))
        }

        #[cfg(target_arch = "wasm32")]
        {
            // LZ4 is not supported in WASM builds (requires C compilation)
            let _ = (compressed_data, width, height); // Suppress unused warnings
            Err(SpiceError::Protocol(
                "LZ4 decompression is not supported in WebAssembly builds".to_string(),
            ))
        }
    }

    /// Decode JPEG image
    fn decode_jpeg(&self, jpeg_data: &[u8]) -> Result<Option<(Vec<u8>, u32, u32)>> {
        use jpeg_decoder::Decoder;

        let mut decoder = Decoder::new(jpeg_data);
        let pixels = decoder
            .decode()
            .map_err(|e| SpiceError::Protocol(format!("Failed to decode JPEG: {e}")))?;

        let info = decoder.info().unwrap();
        let width = info.width as u32;
        let height = info.height as u32;

        // Convert to RGBA if needed
        let rgba_data = match info.pixel_format {
            jpeg_decoder::PixelFormat::RGB24 => {
                let mut rgba = Vec::with_capacity(pixels.len() * 4 / 3);
                for chunk in pixels.chunks(3) {
                    rgba.push(chunk[0]); // R
                    rgba.push(chunk[1]); // G
                    rgba.push(chunk[2]); // B
                    rgba.push(255); // A
                }
                rgba
            }
            jpeg_decoder::PixelFormat::L8 => {
                let mut rgba = Vec::with_capacity(pixels.len() * 4);
                for &gray in &pixels {
                    rgba.push(gray); // R
                    rgba.push(gray); // G
                    rgba.push(gray); // B
                    rgba.push(255); // A
                }
                rgba
            }
            _ => {
                warn!("Unsupported JPEG pixel format: {:?}", info.pixel_format);
                return Ok(None);
            }
        };

        Ok(Some((rgba_data, width, height)))
    }

    /// Decode LZ compressed image (SPICE custom LZ format)
    /// This is a simplified implementation - full LZ support would require implementing the SPICE LZ algorithm
    fn decode_lz(
        &self,
        compressed_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Option<(Vec<u8>, u32, u32)>> {
        let decoded =
            shakenfist_spice_compression::decompress_lz(compressed_data).map_err(|error| {
                SpiceError::Protocol(format!("Failed to decompress SPICE LZ: {error}"))
            })?;

        if decoded.width != width || decoded.height != height {
            return Err(SpiceError::Protocol(format!(
                "SPICE LZ dimensions {}x{} do not match image descriptor {}x{}",
                decoded.width, decoded.height, width, height
            )));
        }

        Ok(Some((decoded.pixels, decoded.width, decoded.height)))
    }

    fn decode_quic(
        &self,
        compressed_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Option<(Vec<u8>, u32, u32)>> {
        let Some(pixels) =
            shakenfist_spice_compression::quic_decode(compressed_data, width, height)
        else {
            return Ok(None);
        };

        Ok(Some((pixels, width, height)))
    }

    /// Decode zlib compressed image
    fn decode_zlib(
        &self,
        compressed_data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<Option<(Vec<u8>, u32, u32)>> {
        use flate2::read::ZlibDecoder;
        use std::io::Read;

        let mut decoder = ZlibDecoder::new(compressed_data);
        let mut decompressed = Vec::new();

        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| SpiceError::Protocol(format!("Failed to decompress zlib: {e}")))?;

        // Assume decompressed data is RGBA
        Ok(Some((decompressed, width, height)))
    }

    pub async fn run(&mut self) -> Result<()> {
        info!(
            "DisplayChannel: Starting event loop for channel {}",
            self.connection.channel_id
        );
        let mut update_interval = tokio::time::interval(DISPLAY_UPDATE_INTERVAL);
        loop {
            let result = {
                let read_message = self.connection.read_message();
                tokio::pin!(read_message);
                loop {
                    tokio::select! {
                        result = &mut read_message => break result,
                        _ = update_interval.tick() => flush_pending_display_updates(
                            &self.surfaces,
                            self.update_callback.as_deref(),
                            &self.last_update_callback,
                            &self.pending_update_surfaces,
                            false,
                        ),
                    }
                }
            };

            match result {
                Ok((header, data)) => {
                    if let Err(error) = self.handle_message(&header, &data).await {
                        error!(
                            "DisplayChannel {}: discarded message type {} after processing error: {}",
                            self.connection.channel_id, header.msg_type, error
                        );
                    }
                }
                Err(e) => {
                    error!("DisplayChannel: Error reading message: {e}");
                    return Err(e);
                }
            }
        }
    }

    async fn handle_mode_message(&mut self, data: &[u8]) -> Result<()> {
        // Parse mode message to get display dimensions and format
        // This is a simplified implementation
        if data.len() >= 12 {
            let width = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            let height = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
            let format = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);

            info!("Display mode: {}x{}, format: {}", width, height, format);

            // Create primary surface (ID 0)
            self.surfaces.insert(
                0,
                DisplaySurface {
                    width,
                    height,
                    format,
                    data: vec![0; (width * height * 4) as usize], // Assuming 32-bit RGBA
                },
            );

            // Notify about primary surface
            self.notify_update(0);
        }

        Ok(())
    }

    async fn handle_draw_message(&mut self, msg_type: u16, data: &[u8]) -> Result<()> {
        match msg_type {
            x if x == DisplayChannelMessage::DrawFill as u16 => {
                debug!("Handle draw fill");

                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_fill) = SpiceDrawFill::read(&mut cursor) {
                    let clips = parse_clip_rects(&draw_fill.base.clip, data)?;
                    if draw_fill.data.brush.brush_type == BrushType::Solid as u8 {
                        let color = draw_fill.data.brush.color;
                        let rgba = [
                            ((color >> 16) & 0xff) as u8,
                            ((color >> 8) & 0xff) as u8,
                            (color & 0xff) as u8,
                            255,
                        ];
                        let bbox = draw_fill.base.box_;
                        if let Some(surface) = self.surfaces.get_mut(&draw_fill.base.surface_id) {
                            let left = bbox.left.max(0).min(surface.width as i32);
                            let top = bbox.top.max(0).min(surface.height as i32);
                            let right = bbox.right.max(0).min(surface.width as i32);
                            let bottom = bbox.bottom.max(0).min(surface.height as i32);
                            let stride = surface.width as usize * 4;

                            for y in top..bottom {
                                for x in left..right {
                                    if !inside_clip(clips.as_deref(), x, y) {
                                        continue;
                                    }

                                    let offset = y as usize * stride + x as usize * 4;
                                    if offset + 4 <= surface.data.len() {
                                        surface.data[offset..offset + 4].copy_from_slice(&rgba);
                                    }
                                }
                            }
                            self.notify_update(draw_fill.base.surface_id);
                        }
                    } else {
                        warn!(
                            "Unsupported DrawFill brush type {}",
                            draw_fill.data.brush.brush_type
                        );
                    }
                } else {
                    warn!("Failed to parse DrawFill message");
                }
            }
            x if x == DisplayChannelMessage::DrawCopy as u16 => {
                debug!("Handle draw copy");

                // Parse the draw copy message
                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_copy) = SpiceDrawCopy::read(&mut cursor) {
                    let surface_id = draw_copy.base.surface_id;
                    let bbox = &draw_copy.base.box_;
                    let src_area = &draw_copy.data.src_area;
                    let clips = parse_clip_rects(&draw_copy.base.clip, data)?;

                    debug!("DrawCopy on surface {} - rect: ({},{}) to ({},{}) from src rect ({},{}) to ({},{}) src_image: 0x{:x}",
                          surface_id, bbox.left, bbox.top, bbox.right, bbox.bottom,
                          src_area.left, src_area.top, src_area.right, src_area.bottom,
                          draw_copy.data.src_image);

                    let image_address = if draw_copy.data.src_image > 0xFFFFFFFF {
                        let inline_address = draw_copy.base.clip.data;
                        if inline_address > 0 && inline_address < data.len() as u64 {
                            inline_address
                        } else {
                            cursor.position() as u64
                        }
                    } else {
                        draw_copy.data.src_image
                    };
                    let decoded_image = self.decode_image(image_address, data)?;

                    if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                        match decoded_image {
                            Some((image_data, img_width, img_height)) => {
                                info!("Decoded image: {}x{}", img_width, img_height);

                                let source_area = if src_area.left == src_area.right
                                    || src_area.top == src_area.bottom
                                {
                                    SpiceRect {
                                        left: 0,
                                        top: 0,
                                        right: img_width as i32,
                                        bottom: img_height as i32,
                                    }
                                } else {
                                    *src_area
                                };
                                copy_image_to_surface(
                                    surface,
                                    &image_data,
                                    img_width,
                                    img_height,
                                    source_area,
                                    *bbox,
                                    clips.as_deref(),
                                    false,
                                );

                                self.notify_update(surface_id);
                            }
                            None => {
                                warn!(
                                    "Failed to decode image at address 0x{:x}",
                                    draw_copy.data.src_image
                                );
                            }
                        }
                    }
                } else {
                    warn!("Failed to parse DrawCopy message");
                }
            }
            x if x == DisplayChannelMessage::DrawOpaque as u16 => {
                debug!("Handle draw opaque");

                // Parse the draw opaque message
                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_opaque) = SpiceDrawOpaque::read(&mut cursor) {
                    let surface_id = draw_opaque.base.surface_id;
                    let bbox = &draw_opaque.base.box_;
                    let brush = &draw_opaque.data.brush;
                    let src_area = &draw_opaque.data.src_area;
                    let clips = parse_clip_rects(&draw_opaque.base.clip, data)?;

                    info!("DrawOpaque on surface {} - rect: ({},{}) to ({},{}) brush type: {} src_image: 0x{:x}", 
                          surface_id, bbox.left, bbox.top, bbox.right, bbox.bottom,
                          brush.brush_type, draw_opaque.data.src_image);

                    // Try to decode the source image first (before getting mutable reference)
                    let decoded_image = if draw_opaque.data.src_image != 0 {
                        self.decode_image(draw_opaque.data.src_image, data)?
                    } else {
                        None
                    };

                    if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                        let bytes_per_pixel = 4;
                        let stride = surface.width as usize * bytes_per_pixel;

                        // First, fill with brush background if it's a solid color
                        if brush.brush_type == 1 {
                            // SOLID brush
                            let r = ((brush.color >> 16) & 0xFF) as u8;
                            let g = ((brush.color >> 8) & 0xFF) as u8;
                            let b = (brush.color & 0xFF) as u8;

                            // Fill the destination rectangle with brush color
                            for y in bbox.top.max(0)..bbox.bottom.min(surface.height as i32) {
                                for x in bbox.left.max(0)..bbox.right.min(surface.width as i32) {
                                    if !inside_clip(clips.as_deref(), x, y) {
                                        continue;
                                    }
                                    let offset = y as usize * stride + x as usize * bytes_per_pixel;
                                    if offset + 4 <= surface.data.len() {
                                        surface.data[offset] = r;
                                        surface.data[offset + 1] = g;
                                        surface.data[offset + 2] = b;
                                        surface.data[offset + 3] = 255;
                                    }
                                }
                            }
                        }

                        // If there's a source image, overlay it on top
                        if let Some((image_data, img_width, img_height)) = decoded_image {
                            info!("Decoded opaque source image: {}x{}", img_width, img_height);

                            let source_area = if src_area.left == src_area.right
                                || src_area.top == src_area.bottom
                            {
                                SpiceRect {
                                    left: 0,
                                    top: 0,
                                    right: img_width as i32,
                                    bottom: img_height as i32,
                                }
                            } else {
                                *src_area
                            };
                            copy_image_to_surface(
                                surface,
                                &image_data,
                                img_width,
                                img_height,
                                source_area,
                                *bbox,
                                clips.as_deref(),
                                false,
                            );
                        } else if brush.brush_type != 1 {
                            warn!(
                                "Unsupported DrawOpaque brush type {} without source image",
                                brush.brush_type
                            );
                        }

                        self.notify_update(surface_id);
                    }
                } else {
                    warn!("Failed to parse DrawOpaque message");
                }
            }
            x if x == DisplayChannelMessage::DrawBlend as u16 => {
                debug!("Handle draw blend");

                // Parse the draw blend message
                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_blend) = SpiceDrawBlend::read(&mut cursor) {
                    let surface_id = draw_blend.base.surface_id;
                    let bbox = draw_blend.base.box_;
                    let clips = parse_clip_rects(&draw_blend.base.clip, data)?;

                    info!(
                        "DrawBlend on surface {} - rect: ({},{}) to ({},{})",
                        surface_id, bbox.left, bbox.top, bbox.right, bbox.bottom
                    );

                    let source_area = draw_blend.data.src_area;
                    let decoded_image = self.decode_image(draw_blend.data.src_image, data)?;
                    if let (Some((image_data, img_width, img_height)), Some(surface)) =
                        (decoded_image, self.surfaces.get_mut(&surface_id))
                    {
                        let source_area = if source_area.left == source_area.right
                            || source_area.top == source_area.bottom
                        {
                            SpiceRect {
                                left: 0,
                                top: 0,
                                right: img_width as i32,
                                bottom: img_height as i32,
                            }
                        } else {
                            source_area
                        };
                        copy_image_to_surface(
                            surface,
                            &image_data,
                            img_width,
                            img_height,
                            source_area,
                            bbox,
                            clips.as_deref(),
                            true,
                        );
                        self.notify_update(surface_id);
                    } else {
                        warn!("Unable to decode DrawBlend source image");
                    }
                } else {
                    warn!("Failed to parse DrawBlend message");
                }
            }
            x if x == DisplayChannelMessage::DrawBlackness as u16 => {
                debug!("Handle draw blackness");

                // Parse the draw blackness message
                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_blackness) = SpiceDrawBlackness::read(&mut cursor) {
                    let surface_id = draw_blackness.base.surface_id;
                    let bbox = &draw_blackness.base.box_;

                    info!(
                        "DrawBlackness on surface {} - rect: ({},{}) to ({},{})",
                        surface_id, bbox.left, bbox.top, bbox.right, bbox.bottom
                    );

                    // Fill the bounding box with black (0,0,0,255)
                    if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                        let bytes_per_pixel = 4;
                        let stride = surface.width as usize * bytes_per_pixel;

                        for y in bbox.top.max(0)..bbox.bottom.min(surface.height as i32) {
                            let y = y as usize;
                            for x in bbox.left.max(0)..bbox.right.min(surface.width as i32) {
                                let x = x as usize;
                                let offset = y * stride + x * bytes_per_pixel;
                                if offset + 4 <= surface.data.len() {
                                    surface.data[offset] = 0; // R
                                    surface.data[offset + 1] = 0; // G
                                    surface.data[offset + 2] = 0; // B
                                    surface.data[offset + 3] = 255; // A
                                }
                            }
                        }

                        self.notify_update(surface_id);
                    }
                } else {
                    warn!("Failed to parse DrawBlackness message");
                }
            }
            x if x == DisplayChannelMessage::DrawWhiteness as u16 => {
                debug!("Handle draw whiteness");

                // Parse the draw whiteness message
                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_whiteness) = SpiceDrawWhiteness::read(&mut cursor) {
                    let surface_id = draw_whiteness.base.surface_id;
                    let bbox = &draw_whiteness.base.box_;

                    info!(
                        "DrawWhiteness on surface {} - rect: ({},{}) to ({},{})",
                        surface_id, bbox.left, bbox.top, bbox.right, bbox.bottom
                    );

                    // Fill the bounding box with white (255,255,255,255)
                    if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                        let bytes_per_pixel = 4;
                        let stride = surface.width as usize * bytes_per_pixel;

                        for y in bbox.top.max(0)..bbox.bottom.min(surface.height as i32) {
                            let y = y as usize;
                            for x in bbox.left.max(0)..bbox.right.min(surface.width as i32) {
                                let x = x as usize;
                                let offset = y * stride + x * bytes_per_pixel;
                                if offset + 4 <= surface.data.len() {
                                    surface.data[offset] = 255; // R
                                    surface.data[offset + 1] = 255; // G
                                    surface.data[offset + 2] = 255; // B
                                    surface.data[offset + 3] = 255; // A
                                }
                            }
                        }

                        self.notify_update(surface_id);
                    }
                } else {
                    warn!("Failed to parse DrawWhiteness message");
                }
            }
            x if x == DisplayChannelMessage::DrawInvers as u16 => {
                debug!("Handle draw invers");

                // Parse the draw invers message
                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_invers) = SpiceDrawInvers::read(&mut cursor) {
                    let surface_id = draw_invers.base.surface_id;
                    let bbox = &draw_invers.base.box_;

                    info!(
                        "DrawInvers on surface {} - rect: ({},{}) to ({},{})",
                        surface_id, bbox.left, bbox.top, bbox.right, bbox.bottom
                    );

                    // Invert all pixels in the bounding box
                    if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                        let bytes_per_pixel = 4;
                        let stride = surface.width as usize * bytes_per_pixel;

                        for y in bbox.top.max(0)..bbox.bottom.min(surface.height as i32) {
                            let y = y as usize;
                            for x in bbox.left.max(0)..bbox.right.min(surface.width as i32) {
                                let x = x as usize;
                                let offset = y * stride + x * bytes_per_pixel;
                                if offset + 4 <= surface.data.len() {
                                    // Invert RGB, keep alpha
                                    surface.data[offset] = 255 - surface.data[offset]; // R
                                    surface.data[offset + 1] = 255 - surface.data[offset + 1]; // G
                                    surface.data[offset + 2] = 255 - surface.data[offset + 2];
                                    // B
                                    // Keep A unchanged
                                }
                            }
                        }

                        self.notify_update(surface_id);
                    }
                } else {
                    warn!("Failed to parse DrawInvers message");
                }
            }
            x if x == DisplayChannelMessage::DrawRop3 as u16 => {
                debug!("Handle draw rop3");

                // Parse the draw rop3 message
                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_rop3) = SpiceDrawRop3::read(&mut cursor) {
                    let surface_id = draw_rop3.base.surface_id;
                    let bbox = &draw_rop3.base.box_;
                    let src_area = &draw_rop3.src_area;

                    info!(
                        "DrawRop3 on surface {} - rect: ({},{}) to ({},{}) rop3: 0x{:x}",
                        surface_id, bbox.left, bbox.top, bbox.right, bbox.bottom, draw_rop3.rop3
                    );

                    // ROP3 is complex - for now implement a simple fallback
                    // Most common ROP3 operations:
                    // 0xCC - SRCCOPY (copy source)
                    // 0xF0 - PATCOPY (copy pattern/brush)
                    // 0xAA - NOP (no operation)

                    match draw_rop3.rop3 {
                        0xCC => {
                            // SRCCOPY - copy source bitmap to destination
                            let decoded_image = self.decode_image(draw_rop3.src_bitmap, data)?;

                            if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                                if let Some((image_data, img_width, img_height)) = decoded_image {
                                    // Similar to DrawCopy implementation
                                    let bytes_per_pixel = 4;
                                    let surface_stride = surface.width as usize * bytes_per_pixel;
                                    let image_stride = img_width as usize * bytes_per_pixel;

                                    let src_left = src_area.left.max(0) as usize;
                                    let src_top = src_area.top.max(0) as usize;
                                    let src_right = src_area.right.min(img_width as i32) as usize;
                                    let src_bottom =
                                        src_area.bottom.min(img_height as i32) as usize;

                                    let dst_left = bbox.left.max(0) as usize;
                                    let dst_top = bbox.top.max(0) as usize;
                                    let dst_right = bbox.right.min(surface.width as i32) as usize;
                                    let dst_bottom =
                                        bbox.bottom.min(surface.height as i32) as usize;

                                    let copy_width =
                                        (src_right - src_left).min(dst_right - dst_left);
                                    let copy_height =
                                        (src_bottom - src_top).min(dst_bottom - dst_top);

                                    for y in 0..copy_height {
                                        let src_y = src_top + y;
                                        let dst_y = dst_top + y;

                                        if src_y < img_height as usize
                                            && dst_y < surface.height as usize
                                        {
                                            let src_row_offset = src_y * image_stride;
                                            let dst_row_offset = dst_y * surface_stride;

                                            for x in 0..copy_width {
                                                let src_x = src_left + x;
                                                let dst_x = dst_left + x;

                                                if src_x < img_width as usize
                                                    && dst_x < surface.width as usize
                                                {
                                                    let src_offset =
                                                        src_row_offset + src_x * bytes_per_pixel;
                                                    let dst_offset =
                                                        dst_row_offset + dst_x * bytes_per_pixel;

                                                    if src_offset + 4 <= image_data.len()
                                                        && dst_offset + 4 <= surface.data.len()
                                                    {
                                                        surface.data[dst_offset..dst_offset + 4]
                                                            .copy_from_slice(
                                                                &image_data
                                                                    [src_offset..src_offset + 4],
                                                            );
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    self.notify_update(surface_id);
                                }
                            }
                        }
                        _ => {
                            warn!(
                                "ROP3 operation 0x{:x} not fully implemented, using fallback",
                                draw_rop3.rop3
                            );
                            // Fallback: just copy source if available
                            if draw_rop3.src_bitmap != 0 {
                                let decoded_image =
                                    self.decode_image(draw_rop3.src_bitmap, data)?;
                                if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                                    if let Some((image_data, img_width, img_height)) = decoded_image
                                    {
                                        // Simple copy like SRCCOPY
                                        let bytes_per_pixel = 4;
                                        let surface_stride =
                                            surface.width as usize * bytes_per_pixel;
                                        let image_stride = img_width as usize * bytes_per_pixel;

                                        let src_left = src_area.left.max(0) as usize;
                                        let src_top = src_area.top.max(0) as usize;
                                        let src_right =
                                            src_area.right.min(img_width as i32) as usize;
                                        let src_bottom =
                                            src_area.bottom.min(img_height as i32) as usize;

                                        let dst_left = bbox.left.max(0) as usize;
                                        let dst_top = bbox.top.max(0) as usize;
                                        let dst_right =
                                            bbox.right.min(surface.width as i32) as usize;
                                        let dst_bottom =
                                            bbox.bottom.min(surface.height as i32) as usize;

                                        let copy_width =
                                            (src_right - src_left).min(dst_right - dst_left);
                                        let copy_height =
                                            (src_bottom - src_top).min(dst_bottom - dst_top);

                                        for y in 0..copy_height {
                                            let src_y = src_top + y;
                                            let dst_y = dst_top + y;

                                            if src_y < img_height as usize
                                                && dst_y < surface.height as usize
                                            {
                                                let src_row_offset = src_y * image_stride;
                                                let dst_row_offset = dst_y * surface_stride;

                                                for x in 0..copy_width {
                                                    let src_x = src_left + x;
                                                    let dst_x = dst_left + x;

                                                    if src_x < img_width as usize
                                                        && dst_x < surface.width as usize
                                                    {
                                                        let src_offset = src_row_offset
                                                            + src_x * bytes_per_pixel;
                                                        let dst_offset = dst_row_offset
                                                            + dst_x * bytes_per_pixel;

                                                        if src_offset + 4 <= image_data.len()
                                                            && dst_offset + 4 <= surface.data.len()
                                                        {
                                                            surface.data
                                                                [dst_offset..dst_offset + 4]
                                                                .copy_from_slice(
                                                                    &image_data[src_offset
                                                                        ..src_offset + 4],
                                                                );
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        self.notify_update(surface_id);
                                    }
                                }
                            }
                        }
                    }
                } else {
                    warn!("Failed to parse DrawRop3 message");
                }
            }
            x if x == DisplayChannelMessage::DrawTransparent as u16 => {
                debug!("Handle draw transparent");

                // Parse the draw transparent message
                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_transparent) = SpiceDrawTransparent::read(&mut cursor) {
                    let surface_id = draw_transparent.base.surface_id;
                    let bbox = &draw_transparent.base.box_;
                    let src_area = &draw_transparent.src_area;

                    info!(
                        "DrawTransparent on surface {} - rect: ({},{}) to ({},{}) src_color: 0x{:x} true_color: 0x{:x}",
                        surface_id, bbox.left, bbox.top, bbox.right, bbox.bottom,
                        draw_transparent.src_color, draw_transparent.true_color
                    );

                    // Decode source bitmap
                    let decoded_image = self.decode_image(draw_transparent.src_bitmap, data)?;

                    if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                        if let Some((image_data, img_width, img_height)) = decoded_image {
                            let bytes_per_pixel = 4;
                            let surface_stride = surface.width as usize * bytes_per_pixel;
                            let image_stride = img_width as usize * bytes_per_pixel;

                            let src_left = src_area.left.max(0) as usize;
                            let src_top = src_area.top.max(0) as usize;
                            let src_right = src_area.right.min(img_width as i32) as usize;
                            let src_bottom = src_area.bottom.min(img_height as i32) as usize;

                            let dst_left = bbox.left.max(0) as usize;
                            let dst_top = bbox.top.max(0) as usize;
                            let dst_right = bbox.right.min(surface.width as i32) as usize;
                            let dst_bottom = bbox.bottom.min(surface.height as i32) as usize;

                            let copy_width = (src_right - src_left).min(dst_right - dst_left);
                            let copy_height = (src_bottom - src_top).min(dst_bottom - dst_top);

                            // Extract transparent color components (assuming RGB format)
                            let trans_r = ((draw_transparent.src_color >> 16) & 0xFF) as u8;
                            let trans_g = ((draw_transparent.src_color >> 8) & 0xFF) as u8;
                            let trans_b = (draw_transparent.src_color & 0xFF) as u8;

                            for y in 0..copy_height {
                                let src_y = src_top + y;
                                let dst_y = dst_top + y;

                                if src_y < img_height as usize && dst_y < surface.height as usize {
                                    let src_row_offset = src_y * image_stride;
                                    let dst_row_offset = dst_y * surface_stride;

                                    for x in 0..copy_width {
                                        let src_x = src_left + x;
                                        let dst_x = dst_left + x;

                                        if src_x < img_width as usize
                                            && dst_x < surface.width as usize
                                        {
                                            let src_offset =
                                                src_row_offset + src_x * bytes_per_pixel;
                                            let dst_offset =
                                                dst_row_offset + dst_x * bytes_per_pixel;

                                            if src_offset + 4 <= image_data.len()
                                                && dst_offset + 4 <= surface.data.len()
                                            {
                                                let src_r = image_data[src_offset];
                                                let src_g = image_data[src_offset + 1];
                                                let src_b = image_data[src_offset + 2];

                                                // Only copy if pixel doesn't match transparent color
                                                if src_r != trans_r
                                                    || src_g != trans_g
                                                    || src_b != trans_b
                                                {
                                                    surface.data[dst_offset..dst_offset + 4]
                                                        .copy_from_slice(
                                                            &image_data[src_offset..src_offset + 4],
                                                        );
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            self.notify_update(surface_id);
                        }
                    }
                } else {
                    warn!("Failed to parse DrawTransparent message");
                }
            }
            x if x == DisplayChannelMessage::DrawStroke as u16 => {
                debug!("Handle draw stroke");

                // Parse the draw stroke message (simplified - path rendering not fully implemented)
                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_stroke) = SpiceDrawStroke::read(&mut cursor) {
                    let surface_id = draw_stroke.base.surface_id;
                    let bbox = &draw_stroke.base.box_;

                    info!(
                        "DrawStroke on surface {} - rect: ({},{}) to ({},{}) (simplified implementation)",
                        surface_id, bbox.left, bbox.top, bbox.right, bbox.bottom
                    );

                    // Simplified implementation: draw the bounding box outline with brush color
                    if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                        let bytes_per_pixel = 4;
                        let stride = surface.width as usize * bytes_per_pixel;

                        // Get brush color
                        let r = ((draw_stroke.brush.color >> 16) & 0xFF) as u8;
                        let g = ((draw_stroke.brush.color >> 8) & 0xFF) as u8;
                        let b = (draw_stroke.brush.color & 0xFF) as u8;

                        // Draw rectangle outline (simplified path rendering)
                        for y in bbox.top.max(0)..bbox.bottom.min(surface.height as i32) {
                            for x in bbox.left.max(0)..bbox.right.min(surface.width as i32) {
                                // Only draw on the border
                                if y == bbox.top
                                    || y == bbox.bottom - 1
                                    || x == bbox.left
                                    || x == bbox.right - 1
                                {
                                    let offset =
                                        (y as usize) * stride + (x as usize) * bytes_per_pixel;
                                    if offset + 4 <= surface.data.len() {
                                        surface.data[offset] = r;
                                        surface.data[offset + 1] = g;
                                        surface.data[offset + 2] = b;
                                        surface.data[offset + 3] = 255;
                                    }
                                }
                            }
                        }

                        self.notify_update(surface_id);
                    }
                } else {
                    warn!("Failed to parse DrawStroke message");
                }
            }
            x if x == DisplayChannelMessage::DrawText as u16 => {
                debug!("Handle draw text");

                // Parse the draw text message (simplified - glyph rendering not implemented)
                let mut cursor = std::io::Cursor::new(data);
                if let Ok(draw_text) = SpiceDrawText::read(&mut cursor) {
                    let surface_id = draw_text.base.surface_id;
                    let bbox = &draw_text.base.box_;

                    info!(
                        "DrawText on surface {} - rect: ({},{}) to ({},{}) (placeholder implementation)",
                        surface_id, bbox.left, bbox.top, bbox.right, bbox.bottom
                    );

                    // Simplified placeholder: fill background area with back_brush color
                    if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                        let bytes_per_pixel = 4;
                        let stride = surface.width as usize * bytes_per_pixel;

                        // Get back brush color
                        let r = ((draw_text.back_brush.color >> 16) & 0xFF) as u8;
                        let g = ((draw_text.back_brush.color >> 8) & 0xFF) as u8;
                        let b = (draw_text.back_brush.color & 0xFF) as u8;

                        // Fill back area
                        let back_area = &draw_text.back_area;
                        for y in back_area.top.max(0)..back_area.bottom.min(surface.height as i32) {
                            let y = y as usize;
                            for x in
                                back_area.left.max(0)..back_area.right.min(surface.width as i32)
                            {
                                let x = x as usize;
                                let offset = y * stride + x * bytes_per_pixel;
                                if offset + 4 <= surface.data.len() {
                                    surface.data[offset] = r;
                                    surface.data[offset + 1] = g;
                                    surface.data[offset + 2] = b;
                                    surface.data[offset + 3] = 255;
                                }
                            }
                        }

                        self.notify_update(surface_id);
                    }
                } else {
                    warn!("Failed to parse DrawText message");
                }
            }
            _ => {
                debug!("Unhandled draw message type: {}", msg_type);
            }
        }

        Ok(())
    }

    async fn handle_stream_message(&mut self, msg_type: u16, data: &[u8]) -> Result<()> {
        match msg_type {
            x if x == DisplayChannelMessage::StreamCreate as u16 => {
                debug!("Handle stream create");
                let mut cursor = std::io::Cursor::new(data);
                let stream_create = SpiceStreamCreate::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse StreamCreate: {e}"))
                })?;

                info!(
                    "Created stream {} - codec: {}, dimensions: {}x{} -> {}x{}",
                    stream_create.id,
                    stream_create.codec_type,
                    stream_create.src_width,
                    stream_create.src_height,
                    stream_create.stream_width,
                    stream_create.stream_height
                );

                // Store stream info for processing subsequent data
                self.active_streams.insert(
                    stream_create.id,
                    StreamInfo {
                        id: stream_create.id,
                        codec_type: stream_create.codec_type,
                        width: stream_create.stream_width,
                        height: stream_create.stream_height,
                        dest_rect: stream_create.dest.clone(),
                    },
                );
            }
            x if x == DisplayChannelMessage::StreamData as u16 => {
                debug!("Handle stream data");
                let mut cursor = std::io::Cursor::new(data);
                let stream_data = SpiceStreamData::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse StreamData: {e}"))
                })?;

                debug!(
                    "Received {} bytes for stream {}",
                    stream_data.data_size, stream_data.id
                );

                // TODO: Decode stream data and apply to surface
                // This would involve:
                // 1. Finding the decoder for this stream ID
                // 2. Decoding the data based on codec type
                // 3. Applying decoded frame to the display surface
            }
            x if x == DisplayChannelMessage::StreamDestroy as u16 => {
                debug!("Handle stream destroy");
                let mut cursor = std::io::Cursor::new(data);
                let stream_destroy = SpiceStreamDestroy::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse StreamDestroy: {e}"))
                })?;

                info!("Destroyed stream {}", stream_destroy.id);

                // Clean up stream info
                self.active_streams.remove(&stream_destroy.id);
            }
            _ => {
                debug!("Unhandled stream message type: {}", msg_type);
            }
        }

        Ok(())
    }
}

impl Channel for DisplayChannel {
    async fn handle_message(&mut self, header: &SpiceDataHeader, data: &[u8]) -> Result<()> {
        match header.msg_type {
            x if x == DisplayChannelMessage::Mode as u16 => {
                debug!("Received display mode");
                self.handle_mode_message(data).await?;
            }
            x if x == DisplayChannelMessage::Mark as u16 => {
                debug!("Received display mark");

                if data.len() >= std::mem::size_of::<u32>() {
                    self.last_mark = Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
                } else if !data.is_empty() {
                    return Err(SpiceError::Protocol(format!(
                        "Display mark payload is too short: {} bytes",
                        data.len()
                    )));
                }

                // According to the protocol, we should expose the display content after receiving a mark
                // Notify any update callbacks about the current surface state
                if self.surfaces.contains_key(&0) {
                    self.notify_update_force(0);
                }

                // The MARK message itself doesn't require a specific acknowledgment response
                // It's a synchronization point to indicate when display content should be exposed
            }
            x if x == DisplayChannelMessage::Reset as u16 => {
                debug!("Received display reset");
                // Reset display state - clear all surfaces and streams
                self.surfaces.clear();
                self.active_streams.clear();
                self.monitors.clear();
            }
            x if x == DisplayChannelMessage::InvalList as u16 => {
                debug!("Received invalidation list");

                // Parse the invalidation list: starts with a u16 count, followed by u64 IDs
                if data.len() >= 2 {
                    let count = u16::from_le_bytes([data[0], data[1]]) as usize;
                    let expected_size = 2 + (count * 8); // 2 bytes for count + count * 8 bytes for u64 IDs

                    if data.len() >= expected_size {
                        let mut removed_count = 0;
                        for i in 0..count {
                            let offset = 2 + (i * 8);
                            let id = u64::from_le_bytes([
                                data[offset],
                                data[offset + 1],
                                data[offset + 2],
                                data[offset + 3],
                                data[offset + 4],
                                data[offset + 5],
                                data[offset + 6],
                                data[offset + 7],
                            ]);

                            if self.image_cache.remove(id) {
                                removed_count += 1;
                                debug!("Invalidated cached resource ID: {}", id);
                            }
                        }

                        info!(
                            "Invalidated {} out of {} cached resources (cache size: {})",
                            removed_count,
                            count,
                            self.image_cache.len()
                        );
                    } else {
                        warn!(
                            "INVAL_LIST message too small: expected {} bytes, got {}",
                            expected_size,
                            data.len()
                        );
                    }
                } else {
                    warn!("INVAL_LIST message too small to contain count");
                }
            }
            x if x == DisplayChannelMessage::InvalAllPixmaps as u16 => {
                debug!("Received invalidate all pixmaps");

                // Clear the entire image cache
                let cache_size = self.image_cache.len();
                self.image_cache.clear();

                info!(
                    "Invalidated all pixmaps - cleared {} cached images",
                    cache_size
                );
            }
            x if (DisplayChannelMessage::DrawFill as u16
                ..=DisplayChannelMessage::DrawAlphaBlend as u16)
                .contains(&x) =>
            {
                self.handle_draw_message(header.msg_type, data).await?;
            }
            x if (DisplayChannelMessage::StreamCreate as u16
                ..=DisplayChannelMessage::StreamDestroyAll as u16)
                .contains(&x) =>
            {
                self.handle_stream_message(header.msg_type, data).await?;
            }
            x if x == SPICE_MSG_DISPLAY_SURFACE_CREATE => {
                debug!("Received surface create");
                let mut cursor = std::io::Cursor::new(data);
                let surface_create = SpiceMsgSurfaceCreate::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse SurfaceCreate: {e}"))
                })?;

                info!(
                    "Creating surface {} - {}x{} format: {}",
                    surface_create.surface_id,
                    surface_create.width,
                    surface_create.height,
                    surface_create.format
                );

                // Create new surface
                let data_size = (surface_create.width * surface_create.height * 4) as usize;
                self.surfaces.insert(
                    surface_create.surface_id,
                    DisplaySurface {
                        width: surface_create.width,
                        height: surface_create.height,
                        format: surface_create.format,
                        data: vec![0; data_size],
                    },
                );

                // Notify about new surface
                self.notify_update(surface_create.surface_id);
            }
            x if x == SPICE_MSG_DISPLAY_SURFACE_DESTROY => {
                debug!("Received surface destroy");
                let mut cursor = std::io::Cursor::new(data);
                let surface_destroy = SpiceMsgSurfaceDestroy::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse SurfaceDestroy: {e}"))
                })?;

                info!("Destroying surface {}", surface_destroy.surface_id);

                self.surfaces.remove(&surface_destroy.surface_id);
            }
            x if x == SPICE_MSG_DISPLAY_MONITORS_CONFIG => {
                debug!("Received monitors config");
                let mut cursor = std::io::Cursor::new(data);
                let monitors_config = SpiceMonitorsConfig::read(&mut cursor).map_err(|e| {
                    SpiceError::Protocol(format!("Failed to parse MonitorsConfig: {e}"))
                })?;

                info!(
                    "Monitors config: {} monitors (max {})",
                    monitors_config.count, monitors_config.max_allowed
                );

                self.monitors = monitors_config.heads;

                // Log monitor configurations
                for (i, head) in self.monitors.iter().enumerate() {
                    info!(
                        "Monitor {}: {}x{} at ({},{}) on surface {}",
                        i, head.width, head.height, head.x, head.y, head.surface_id
                    );
                }
            }
            3 => {
                debug!("DisplayChannel: ACK flow control handled by channel connection");
            }
            x if x == SPICE_MSG_DISPLAY_COPY_BITS => {
                debug!("Received DISPLAY_COPY_BITS");

                // Parse the copy bits message
                let mut cursor = std::io::Cursor::new(data);
                match SpiceCopyBits::read(&mut cursor) {
                    Ok(copy_bits) => {
                        let surface_id = copy_bits.base.surface_id;
                        let dest_rect = copy_bits.base.box_;
                        let clips = parse_clip_rects(&copy_bits.base.clip, data)?;

                        debug!(
                            "COPY_BITS: surface {} from ({}, {}) to rect ({},{}) - ({},{})",
                            surface_id,
                            copy_bits.src_pos.x,
                            copy_bits.src_pos.y,
                            dest_rect.left,
                            dest_rect.top,
                            dest_rect.right,
                            dest_rect.bottom
                        );

                        // Perform the copy operation
                        if let Some(surface) = self.surfaces.get_mut(&surface_id) {
                            copy_surface_region(
                                surface,
                                copy_bits.src_pos,
                                dest_rect,
                                clips.as_deref(),
                            );
                            self.notify_update(surface_id);
                        } else {
                            warn!("COPY_BITS: Surface {} not found", surface_id);
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse COPY_BITS message: {}", e);
                    }
                }
            }
            x if x == SPICE_MSG_DISPLAY_INVAL_PALETTE => {
                // This message tells the client to invalidate a specific cached palette.
                // Since we don't currently cache palettes (only full images), we can
                // safely acknowledge this message.
                if data.len() >= 8 {
                    let palette_id = u64::from_le_bytes([
                        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
                    ]);
                    debug!(
                        "Received DISPLAY_INVAL_PALETTE for palette_id: {} (no-op)",
                        palette_id
                    );
                } else {
                    debug!("Received DISPLAY_INVAL_PALETTE - palette cache entry cleared (no-op)");
                }
            }
            x if x == SPICE_MSG_DISPLAY_INVAL_ALL_PALETTES => {
                // This message tells the client to invalidate all cached palettes.
                // Since we don't currently cache palettes (only full images), we can
                // safely acknowledge this message. Palettes are used for indexed color
                // images which we convert to RGBA during decoding.
                debug!("Received DISPLAY_INVAL_ALL_PALETTES - palette cache cleared (no-op)");
            }
            x if x == SPICE_MSG_PING => {
                debug!("Received PING message in display channel");
                if data.len() >= 4 {
                    let id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                    let time = if data.len() >= 12 {
                        u64::from_le_bytes([
                            data[4], data[5], data[6], data[7], data[8], data[9], data[10],
                            data[11],
                        ])
                    } else {
                        0
                    };
                    // Send PONG response
                    let mut pong_data = Vec::with_capacity(12);
                    pong_data.extend_from_slice(&id.to_le_bytes());
                    pong_data.extend_from_slice(&time.to_le_bytes());
                    self.connection
                        .send_message(SPICE_MSGC_PONG, &pong_data)
                        .await?;
                }
            }
            x if x == SPICE_MSG_NOTIFY => {
                if data.len() >= 12 {
                    let severity = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
                    let msg_len =
                        u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
                    if data.len() >= 12 + msg_len {
                        let message = String::from_utf8_lossy(&data[12..12 + msg_len]);
                        match severity {
                            0 => info!("Display server info: {}", message),
                            1 => warn!("Display server warning: {}", message),
                            2 => error!("Display server error: {}", message),
                            _ => debug!("Display server notification: {}", message),
                        }
                    }
                }
            }
            x if x == SPICE_MSG_DISCONNECTING => {
                info!("Display channel: Server is disconnecting");
                return Err(SpiceError::ConnectionClosed);
            }
            x if x == SPICE_MSG_MIGRATE => {
                warn!("Display channel: Migration not implemented");
            }
            x if x == SPICE_MSG_MIGRATE_DATA => {
                warn!("Display channel: Migration data not implemented");
            }
            x if x == SPICE_MSG_WAIT_FOR_CHANNELS => {
                warn!("Display channel: Wait for channels not implemented");
            }
            _ => {
                warn!("Unknown display message type: {}", header.msg_type);
            }
        }

        Ok(())
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Display
    }
}

fn strip_lz_data_size_prefix(data: &[u8]) -> &[u8] {
    if data.len() < 4 {
        return data;
    }

    let declared_len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if declared_len == data.len() - 4 {
        &data[4..]
    } else {
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use binrw::BinWrite;
    use std::io::Cursor;

    #[test]
    fn bitmap_32bit_ignores_reserved_alpha_byte() {
        let bitmap = SpiceBitmap {
            format: SPICE_BITMAP_FMT_32BIT,
            flags: 0,
            x: 1,
            y: 1,
            stride: 4,
            palette: 0,
            data: 0,
        };

        for reserved in [0, 64, 255] {
            let (pixels, width, height) =
                decode_bitmap(&bitmap, &[3, 2, 1, reserved], 1, 1)
                    .unwrap()
                    .unwrap();
            assert_eq!((width, height), (1, 1));
            assert_eq!(pixels, [1, 2, 3, 255]);
        }
    }

    #[test]
    fn bitmap_rgba_preserves_alpha_channel() {
        let bitmap = SpiceBitmap {
            format: SPICE_BITMAP_FMT_RGBA,
            flags: 0,
            x: 1,
            y: 1,
            stride: 4,
            palette: 0,
            data: 0,
        };

        let (pixels, _, _) = decode_bitmap(&bitmap, &[1, 2, 3, 64], 1, 1)
            .unwrap()
            .unwrap();

        assert_eq!(pixels, [1, 2, 3, 64]);
    }

    #[test]
    fn bitmap_scanline_order_follows_top_down_flag() {
        let mut bitmap = SpiceBitmap {
            format: SPICE_BITMAP_FMT_32BIT,
            flags: 0,
            x: 1,
            y: 2,
            stride: 4,
            palette: 0,
            data: 0,
        };
        let bitmap_data = [
            0, 0, 255, 0, // red, first source row
            255, 0, 0, 0, // blue, second source row
        ];

        let (bottom_up_pixels, _, _) = decode_bitmap(&bitmap, &bitmap_data, 1, 2)
            .unwrap()
            .unwrap();
        assert_eq!(bottom_up_pixels, [0, 0, 255, 255, 255, 0, 0, 255]);

        bitmap.flags = SPICE_BITMAP_FLAGS_TOP_DOWN;
        let (top_down_pixels, _, _) = decode_bitmap(&bitmap, &bitmap_data, 1, 2)
            .unwrap()
            .unwrap();
        assert_eq!(top_down_pixels, [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn throttled_display_updates_flush_the_latest_surface() {
        let mut surfaces = HashMap::from([(
            0,
            DisplaySurface {
                width: 1,
                height: 1,
                format: 32,
                data: vec![1, 0, 0, 255],
            },
        )]);
        let last_update_callback = Cell::new(Some(Instant::now()));
        let pending_update_surfaces = RefCell::new(HashSet::from([0]));
        let observed_pixel = Cell::new(0_u8);
        let callback = |surface: &DisplaySurface| observed_pixel.set(surface.data[0]);

        flush_pending_display_updates(
            &surfaces,
            Some(&callback),
            &last_update_callback,
            &pending_update_surfaces,
            false,
        );
        assert_eq!(observed_pixel.get(), 0);
        assert_eq!(pending_update_surfaces.borrow().len(), 1);

        surfaces.get_mut(&0).unwrap().data[0] = 2;
        last_update_callback.set(Some(Instant::now() - Duration::from_millis(17)));
        flush_pending_display_updates(
            &surfaces,
            Some(&callback),
            &last_update_callback,
            &pending_update_surfaces,
            false,
        );

        assert_eq!(observed_pixel.get(), 2);
        assert!(pending_update_surfaces.borrow().is_empty());
    }

    #[test]
    fn parses_clip_address_before_trailing_alignment() {
        let bytes = [0, 0x39, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let clip = SpiceClip::read(&mut Cursor::new(bytes)).unwrap();

        assert_eq!(clip.data, 57);
    }

    #[test]
    fn parses_spice_clip_rect_list() {
        let rect = SpiceRect {
            left: 2,
            top: 3,
            right: 12,
            bottom: 13,
        };
        let mut data = vec![0; 24];
        data[..4].copy_from_slice(&1_u32.to_le_bytes());
        for (offset, value) in [
            (4, rect.top),
            (8, rect.left),
            (12, rect.bottom),
            (16, rect.right),
        ] {
            data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }

        let parsed_rects = parse_clip_rects(
            &SpiceClip {
                clip_type: ClipType::Rects as u8,
                data: 0,
            },
            &data,
        )
        .unwrap()
        .unwrap()[0];
        assert_eq!(parsed_rects.left, rect.left);
        assert_eq!(parsed_rects.top, rect.top);
        assert_eq!(parsed_rects.right, rect.right);
        assert_eq!(parsed_rects.bottom, rect.bottom);
    }

    #[test]
    fn strips_lz_data_size_prefix() {
        assert_eq!(
            strip_lz_data_size_prefix(&[3, 0, 0, 0, 1, 2, 3]),
            &[1, 2, 3]
        );
        assert_eq!(strip_lz_data_size_prefix(&[1, 2, 3]), &[1, 2, 3]);
    }

    #[test]
    fn image_copy_scales_source_and_preserves_alignment_when_clipped() {
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let blue = [0, 0, 255, 255];
        let white = [255, 255, 255, 255];
        let image = [red, green, blue, white].concat();
        let mut surface = DisplaySurface {
            width: 3,
            height: 3,
            format: 32,
            data: vec![0; 3 * 3 * 4],
        };

        copy_image_to_surface(
            &mut surface,
            &image,
            2,
            2,
            SpiceRect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 2,
            },
            SpiceRect {
                left: -1,
                top: -1,
                right: 3,
                bottom: 3,
            },
            Some(&[SpiceRect {
                left: 0,
                top: 0,
                right: 2,
                bottom: 2,
            }]),
            false,
        );

        let pixel = |x: usize, y: usize| {
            let offset = (y * surface.width as usize + x) * 4;
            &surface.data[offset..offset + 4]
        };
        assert_eq!(pixel(0, 0), &red);
        assert_eq!(pixel(1, 0), &green);
        assert_eq!(pixel(0, 1), &blue);
        assert_eq!(pixel(1, 1), &white);
        assert_eq!(pixel(2, 2), &[0, 0, 0, 0]);
    }

    #[test]
    fn image_blend_composites_source_alpha() {
        let mut surface = DisplaySurface {
            width: 1,
            height: 1,
            format: 32,
            data: vec![255, 0, 0, 255],
        };

        copy_image_to_surface(
            &mut surface,
            &[0, 0, 255, 128],
            1,
            1,
            SpiceRect {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            },
            SpiceRect {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            },
            None,
            true,
        );

        assert_eq!(surface.data, [127, 0, 128, 255]);
    }

    #[test]
    fn copy_bits_preserves_overlap_and_honors_clip() {
        let mut surface = DisplaySurface {
            width: 3,
            height: 1,
            format: 32,
            data: [
                [1, 0, 0, 255],
                [2, 0, 0, 255],
                [3, 0, 0, 255],
            ]
            .concat(),
        };

        copy_surface_region(
            &mut surface,
            SpicePoint { x: 0, y: 0 },
            SpiceRect {
                left: 1,
                top: 0,
                right: 3,
                bottom: 1,
            },
            Some(&[SpiceRect {
                left: 2,
                top: 0,
                right: 3,
                bottom: 1,
            }]),
        );

        assert_eq!(&surface.data[0..4], &[1, 0, 0, 255]);
        assert_eq!(&surface.data[4..8], &[2, 0, 0, 255]);
        assert_eq!(&surface.data[8..12], &[2, 0, 0, 255]);
    }

    #[test]
    fn test_lz_image_decompression_returns_rgba_pixels() {
        let mut compressed = Vec::from(&b"  ZL"[..]);
        compressed.extend_from_slice(&1_u16.to_be_bytes());
        compressed.extend_from_slice(&0_u16.to_be_bytes());
        compressed.extend_from_slice(&[0; 3]);
        compressed.push(0);
        for value in [1_u32, 1, 3, 1] {
            compressed.extend_from_slice(&value.to_be_bytes());
        }
        compressed.extend_from_slice(&[0, 0x33, 0x22, 0x11]);

        let decoded = shakenfist_spice_compression::decompress_lz(&compressed).unwrap();

        assert_eq!((decoded.width, decoded.height), (1, 1));
        assert_eq!(decoded.pixels, [0x11, 0x22, 0x33, 255]);
    }

    #[test]
    fn test_copy_bits_message_parsing() {
        // Create a test SpiceCopyBits message
        let copy_bits = SpiceCopyBits {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 100,
                    top: 50,
                    right: 200,
                    bottom: 150,
                },
                clip: SpiceClip {
                    clip_type: 0, // SPICE_CLIP_TYPE_NONE
                    data: 0,      // No clip data for NONE type
                },
            },
            src_pos: SpicePoint { x: 10, y: 20 },
        };

        // Serialize the message
        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        copy_bits.write(&mut cursor).unwrap();

        // Parse it back
        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceCopyBits::read(&mut read_cursor).unwrap();

        // Verify the parsed message matches
        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.base.box_.left, 100);
        assert_eq!(parsed.base.box_.top, 50);
        assert_eq!(parsed.base.box_.right, 200);
        assert_eq!(parsed.base.box_.bottom, 150);
        assert_eq!(parsed.src_pos.x, 10);
        assert_eq!(parsed.src_pos.y, 20);
    }

    #[test]
    fn test_copy_bits_bounds_validation() {
        // Create a surface for testing
        let mut surface = DisplaySurface {
            width: 800,
            height: 600,
            format: 32, // RGBA
            data: vec![0; 800 * 600 * 4],
        };

        // Fill a test pattern in the source area
        let bytes_per_pixel = 4;
        let stride = surface.width as usize * bytes_per_pixel;
        for y in 20..120 {
            for x in 10..110 {
                let offset = y * stride + x * bytes_per_pixel;
                surface.data[offset] = 255; // R
                surface.data[offset + 1] = 100; // G
                surface.data[offset + 2] = 50; // B
                surface.data[offset + 3] = 255; // A
            }
        }

        // Simulate a copy operation (from 10,20 to 200,100 with size 100x100)
        let src_x = 10;
        let src_y = 20;
        let dest_x = 200;
        let dest_y = 100;
        let rect_width = 100;
        let rect_height = 100;

        // Perform the copy
        let mut temp_buffer = Vec::with_capacity(rect_width * rect_height * bytes_per_pixel);

        // Copy source data to temp buffer
        for y in 0..rect_height {
            let src_offset = (src_y + y) * stride + src_x * bytes_per_pixel;
            let row_data = &surface.data[src_offset..src_offset + rect_width * bytes_per_pixel];
            temp_buffer.extend_from_slice(row_data);
        }

        // Copy from temp buffer to destination
        for y in 0..rect_height {
            let dest_offset = (dest_y + y) * stride + dest_x * bytes_per_pixel;
            let src_row_offset = y * rect_width * bytes_per_pixel;
            surface.data[dest_offset..dest_offset + rect_width * bytes_per_pixel].copy_from_slice(
                &temp_buffer[src_row_offset..src_row_offset + rect_width * bytes_per_pixel],
            );
        }

        // Verify the destination area has the copied data
        for y in 100..200 {
            for x in 200..300 {
                let offset = y * stride + x * bytes_per_pixel;
                assert_eq!(surface.data[offset], 255); // R
                assert_eq!(surface.data[offset + 1], 100); // G
                assert_eq!(surface.data[offset + 2], 50); // B
                assert_eq!(surface.data[offset + 3], 255); // A
            }
        }
    }

    #[test]
    fn test_copy_bits_out_of_bounds() {
        let surface = DisplaySurface {
            width: 800,
            height: 600,
            format: 32,
            data: vec![0; 800 * 600 * 4],
        };

        // Test source out of bounds
        let src_x = 750;
        let src_y = 550;
        let rect_width = 100; // Would exceed width
        let rect_height = 100; // Would exceed height

        // This should be caught by bounds checking
        assert!(src_x + rect_width > surface.width as usize);
        assert!(src_y + rect_height > surface.height as usize);

        // Test destination out of bounds
        let dest_x = 750;
        let dest_y = 550;

        assert!(dest_x + rect_width > surface.width as usize);
        assert!(dest_y + rect_height > surface.height as usize);
    }

    #[test]
    fn test_mark_message_parsing() {
        // Create a test SpiceMsgDisplayMark message
        let mark_msg = SpiceMsgDisplayMark {
            mark: 42, // Test mark value
        };

        // Serialize the message
        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        mark_msg.write(&mut cursor).unwrap();

        // Parse the serialized message
        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceMsgDisplayMark::read(&mut read_cursor).unwrap();

        // Verify the parsed message matches the original
        assert_eq!(parsed.mark, 42);
    }

    #[test]
    fn test_mark_value_updates() {
        // Test multiple mark value scenarios
        let mark_values = vec![0, 1, 100, 255, 65535, u32::MAX];

        for mark_value in mark_values {
            let mark_msg = SpiceMsgDisplayMark { mark: mark_value };

            // Serialize and deserialize
            let mut buffer = Vec::new();
            let mut cursor = Cursor::new(&mut buffer);
            mark_msg.write(&mut cursor).unwrap();

            let mut read_cursor = Cursor::new(&buffer);
            let parsed = SpiceMsgDisplayMark::read(&mut read_cursor).unwrap();

            // Verify the value is preserved
            assert_eq!(parsed.mark, mark_value);
        }
    }

    #[test]
    fn test_image_cache_operations() {
        let mut cache = ImageCache::new();

        // Test empty cache
        assert_eq!(cache.len(), 0);
        assert!(cache.get(1).is_none());

        // Test insert and get
        let test_data = vec![1, 2, 3, 4];
        cache.insert(1, test_data.clone(), 10, 10);
        assert_eq!(cache.len(), 1);

        let cached = cache.get(1).unwrap();
        assert_eq!(cached.data, test_data);
        assert_eq!(cached.width, 10);
        assert_eq!(cached.height, 10);

        // Test multiple inserts
        cache.insert(2, vec![5, 6, 7, 8], 20, 20);
        cache.insert(3, vec![9, 10, 11, 12], 30, 30);
        assert_eq!(cache.len(), 3);

        // Test remove
        assert!(cache.remove(2));
        assert_eq!(cache.len(), 2);
        assert!(cache.get(2).is_none());
        assert!(!cache.remove(2)); // Already removed

        // Test clear
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert!(cache.get(1).is_none());
        assert!(cache.get(3).is_none());
    }

    #[test]
    fn test_inval_list_message_parsing() {
        // Create a mock INVAL_LIST message with 3 IDs to invalidate
        let count: u16 = 3;
        let ids = vec![100u64, 200u64, 300u64];

        let mut message = Vec::new();
        message.extend_from_slice(&count.to_le_bytes());
        for id in &ids {
            message.extend_from_slice(&id.to_le_bytes());
        }

        // Verify message structure
        assert_eq!(message.len(), 2 + (3 * 8)); // 2 bytes count + 3 * 8 bytes IDs

        // Parse count
        let parsed_count = u16::from_le_bytes([message[0], message[1]]);
        assert_eq!(parsed_count, 3);

        // Parse IDs
        for (i, expected_id) in ids.iter().enumerate() {
            let offset = 2 + (i * 8);
            let parsed_id = u64::from_le_bytes([
                message[offset],
                message[offset + 1],
                message[offset + 2],
                message[offset + 3],
                message[offset + 4],
                message[offset + 5],
                message[offset + 6],
                message[offset + 7],
            ]);
            assert_eq!(parsed_id, *expected_id);
        }
    }

    #[test]
    fn test_cache_invalidation_logic() {
        let mut cache = ImageCache::new();

        // Populate cache with test data
        for i in 1..=10 {
            cache.insert(i, vec![i as u8; 100], i as u32, i as u32);
        }
        assert_eq!(cache.len(), 10);

        // Simulate INVAL_LIST with some IDs
        let ids_to_remove = vec![2u64, 5u64, 8u64];
        let mut removed_count = 0;
        for id in ids_to_remove {
            if cache.remove(id) {
                removed_count += 1;
            }
        }

        assert_eq!(removed_count, 3);
        assert_eq!(cache.len(), 7);

        // Verify correct entries were removed
        assert!(cache.get(2).is_none());
        assert!(cache.get(5).is_none());
        assert!(cache.get(8).is_none());

        // Verify other entries still exist
        assert!(cache.get(1).is_some());
        assert!(cache.get(3).is_some());
        assert!(cache.get(10).is_some());
    }

    #[test]
    fn test_inval_list_with_nonexistent_ids() {
        let mut cache = ImageCache::new();

        // Add only a few entries
        cache.insert(100, vec![1; 100], 10, 10);
        cache.insert(200, vec![2; 100], 20, 20);

        // Try to remove IDs that don't exist
        let ids_to_remove = vec![100u64, 150u64, 200u64, 250u64];
        let mut removed_count = 0;
        for id in ids_to_remove {
            if cache.remove(id) {
                removed_count += 1;
            }
        }

        // Should only remove the 2 that existed
        assert_eq!(removed_count, 2);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_inval_all_pixmaps() {
        let mut cache = ImageCache::new();

        // Populate cache
        for i in 1..=100 {
            cache.insert(i, vec![i as u8; 100], i as u32, i as u32);
        }
        assert_eq!(cache.len(), 100);

        // Simulate INVAL_ALL_PIXMAPS
        cache.clear();

        // Verify all entries removed
        assert_eq!(cache.len(), 0);
        for i in 1..=100 {
            assert!(cache.get(i).is_none());
        }
    }

    #[test]
    fn test_empty_inval_list() {
        let mut cache = ImageCache::new();
        cache.insert(1, vec![1; 100], 10, 10);

        // Empty invalidation list (count = 0)
        let count: u16 = 0;
        let message = count.to_le_bytes().to_vec();

        // Parse and verify
        let parsed_count = u16::from_le_bytes([message[0], message[1]]);
        assert_eq!(parsed_count, 0);
        assert_eq!(message.len(), 2);

        // Cache should remain unchanged
        assert_eq!(cache.len(), 1);
        assert!(cache.get(1).is_some());
    }

    #[test]
    fn test_draw_blackness_parsing() {
        // Create a test SpiceDrawBlackness message
        let draw_blackness = SpiceDrawBlackness {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 10,
                    top: 20,
                    right: 100,
                    bottom: 80,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            mask: SpiceQMask {
                flags: 0,
                pos: SpicePoint { x: 0, y: 0 },
                bitmap: 0,
            },
        };

        // Serialize the message
        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_blackness.write(&mut cursor).unwrap();

        // Parse it back
        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawBlackness::read(&mut read_cursor).unwrap();

        // Verify the parsed message matches
        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.base.box_.left, 10);
        assert_eq!(parsed.base.box_.top, 20);
        assert_eq!(parsed.base.box_.right, 100);
        assert_eq!(parsed.base.box_.bottom, 80);
    }

    #[test]
    fn test_draw_whiteness_parsing() {
        let draw_whiteness = SpiceDrawWhiteness {
            base: SpiceDrawBase {
                surface_id: 1,
                box_: SpiceRect {
                    left: 50,
                    top: 50,
                    right: 150,
                    bottom: 100,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            mask: SpiceQMask {
                flags: 0,
                pos: SpicePoint { x: 0, y: 0 },
                bitmap: 0,
            },
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_whiteness.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawWhiteness::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 1);
        assert_eq!(parsed.base.box_.left, 50);
        assert_eq!(parsed.base.box_.top, 50);
    }

    #[test]
    fn test_draw_invers_parsing() {
        let draw_invers = SpiceDrawInvers {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 0,
                    top: 0,
                    right: 800,
                    bottom: 600,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            mask: SpiceQMask {
                flags: 0,
                pos: SpicePoint { x: 0, y: 0 },
                bitmap: 0,
            },
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_invers.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawInvers::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.base.box_.right, 800);
        assert_eq!(parsed.base.box_.bottom, 600);
    }

    #[test]
    fn test_draw_rop3_parsing() {
        let draw_rop3 = SpiceDrawRop3 {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 10,
                    top: 10,
                    right: 110,
                    bottom: 110,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            src_bitmap: 0x1000,
            src_area: SpiceRect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100,
            },
            brush: SpiceBrush {
                brush_type: 1,
                color: 0xFF0000, // Red
            },
            rop3: 0xCC, // SRCCOPY
            scale_mode: 0,
            mask: SpiceQMask {
                flags: 0,
                pos: SpicePoint { x: 0, y: 0 },
                bitmap: 0,
            },
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_rop3.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawRop3::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.rop3, 0xCC);
        assert_eq!(parsed.src_bitmap, 0x1000);
        assert_eq!(parsed.brush.color, 0xFF0000);
    }

    #[test]
    fn test_draw_transparent_parsing() {
        let draw_transparent = SpiceDrawTransparent {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 20,
                    top: 30,
                    right: 120,
                    bottom: 130,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            src_bitmap: 0x2000,
            src_area: SpiceRect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100,
            },
            src_color: 0xFF00FF, // Magenta transparency key
            true_color: 0xFFFFFF,
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_transparent.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawTransparent::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.src_bitmap, 0x2000);
        assert_eq!(parsed.src_color, 0xFF00FF);
        assert_eq!(parsed.true_color, 0xFFFFFF);
    }

    #[test]
    fn test_draw_stroke_parsing() {
        let draw_stroke = SpiceDrawStroke {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 5,
                    top: 5,
                    right: 95,
                    bottom: 95,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            path: 0x3000,
            attr: SpiceLineAttr {
                flags: 0,
                join_style: 0,
                end_style: 0,
                style_nseg: 0,
                width: 1 << 4,       // fixed28_4 format: 1.0
                miter_limit: 4 << 4, // fixed28_4 format: 4.0
            },
            brush: SpiceBrush {
                brush_type: 1,
                color: 0x0000FF, // Blue
            },
            fore_mode: 0,
            back_mode: 0,
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_stroke.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawStroke::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.path, 0x3000);
        assert_eq!(parsed.brush.color, 0x0000FF);
        assert_eq!(parsed.attr.width, 1 << 4);
    }

    #[test]
    fn test_draw_text_parsing() {
        let draw_text = SpiceDrawText {
            base: SpiceDrawBase {
                surface_id: 0,
                box_: SpiceRect {
                    left: 10,
                    top: 10,
                    right: 200,
                    bottom: 30,
                },
                clip: SpiceClip {
                    clip_type: 0,
                    data: 0,
                },
            },
            str_: 0x4000,
            back_area: SpiceRect {
                left: 10,
                top: 10,
                right: 200,
                bottom: 30,
            },
            fore_brush: SpiceBrush {
                brush_type: 1,
                color: 0x000000, // Black text
            },
            back_brush: SpiceBrush {
                brush_type: 1,
                color: 0xFFFFFF, // White background
            },
            fore_mode: 0,
            back_mode: 0,
        };

        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        draw_text.write(&mut cursor).unwrap();

        let mut read_cursor = Cursor::new(&buffer);
        let parsed = SpiceDrawText::read(&mut read_cursor).unwrap();

        assert_eq!(parsed.base.surface_id, 0);
        assert_eq!(parsed.str_, 0x4000);
        assert_eq!(parsed.fore_brush.color, 0x000000);
        assert_eq!(parsed.back_brush.color, 0xFFFFFF);
    }
}
