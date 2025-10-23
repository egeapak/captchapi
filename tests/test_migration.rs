use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[tokio::test]
async fn test_migration_creates_tables() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(":memory:")
                .create_if_missing(true),
        )
        .await
        .expect("Failed to create test database");

    // Run migrations
    let result = sqlx::migrate!("./migrations").run(&pool).await;
    println!("Migration result: {:?}", result);

    if let Err(e) = result {
        panic!("Migration failed: {:?}", e);
    }

    // Check if tables exist
    let sessions_check =
        sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name='sessions'")
            .fetch_optional(&pool)
            .await
            .expect("Failed to query sessions table");

    println!("Sessions table exists: {}", sessions_check.is_some());

    let api_keys_check =
        sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name='api_keys'")
            .fetch_optional(&pool)
            .await
            .expect("Failed to query api_keys table");

    println!("API keys table exists: {}", api_keys_check.is_some());

    assert!(sessions_check.is_some(), "sessions table should exist");
    assert!(api_keys_check.is_some(), "api_keys table should exist");
}
