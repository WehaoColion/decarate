// v0.0.2 - Add an opt-in synthetic archive benchmark without accessing personal data.
// v0.0.1 - Guard repeat validation, byte corruption, validator scope and live SQLite sidecars.
use super::*;

struct Fixture {
    directory: PathBuf,
    backup: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let mut token = [0u8; 16];
        OsRng.fill_bytes(&mut token);
        let directory =
            std::env::temp_dir().join(format!("tenrate-archive-cache-{}", sha256_hex(&token)));
        fs::create_dir(&directory).unwrap();
        let store = SqliteServerStore::open(directory.join("live.sqlite3"), None).unwrap();
        let backup = directory.join("archive.sqlite3");
        store.create_verified_backup(&backup, 100).unwrap();
        Self { directory, backup }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
fn validations() -> usize {
    backup_verification::VALIDATIONS.with(|count| count.get())
}

#[test]
fn archive_verification_cache_reuses_exact_bytes_and_keeps_caller_identity() {
    let fixture = Fixture::new();
    let before = validations();
    let original = SqliteServerStore::verify_existing_backup(&fixture.backup, 100).unwrap();
    assert_eq!(validations(), before + 1);
    let copied = fixture.directory.join("copy.sqlite3");
    fs::copy(&fixture.backup, &copied).unwrap();
    let report = SqliteServerStore::verify_existing_backup(&copied, 200).unwrap();
    assert_eq!(
        validations(),
        before + 1,
        "byte-identical archive repeated all SQL checks"
    );
    assert_eq!(report.destination, copied);
    assert_eq!(report.created_at_epoch_millis, 200);
    assert_eq!(report.sha256, original.sha256);
    assert_eq!(report.server_instance_id, original.server_instance_id);
    assert!(
        preschema_privacy::verify(&fixture.backup, 300).is_err(),
        "a recovery validation must not authorize an unsupported older schema"
    );
}

#[test]
fn archive_verification_cache_rejects_same_size_modified_database() {
    let fixture = Fixture::new();
    SqliteServerStore::verify_existing_backup(&fixture.backup, 100).unwrap();
    let length = fs::metadata(&fixture.backup).unwrap().len();
    let connection = Connection::open(&fixture.backup).unwrap();
    connection
        .execute_batch("PRAGMA user_version=999;")
        .unwrap();
    drop(connection);
    assert_eq!(fs::metadata(&fixture.backup).unwrap().len(), length);
    assert!(
        SqliteServerStore::verify_existing_backup(&fixture.backup, 200).is_err(),
        "cached archive bypassed changed schema validation"
    );
}

#[test]
fn archive_verification_cache_never_reuses_an_image_with_live_wal() {
    let fixture = Fixture::new();
    let connection = Connection::open(&fixture.backup).unwrap();
    connection
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    let before = validations();
    SqliteServerStore::verify_existing_backup(&fixture.backup, 100).unwrap();
    SqliteServerStore::verify_existing_backup(&fixture.backup, 200).unwrap();
    assert_eq!(
        validations(),
        before + 2,
        "live WAL images must always execute SQL validation"
    );
    let unchanged_main = sha256_file(&fixture.backup).unwrap();
    connection
        .execute_batch("PRAGMA user_version=999;")
        .unwrap();
    assert_eq!(sha256_file(&fixture.backup).unwrap(), unchanged_main);
    assert!(
        SqliteServerStore::verify_existing_backup(&fixture.backup, 300).is_err(),
        "validation ignored the newer schema in WAL"
    );
}

#[test]
#[ignore = "opt-in synthetic timing probe; no timing threshold in correctness tests"]
fn archive_verification_cache_benchmark_synthetic_history() {
    let fixture = Fixture::new();
    let store =
        SqliteServerStore::open(fixture.directory.join("benchmark-live.sqlite3"), None).unwrap();
    store
        .create_user(NewStoredUser {
            id: "synthetic".into(),
            email: "synthetic@example.test".into(),
            password_salt: "synthetic-salt".into(),
            password_hash: "synthetic-hash".into(),
            password_scheme: "legacy_sha256".into(),
            created_at_epoch_millis: 10,
            updated_at_epoch_millis: 10,
            app_data_json: "{}".into(),
            account_revision: 0,
        })
        .unwrap();
    let mut payload = vec![0u8; 128 * 1024];
    for revision in 0..16 {
        OsRng.fill_bytes(&mut payload);
        let data = serde_json::json!({"notes":[],"syntheticPayload":base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &payload)}).to_string();
        store
            .compare_and_swap_account("synthetic", revision, &data, 100 + revision)
            .unwrap();
    }
    let path = fixture.directory.join("benchmark.sqlite3");
    let backup = store.create_verified_backup(&path, 1000).unwrap();
    let first = std::time::Instant::now();
    SqliteServerStore::verify_existing_backup(&path, 1000).unwrap();
    let cold = first.elapsed().as_secs_f64();
    let count = validations();
    let repeated = std::time::Instant::now();
    for stamp in 1001..1006 {
        SqliteServerStore::verify_existing_backup(&path, stamp).unwrap();
    }
    assert_eq!(validations(), count);
    println!(
        "ARCHIVE_CACHE_BENCHMARK {}",
        serde_json::json!({"synthetic":true,"sizeBytes":backup.size_bytes,"historyWrites":16,"coldSeconds":cold,"warmMeanSeconds":repeated.elapsed().as_secs_f64()/5.0,"warmRepeats":5})
    );
}
