use crate::channels::display::DisplayChannel;
use crate::channels::main::MainChannel;
use crate::error::{Result, SpiceError};
use crate::protocol::ChannelType;
use crate::video::{create_video_output, VideoOutput};

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use tracing::{error, info};

#[cfg(not(target_arch = "wasm32"))]
use tokio::task::JoinHandle;

// For WASM, we'll use a different approach for task handles
#[cfg(target_arch = "wasm32")]
type TaskHandle = (); // Placeholder since we can't cancel wasm tasks easily

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ChannelKey {
    channel_type: ChannelType,
    channel_id: u8,
}

impl ChannelKey {
    fn new(channel_type: ChannelType, channel_id: u8) -> Self {
        Self {
            channel_type,
            channel_id,
        }
    }
}

pub struct SpiceClient {
    host: String,
    port: u16,
    #[cfg(target_arch = "wasm32")]
    websocket_url: Option<String>,
    #[cfg(target_arch = "wasm32")]
    auth_token: Option<String>,
    password: Option<String>,
    main_channel: Option<MainChannel>,
    display_channels: HashMap<u8, DisplayChannel>,
    #[cfg(not(target_arch = "wasm32"))]
    channel_tasks: Vec<JoinHandle<Result<()>>>,
    #[cfg(target_arch = "wasm32")]
    channel_tasks: Vec<TaskHandle>,
    video_output: Arc<dyn VideoOutput>,
    // Channel readiness tracking for WAIT_FOR_CHANNELS
    channel_readiness: Arc<Mutex<HashMap<ChannelKey, bool>>>,
    sync_notify: Arc<Notify>,
}

