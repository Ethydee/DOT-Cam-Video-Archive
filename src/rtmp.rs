use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// Manages RTMP ingest processes for cameras using stream keys.
/// For each stream key, runs an ffmpeg RTMP listener that writes HLS segments.
pub type IngestState = HashMap<String, tokio::process::Child>;

pub fn spawn_rtmp_listener(
    stream_key: String,
    rtmp_port: u16,
    state: Arc<Mutex<IngestState>>,
) {
    tokio::spawn(async move {
        let out_dir = format!("DATA/ingest/{}", stream_key);
        std::fs::create_dir_all(&out_dir).ok();

        let playlist = format!("{}/stream.m3u8", out_dir);
        let seg_pattern = format!("{}/seg_%03d.ts", out_dir);
        let listen_url = format!("rtmp://0.0.0.0:{}/live/{}", rtmp_port, stream_key);

        info!("Starting RTMP listener for key {} on port {}", stream_key, rtmp_port);

        let child = tokio::process::Command::new("ffmpeg")
            .args([
                "-listen", "1",
                "-i", &listen_url,
                "-c", "copy",
                "-f", "hls",
                "-hls_time", "4",
                "-hls_list_size", "900",  // ~1 hour of 4s segments
                "-hls_flags", "delete_segments+append_list",
                "-hls_segment_filename", &seg_pattern,
                &playlist,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();

        match child {
            Ok(proc) => {
                state.lock().await.insert(stream_key.clone(), proc);
                info!("RTMP listener active for key {}", stream_key);
            }
            Err(e) => {
                error!("Failed to start RTMP listener for key {}: {}", stream_key, e);
            }
        }
    });
}

pub async fn stop_rtmp_listener(stream_key: &str, state: &Arc<Mutex<IngestState>>) {
    let mut locked = state.lock().await;
    if let Some(mut child) = locked.remove(stream_key) {
        warn!("Stopping RTMP listener for key {}", stream_key);
        let _ = child.kill().await;
    }
}
