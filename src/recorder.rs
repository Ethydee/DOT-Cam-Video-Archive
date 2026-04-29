use sqlx::{Row, SqlitePool};
use std::{
    collections::HashSet,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    process::Command,
    sync::Mutex,
    time::sleep,
};
use tracing::{error, info, warn};

/// Tracks which camera IDs currently have a recording ffmpeg process running.
pub type RecorderState = HashSet<i64>;

/// Spawns the background loop that continuously records segments for all cameras
/// that have `recording = 1`.
pub fn spawn_recorder_loop(pool: SqlitePool, state: Arc<Mutex<RecorderState>>) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_recording_cycle(&pool, &state).await {
                error!("Recorder cycle error: {}", e);
            }
            sleep(Duration::from_secs(15)).await;
        }
    });
}

async fn run_recording_cycle(
    pool: &SqlitePool,
    state: &Arc<Mutex<RecorderState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Fetch all cameras that should be recording
    let rows = sqlx::query(
        "SELECT id, name, source_type, stream_url, stream_key, rewind_hours FROM cameras WHERE recording = 1"
    )
    .fetch_all(pool)
    .await?;

    let mut active = state.lock().await;

    for row in &rows {
        let cam_id: i64 = row.get("id");
        let name: String = row.get("name");
        let source_type: String = row.get("source_type");
        let stream_url: Option<String> = row.get("stream_url");
        let stream_key: Option<String> = row.get("stream_key");
        let rewind_hours: i64 = row.get("rewind_hours");

        if active.contains(&cam_id) {
            continue; // already recording
        }

        // Determine the input URL
        let input_url = match source_type.as_str() {
            "hls" => {
                if let Some(url) = &stream_url {
                    url.clone()
                } else {
                    warn!("Camera {} ({}) has no stream_url, skipping", cam_id, name);
                    continue;
                }
            }
            "stream_key" => {
                if let Some(key) = &stream_key {
                    // The RTMP ingest writes HLS segments here
                    let ingest_path = format!("DATA/ingest/{}/stream.m3u8", key);
                    if !PathBuf::from(&ingest_path).exists() {
                        // No active ingest yet — skip
                        continue;
                    }
                    ingest_path
                } else {
                    warn!("Camera {} ({}) has no stream_key, skipping", cam_id, name);
                    continue;
                }
            }
            _ => {
                warn!("Camera {} has unknown source_type: {}", cam_id, source_type);
                continue;
            }
        };

        // Create output directory
        let out_dir = format!("DATA/segments/{}", cam_id);
        std::fs::create_dir_all(&out_dir).ok();

        let max_segments = rewind_hours * 3600 / 4; // 4-second segments

        info!("Starting recorder for camera {} ({})", cam_id, name);
        active.insert(cam_id);

        let state_clone = Arc::clone(state);
        let out_dir_clone = out_dir.clone();

        tokio::spawn(async move {
            let result = run_ffmpeg_recorder(&input_url, &out_dir_clone, max_segments).await;
            if let Err(e) = result {
                error!("FFmpeg recorder for camera {} died: {}", cam_id, e);
            }
            // Remove from active set so it gets restarted
            state_clone.lock().await.remove(&cam_id);
            info!("Recorder for camera {} stopped, will restart on next cycle", cam_id);
        });
    }

    Ok(())
}

async fn run_ffmpeg_recorder(
    input_url: &str,
    out_dir: &str,
    _max_segments: i64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Use a monotonic counter (%d) for unique filenames — strftime with %S
    // causes collisions when multiple segments are written in the same second.
    let segment_pattern = format!("{}/seg_%06d.ts", out_dir);
    let playlist_path = format!("{}/live.m3u8", out_dir);

    let mut child = Command::new("ffmpeg")
        .args([
            "-i", input_url,
            // Output 1: HLS Stream
            "-map", "0",
            "-c", "copy",
            "-f", "hls",
            "-hls_time", "4",
            // Small live window — only the latest 10 segments in the playlist.
            // All segments stay on disk for DVR; our cleanup loop handles deletion.
            "-hls_list_size", "10",
            // No delete_segments (we manage deletion), no append_list (avoids
            // the playlist growing unboundedly and causing parse lag in hls.js).
            "-hls_flags", "temp_file",
            "-hls_segment_filename", &segment_pattern,
            &playlist_path,
            
            // Output 2: Thumbnail Image (1 frame every 10 seconds)
            "-map", "0:v",
            "-vf", "fps=1/10,scale=320:-1",
            "-update", "1",
            &format!("{}/thumb.jpg", out_dir),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;

    // Wait for ffmpeg to finish (or crash)
    child.wait().await?;
    Ok(())
}

/// Clean up old segments that exceed the rewind window.
pub fn spawn_cleanup_loop(pool: SqlitePool) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = cleanup_old_segments(&pool).await {
                error!("Cleanup error: {}", e);
            }
            sleep(Duration::from_secs(300)).await; // every 5 minutes
        }
    });
}

async fn cleanup_old_segments(pool: &SqlitePool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let rows = sqlx::query("SELECT id, rewind_hours FROM cameras")
        .fetch_all(pool)
        .await?;

    for row in &rows {
        let cam_id: i64 = row.get("id");
        let rewind_hours: i64 = row.get("rewind_hours");
        let seg_dir = format!("DATA/segments/{}", cam_id);
        let dir = PathBuf::from(&seg_dir);

        if !dir.exists() {
            continue;
        }

        let cutoff = chrono::Utc::now() - chrono::Duration::hours(rewind_hours);
        let cutoff_ts = cutoff.timestamp();

        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "ts") {
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        if let Ok(modified) = metadata.modified() {
                            let mod_ts = modified
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs() as i64;
                            if mod_ts < cutoff_ts {
                                std::fs::remove_file(&path).ok();
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
