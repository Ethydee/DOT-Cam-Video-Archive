use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use tracing::info;

pub async fn init_db() -> Result<SqlitePool, sqlx::Error> {
    std::fs::create_dir_all("DATA").ok();

    let db_url = "sqlite:DATA/app.db?mode=rwc";
    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect(db_url)
        .await?;

    // ── Run migrations ──────────────────────────────────────────────────
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'user',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS cameras (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            location TEXT,
            source_type TEXT NOT NULL DEFAULT 'hls',
            stream_url TEXT,
            stream_key TEXT UNIQUE,
            rewind_hours INTEGER NOT NULL DEFAULT 24,
            recording INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )"
    )
    .execute(&pool)
    .await?;

    // ── Seed defaults ───────────────────────────────────────────────────
    // Default admin account
    let admin_exists: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM users WHERE username = 'admin'"
    )
    .fetch_one(&pool)
    .await
    .unwrap_or(false);

    if !admin_exists {
        let hash = bcrypt::hash("admin", 10).expect("bcrypt hash failed");
        sqlx::query("INSERT INTO users (username, password_hash, role) VALUES ('admin', ?, 'admin')")
            .bind(&hash)
            .execute(&pool)
            .await?;
        info!("Created default admin account (admin/admin)");
    }

    // Default settings
    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('default_rewind_hours', '24')")
        .execute(&pool)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('rtmp_port', '1935')")
        .execute(&pool)
        .await?;

    info!("Database initialized at DATA/app.db");
    Ok(pool)
}
