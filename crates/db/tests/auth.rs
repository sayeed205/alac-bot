use std::time::Duration;

use db::{connect, migrate, Auth};

#[tokio::test]
async fn authorization_and_migration_parity() {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://admin:password@localhost:5432/alac_bot_v2_test".to_owned()
        });
    let client = match tokio::time::timeout(Duration::from_secs(3), connect(&url)).await {
        Ok(Ok(client)) => client,
        Ok(Err(error)) => {
            eprintln!("skipping db integration tests: PostgreSQL unreachable: {error}");
            return;
        }
        Err(_) => {
            eprintln!("skipping db integration tests: PostgreSQL connection timed out");
            return;
        }
    };

    if let Err(error) = migrate(&client).await {
        eprintln!("skipping db integration tests: migrations failed: {error}");
        return;
    }
    migrate(&client).await.expect("migration is idempotent");

    let auth = Auth::new(client.clone(), 900_000_001);
    let user = 900_000_002;
    let chat = -900_000_003;
    let _ = auth.revoke(user).await;
    let _ = auth.revoke(chat).await;

    assert!(auth
        .authorize(user, Some("first"))
        .await
        .expect("authorize"));
    assert!(!auth
        .authorize(user, Some("updated"))
        .await
        .expect("re-authorize"));
    assert!(auth.is_authorized(user, None).await.expect("user auth"));
    assert!(auth
        .is_authorized(123, Some(user))
        .await
        .expect("chat auth"));
    assert!(!auth.is_authorized(123, Some(456)).await.expect("auth miss"));
    assert!(auth.is_admin(900_000_001));
    assert!(auth
        .is_authorized(900_000_001, None)
        .await
        .expect("admin auth"));

    assert!(auth
        .authorize(chat, Some("group"))
        .await
        .expect("group authorize"));
    let list = auth.list_authorized().await.expect("list authorized");
    let listed = list
        .iter()
        .find(|item| item.telegram_id == user)
        .expect("user listed");
    assert_eq!(listed.name.as_deref(), Some("updated"));
    assert!(listed.created_at.timestamp() > 0);

    assert!(auth.revoke(user).await.expect("revoke hit"));
    assert!(!auth.revoke(user).await.expect("revoke miss"));
    assert!(!auth.is_authorized(user, None).await.expect("revoked user"));
    assert!(auth.revoke(chat).await.expect("cleanup group"));
}