impl SpiceClient {
    pub fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
            #[cfg(target_arch = "wasm32")]
            websocket_url: None,
            #[cfg(target_arch = "wasm32")]
            auth_token: None,
            password: None,
            main_channel: None,
            display_channels: HashMap::new(),
            channel_tasks: Vec::new(),
            video_output: create_video_output(),
            channel_readiness: Arc::new(Mutex::new(HashMap::new())),
            sync_notify: Arc::new(Notify::new()),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn new_websocket(websocket_url: String) -> Self {
        Self::new_websocket_with_auth(websocket_url, None)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn new_websocket_with_auth(websocket_url: String, auth_token: Option<String>) -> Self {
        // Extract host/port from WebSocket URL for display purposes
        let (host, port) = if websocket_url.contains("://") {
            let without_protocol = websocket_url
                .split("://")
                .nth(1)
                .unwrap_or("localhost:8080");
            let parts: Vec<&str> = without_protocol.split(':').collect();
            let host = parts.first().unwrap_or(&"localhost").to_string();
            let port = parts
                .get(1)
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(8080);
            (host, port)
        } else {
            ("websocket".to_string(), 0)
        };

        Self {
            host,
            port,
            websocket_url: Some(websocket_url),
            auth_token,
            password: None,
            main_channel: None,
            display_channels: HashMap::new(),
            channel_tasks: Vec::new(),
            video_output: create_video_output(),
            channel_readiness: Arc::new(Mutex::new(HashMap::new())),
            sync_notify: Arc::new(Notify::new()),
        }
    }

    pub fn set_password(&mut self, password: String) {
        self.password = Some(password);
    }

    pub async fn connect(&mut self) -> Result<()> {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(ref ws_url) = self.websocket_url {
                info!("Connecting to SPICE server via WebSocket: {}", ws_url);
                // Connect to main channel via WebSocket
                let mut main_channel = MainChannel::new_websocket_with_password(
                    ws_url,
                    self.auth_token.clone(),
                    self.password.clone(),
                )
                .await?;
                main_channel.initialize().await?;

                // Get available channels
                let channels = main_channel.get_channels_list().await?;
                info!("Available channels: {:?}", channels);

                // TODO: For now, skip additional channels due to WebSocket proxy limitations
                // The proxy creates one TCP connection per WebSocket, but SPICE expects multiple TCP connections
                info!("Skipping additional channels - using main channel only for now");

                self.main_channel = Some(main_channel);
                return Ok(());
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            info!("Connecting to SPICE server at {}:{}", self.host, self.port);

            // Connect to main channel first
            info!("Creating main channel connection...");
            let mut main_channel = MainChannel::new(&self.host, self.port).await?;
            info!("Main channel created, initializing...");
            main_channel.initialize().await?;
            info!("Main channel initialized, getting channels list...");

            // Get the session_id from main channel
            let session_id = main_channel.get_session_id();
            info!("Got session_id from main channel: {:?}", session_id);

            // Get available channels
            let channels = main_channel.get_channels_list().await?;
            info!("Available channels: {:?}", channels);

            // Connect to display channels
            for (channel_type, channel_id) in channels {
                match channel_type {
                    ChannelType::Display => {
                        let display_channel = DisplayChannel::new_with_connection_id(
                            &self.host, self.port, channel_id, session_id,
                        )
                        .await?;
                        self.display_channels.insert(channel_id, display_channel);
                        info!(
                            "Connected to display channel {} with connection_id = {}",
                            channel_id,
                            session_id.unwrap_or(0)
                        );
                    }
                    _ => {
                        info!("Ignoring channel type {:?} id {}", channel_type, channel_id);
                    }
                }
            }

            self.main_channel = Some(main_channel);
            Ok(())
        }

        #[cfg(target_arch = "wasm32")]
        Err(SpiceError::Protocol(
            "No connection method available".to_string(),
        ))
    }

    pub async fn start_event_loop(&mut self) -> Result<()> {
        if self.main_channel.is_none() {
            return Err(SpiceError::Protocol(
                "Not connected to main channel".to_string(),
            ));
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            // Start main channel task
            if let Some(mut main_channel) = self.main_channel.take() {
                let main_task = tokio::spawn(async move { main_channel.run().await });
                self.channel_tasks.push(main_task);
            }

            // Start display channel tasks
            let display_channels = std::mem::take(&mut self.display_channels);
            for (channel_id, mut display_channel) in display_channels {
                let display_task = tokio::spawn(async move { display_channel.run().await });
                self.channel_tasks.push(display_task);
                info!("Started event loop for display channel {}", channel_id);
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            // In WASM, we use wasm_bindgen_futures::spawn_local for non-Send futures
            if let Some(mut main_channel) = self.main_channel.take() {
                wasm_bindgen_futures::spawn_local(async move {
                    if let Err(e) = main_channel.run().await {
                        error!("Main channel error: {}", e);
                    }
                });
                self.channel_tasks.push(()); // Placeholder since we can't track/cancel these
            }

            // Start display channel tasks
            let display_channels = std::mem::take(&mut self.display_channels);
            for (channel_id, mut display_channel) in display_channels {
                wasm_bindgen_futures::spawn_local(async move {
                    if let Err(e) = display_channel.run().await {
                        error!("Display channel {} error: {}", channel_id, e);
                    }
                });
                self.channel_tasks.push(()); // Placeholder since we can't track/cancel these
                info!("Started event loop for display channel {}", channel_id);
            }
        }

        Ok(())
    }

    pub async fn get_display_surface(
        &self,
        channel_id: u8,
    ) -> Option<&crate::channels::display::DisplaySurface> {
        self.display_channels
            .get(&channel_id)?
            .get_primary_surface()
    }

    pub fn get_video_output(&self) -> Arc<dyn VideoOutput> {
        self.video_output.clone()
    }

    pub async fn update_video_from_display(&self, channel_id: u8) -> Result<()> {
        if let Some(surface) = self.get_display_surface(channel_id).await {
            self.video_output.update_frame(surface).await;
        }
        Ok(())
    }

    pub async fn wait_for_completion(&mut self) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let tasks = std::mem::take(&mut self.channel_tasks);

            for task in tasks {
                match task.await {
                    Ok(Ok(())) => {
                        info!("Channel task completed successfully");
                    }
                    Ok(Err(e)) => {
                        error!("Channel task failed: {}", e);
                        return Err(e);
                    }
                    Err(e) => {
                        error!("Task join error: {e}");
                        return Err(SpiceError::Protocol(format!("Task join error: {e}")));
                    }
                }
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            // In WASM, we can't wait for spawned tasks, so just clear the placeholder list
            self.channel_tasks.clear();
            info!("WASM: Tasks are running in background, cannot wait for completion");
        }

        Ok(())
    }

    /// Mark a channel as ready and notify waiting tasks
    pub async fn mark_channel_ready(&self, channel_type: ChannelType, channel_id: u8) {
        let key = ChannelKey::new(channel_type, channel_id);
        let mut readiness = self.channel_readiness.lock().await;
        readiness.insert(key, true);
        drop(readiness); // Release lock before notifying
        self.sync_notify.notify_waiters();
        info!("Channel {:?}:{} marked as ready", channel_type, channel_id);
    }

    /// Check if a specific channel is ready
    pub async fn is_channel_ready(&self, channel_type: ChannelType, channel_id: u8) -> bool {
        let key = ChannelKey::new(channel_type, channel_id);
        let readiness = self.channel_readiness.lock().await;
        readiness.get(&key).copied().unwrap_or(false)
    }

    /// Wait until all specified channels are ready
    pub async fn wait_for_channels(&self, channels: &[(ChannelType, u8)]) -> Result<()> {
        use crate::utils::sleep;
        use instant::Duration;

        info!("Waiting for {} channels to be ready", channels.len());

        let timeout = Duration::from_secs(30);
        let start = instant::Instant::now();

        loop {
            // Check if all channels are ready
            let mut all_ready = true;
            for (channel_type, channel_id) in channels {
                if !self.is_channel_ready(*channel_type, *channel_id).await {
                    all_ready = false;
                    break;
                }
            }

            if all_ready {
                info!("All required channels are ready");
                return Ok(());
            }

            // Check timeout
            if start.elapsed() >= timeout {
                return Err(SpiceError::Protocol(
                    "Timeout waiting for channels to be ready".to_string(),
                ));
            }

            // Wait for notification with timeout
            #[cfg(not(target_arch = "wasm32"))]
            {
                tokio::select! {
                    _ = self.sync_notify.notified() => {},
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {},
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                sleep(Duration::from_millis(100)).await;
            }
        }
    }

    pub fn disconnect(&mut self) {
        info!("Disconnecting from SPICE server");

        // Cancel all running tasks (native only)
        #[cfg(not(target_arch = "wasm32"))]
        {
            for task in &self.channel_tasks {
                task.abort();
            }
        }
        self.channel_tasks.clear();

        // Clear channels and schedule video output clearing
        self.main_channel = None;
        self.display_channels.clear();

        // Video output will be cleared when new frames arrive
    }
}

impl Drop for SpiceClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}
