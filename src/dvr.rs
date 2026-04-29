use std::path::PathBuf;

/// Generate a DVR-style HLS playlist from cached segments on disk.
/// `cam_id`     — camera identifier (segments stored in DATA/segments/{cam_id}/)
/// `from_ts`    — optional start timestamp (Unix seconds). If None, returns latest segments.
/// `duration`   — how many seconds of content to include (default: 3600 = 1 hour)
/// `live`       — if true, include EXT-X-ENDLIST: NO so the player stays in live mode
pub fn generate_playlist(
    cam_id: i64,
    from_ts: Option<i64>,
    duration: Option<i64>,
    live: bool,
) -> Result<String, String> {
    let seg_dir = format!("DATA/segments/{}", cam_id);
    let dir = PathBuf::from(&seg_dir);

    if !dir.exists() {
        return Err("No segments found for this camera".into());
    }

    // Collect all .ts files with their modification timestamps
    let mut segments: Vec<(i64, String)> = Vec::new();

    let entries = std::fs::read_dir(&dir).map_err(|e| format!("Cannot read dir: {}", e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "ts") {
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(modified) = meta.modified() {
                    let ts = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    let filename = path.file_name().unwrap().to_string_lossy().to_string();
                    segments.push((ts, filename));
                }
            }
        }
    }

    if segments.is_empty() {
        return Err("No segments available".into());
    }

    // Sort by timestamp
    segments.sort_by_key(|(ts, _)| *ts);

    let dur = duration.unwrap_or(3600);

    // Filter based on time range
    let filtered: Vec<&(i64, String)> = if let Some(start) = from_ts {
        let end = start + dur;
        segments.iter().filter(|(ts, _)| *ts >= start && *ts <= end).collect()
    } else {
        // Latest segments — take from the end
        let max_segs = (dur / 4).max(1) as usize; // 4-second segments
        let start_idx = segments.len().saturating_sub(max_segs);
        segments[start_idx..].iter().collect()
    };

    if filtered.is_empty() {
        return Err("No segments in the requested time range".into());
    }

    // Build M3U8
    let mut playlist = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:5\n");

    // Use the index of first segment as media sequence
    let first_ts = filtered.first().map(|(ts, _)| *ts).unwrap_or(0);
    let media_seq = (first_ts / 4) as u64;
    playlist.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{}\n", media_seq));

    let mut prev_ts: Option<i64> = None;
    for (ts, filename) in &filtered {
        // Detect gaps: if time between segments exceeds expected duration + tolerance,
        // insert a discontinuity tag so hls.js resets its decoder and skips the gap.
        if let Some(prev) = prev_ts {
            if *ts - prev > 6 {
                playlist.push_str("#EXT-X-DISCONTINUITY\n");
            }
        }
        prev_ts = Some(*ts);

        playlist.push_str("#EXTINF:4.0,\n");
        playlist.push_str(&format!("/api/segments/{}/{}\n", cam_id, filename));
    }

    if !live {
        playlist.push_str("#EXT-X-ENDLIST\n");
    }

    Ok(playlist)
}

/// Generate a complete VOD playlist with ALL available segments for a camera.
/// Used for smooth DVR scrubbing — the frontend loads this once and seeks
/// with video.currentTime instead of loading new playlists per position.
pub fn generate_full_playlist(cam_id: i64, max_hours: i64) -> Result<String, String> {
    let seg_dir = format!("DATA/segments/{}", cam_id);
    let dir = PathBuf::from(&seg_dir);

    if !dir.exists() {
        return Err("No segments found for this camera".into());
    }

    let mut segments: Vec<(i64, String)> = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("Cannot read dir: {}", e))?;

    let cutoff = chrono::Utc::now().timestamp() - (max_hours * 3600);

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "ts") {
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(modified) = meta.modified() {
                    let ts = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    if ts >= cutoff {
                        let filename = path.file_name().unwrap().to_string_lossy().to_string();
                        segments.push((ts, filename));
                    }
                }
            }
        }
    }

    if segments.is_empty() {
        return Err("No segments available".into());
    }

    segments.sort_by_key(|(ts, _)| *ts);

    let mut playlist = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:5\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MEDIA-SEQUENCE:0\n");

    let mut prev_ts: Option<i64> = None;
    for (ts, filename) in &segments {
        if let Some(prev) = prev_ts {
            if *ts - prev > 6 {
                playlist.push_str("#EXT-X-DISCONTINUITY\n");
            }
        }
        prev_ts = Some(*ts);
        playlist.push_str("#EXTINF:4.0,\n");
        playlist.push_str(&format!("/api/segments/{}/{}\n", cam_id, filename));
    }

    playlist.push_str("#EXT-X-ENDLIST\n");
    Ok(playlist)
}

/// List available dates that have archived footage for a camera.
pub fn list_archive_dates(cam_id: i64) -> Vec<String> {
    let seg_dir = format!("DATA/segments/{}", cam_id);
    let dir = PathBuf::from(&seg_dir);
    let mut dates: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "ts") {
                // Filename pattern: seg_YYYYMMDD_HHMMSS.ts
                let fname = path.file_name().unwrap().to_string_lossy().to_string();
                if fname.starts_with("seg_") && fname.len() >= 12 {
                    let date_part = &fname[4..12]; // YYYYMMDD
                    if let Ok(_) = chrono::NaiveDate::parse_from_str(date_part, "%Y%m%d") {
                        let formatted = format!(
                            "{}-{}-{}",
                            &date_part[0..4],
                            &date_part[4..6],
                            &date_part[6..8]
                        );
                        dates.insert(formatted);
                    }
                }
            }
        }
    }

    dates.into_iter().collect()
}

/// Returns (oldest_ts, newest_ts, duration_seconds) for available segments.
/// Used by the frontend to set the DVR slider to the actual recording length.
pub fn get_segment_range(cam_id: i64) -> Option<(i64, i64, i64)> {
    let seg_dir = format!("DATA/segments/{}", cam_id);
    let dir = PathBuf::from(&seg_dir);

    if !dir.exists() {
        return None;
    }

    let mut oldest: Option<i64> = None;
    let mut newest: Option<i64> = None;

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "ts") {
                if let Ok(meta) = std::fs::metadata(&path) {
                    if let Ok(modified) = meta.modified() {
                        let ts = modified
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;
                        oldest = Some(oldest.map_or(ts, |o: i64| o.min(ts)));
                        newest = Some(newest.map_or(ts, |n: i64| n.max(ts)));
                    }
                }
            }
        }
    }

    match (oldest, newest) {
        (Some(o), Some(n)) => Some((o, n, n - o)),
        _ => None,
    }
}
