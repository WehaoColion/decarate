#[test]
fn mirror_proof_cleanup_reobserves_replaced_missing_and_unreadable_sources() {
    let (directory, store) = temp_store("mirror_observation_scope");
    let states = [
        state_with_sessions(2, 100),
        state_with_sessions(3, 200),
        state_with_sessions(4, 300),
    ];
    for (index, state) in states.iter().enumerate() {
        store
            .record("owner", state, 100 + index as i64, "local_save")
            .unwrap();
    }
    let primary = directory.join("timer_state_guest.json");
    let temporary =
        |generation| directory.join(format!("timer_state_guest.json.tmp-observe-{generation}"));
    let hashes = || {
        let connection = store.open_connection(false).unwrap();
        let mut statement = connection
            .prepare(
                "SELECT raw_sha256 FROM desktop_state_mirror_provenance
             WHERE owner='owner' AND source='timer_state_guest.json' ORDER BY raw_sha256",
            )
            .unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    };
    let writer = store.lock_recovery_mirror_writer().unwrap();
    fs::write(&primary, &states[0]).unwrap();
    assert!(writer
        .prepare("owner", &primary, &temporary(1), &states[0], None)
        .unwrap());
    assert!(writer
        .prepare("owner", &primary, &temporary(2), &states[1], None)
        .unwrap());
    assert_eq!(hashes().len(), 2);

    // The prior prepare saw generation one; a later prepare must observe the
    // replacement, discard only that stale proof, and retain generation two.
    fs::write(&primary, &states[1]).unwrap();
    assert!(writer
        .prepare("owner", &primary, &temporary(3), &states[2], None)
        .unwrap());
    let mut expected = vec![
        sha256_hex(states[1].as_bytes()),
        sha256_hex(states[2].as_bytes()),
    ];
    expected.sort();
    assert_eq!(hashes(), expected);
    assert!(store
        .recovery_mirror_is_verified("owner", &primary, &states[1])
        .unwrap());
    assert!(!store
        .recovery_mirror_is_verified("owner", &primary, &states[2])
        .unwrap());

    // Prepared evidence must not authorize a subsequent external replacement.
    fs::write(&primary, "{}").unwrap();
    assert!(!store
        .recovery_mirror_is_verified("owner", &primary, "{}")
        .unwrap());
    fs::remove_file(&primary).unwrap();
    fs::create_dir(&primary).unwrap();
    assert!(writer
        .prepare("owner", &primary, &temporary(4), &states[0], None)
        .is_err());
    assert_eq!(
        hashes(),
        expected,
        "unreadable generations must not be silently discarded"
    );

    // Missing is a new observation, so neither unreadable state nor old hashes
    // may leak across calls. New evidence still does not claim bytes exist.
    fs::remove_dir(&primary).unwrap();
    assert!(writer
        .prepare("owner", &primary, &temporary(4), &states[0], None)
        .unwrap());
    assert_eq!(hashes(), vec![sha256_hex(states[0].as_bytes())]);
    assert!(!store
        .recovery_mirror_is_verified("owner", &primary, &states[0])
        .unwrap());
    fs::write(&primary, &states[0]).unwrap();
    assert!(store
        .recovery_mirror_is_verified("owner", &primary, &states[0])
        .unwrap());
    drop(writer);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mirror_provenance_survives_history_pruning_but_prepared_proof_is_not_present() {
    let (directory, store) = temp_store("mirror_provenance_prune");
    let (_, sealed, _) = private_media_core_fixture(&store, "owner");
    store.record("owner", &sealed, 100, "local_save").unwrap();
    let primary = directory.join("timer_state_guest.json");
    let temporary = directory.join("timer_state_guest.json.tmp-proof");
    let writer = store.lock_recovery_mirror_writer().unwrap();
    assert!(writer
        .prepare("owner", &primary, &temporary, &sealed, None)
        .unwrap());
    drop(writer);
    assert!(!store
        .recovery_mirror_is_verified("owner", &primary, &sealed)
        .unwrap());
    let current = app_data::default_app_data_json(300);
    store.record("owner", &current, 300, "local_save").unwrap();
    let connection = store.open_connection(false).unwrap();
    connection
        .execute(
            "DELETE FROM desktop_state_snapshots WHERE raw_sha256=?1",
            params![sha256_hex(sealed.as_bytes())],
        )
        .unwrap();
    let pending = store.private_media_references("owner", &current).unwrap();
    assert!(
        pending
            .queries(&Default::default(), false)
            .unwrap()
            .is_empty(),
        "prepared metadata falsely claimed a file existed"
    );
    fs::write(&primary, &sealed).unwrap();
    let proven = store.private_media_references("owner", &current).unwrap();
    assert_eq!(
        proven.queries(&Default::default(), false).unwrap().len(),
        1,
        "pruning history stranded an independently proven mirror"
    );
    assert!(store
        .recovery_mirror_is_verified("owner", &primary, &sealed)
        .unwrap());
    let path = store.database_path().to_path_buf();
    drop(connection);
    drop(store);
    let reopened = DesktopStateStore::open(path).unwrap();
    assert_eq!(
        reopened
            .private_media_references("owner", &current)
            .unwrap()
            .queries(&Default::default(), false)
            .unwrap()
            .len(),
        1
    );
    drop(reopened);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mirror_provenance_rejects_foreign_owner_modified_bytes_and_tampered_binding() {
    let (directory, store) = temp_store("mirror_provenance_identity");
    let (_, sealed, _) = private_media_core_fixture(&store, "owner-a");
    store.record("owner-a", &sealed, 100, "local_save").unwrap();
    store
        .record(
            "owner-b",
            &app_data::default_app_data_json(100),
            100,
            "local_save",
        )
        .unwrap();
    let primary = directory.join("timer_state_guest.json");
    let temporary = directory.join("timer_state_guest.json.tmp-proof");
    let writer = store.lock_recovery_mirror_writer().unwrap();
    assert!(!writer
        .prepare("owner-b", &primary, &temporary, &sealed, None)
        .unwrap());
    assert!(writer
        .prepare("owner-a", &primary, &temporary, &sealed, None)
        .unwrap());
    drop(writer);
    fs::write(&primary, &sealed).unwrap();
    assert!(!store
        .recovery_mirror_is_verified("owner-b", &primary, &sealed)
        .unwrap());
    assert!(!store
        .recovery_mirror_is_verified("owner-a", &primary, &format!("{sealed} "))
        .unwrap());
    let connection = store.open_connection(false).unwrap();
    connection
        .execute(
            "UPDATE desktop_state_mirror_provenance SET binding=?1 WHERE owner='owner-a'",
            params!["0".repeat(64)],
        )
        .unwrap();
    assert!(store
        .recovery_mirror_is_verified("owner-a", &primary, &sealed)
        .is_err());
    drop(connection);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mirror_provenance_capacity_keeps_live_sources_and_failed_commit_rolls_back() {
    let (directory, store) = temp_store("mirror_provenance_capacity");
    let raw = media_retention_test_state();
    store.record("owner", &raw, 100, "local_save").unwrap();
    let writer = store.lock_recovery_mirror_writer().unwrap();
    // Each operation registers two source identities, and both files remain.
    for i in 0..16 {
        let target = directory.join(format!("timer_state_guest.json.invalid-{i}"));
        let temporary = directory.join(format!("timer_state_guest.json.tmp-{i}"));
        assert!(writer
            .prepare("owner", &target, &temporary, &raw, None)
            .unwrap());
        fs::write(&target, &raw).unwrap();
        fs::write(&temporary, &raw).unwrap();
    }
    let before = store.journal_evidence().unwrap();
    assert!(writer
        .prepare(
            "owner",
            &directory.join("timer_state_guest.json"),
            &directory.join("timer_state_guest.json.tmp-overflow"),
            &raw,
            None
        )
        .is_err());
    assert_eq!(store.journal_evidence().unwrap(), before);
    for i in 0..16 {
        assert!(store
            .recovery_mirror_is_verified(
                "owner",
                &directory.join(format!("timer_state_guest.json.invalid-{i}")),
                &raw
            )
            .unwrap());
    }
    drop(writer);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mirror_provenance_copied_journal_does_not_bind_another_workspace() {
    let (directory, store) = temp_store("mirror_provenance_workspace");
    let other = directory.join("other-workspace");
    fs::create_dir_all(&other).unwrap();
    let raw = media_retention_test_state();
    store.record("guest-v1", &raw, 100, "local_save").unwrap();
    let target = directory.join("timer_state_guest.json");
    let temporary = directory.join("timer_state_guest.json.tmp-proof");
    let writer = store.lock_recovery_mirror_writer().unwrap();
    writer
        .prepare("guest-v1", &target, &temporary, &raw, None)
        .unwrap();
    fs::write(&target, &raw).unwrap();
    let database = store.database_path().to_path_buf();
    drop(writer);
    drop(store);
    let copied_database = other.join("state_journal.sqlite3");
    fs::copy(database, &copied_database).unwrap();
    let copied_target = other.join("timer_state_guest.json");
    fs::write(&copied_target, &raw).unwrap();
    let copied = DesktopStateStore::open(copied_database).unwrap();
    assert!(
        copied
            .recovery_mirror_is_verified("guest-v1", &copied_target, &raw)
            .is_err(),
        "copying a journal and file minted provenance for another guest workspace"
    );
    drop(copied);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mirror_provenance_failed_metadata_commit_retains_previous_proof() {
    let (directory, store) = temp_store("mirror_provenance_rollback");
    let old = media_retention_test_state();
    store.record("owner", &old, 100, "local_save").unwrap();
    let target = directory.join("timer_state_guest.json");
    let temporary = directory.join("timer_state_guest.json.tmp-proof");
    let writer = store.lock_recovery_mirror_writer().unwrap();
    writer
        .prepare("owner", &target, &temporary, &old, None)
        .unwrap();
    fs::write(&target, &old).unwrap();
    let new = app_data::default_app_data_json(300);
    store.record("owner", &new, 300, "local_save").unwrap();
    let before = store.journal_evidence().unwrap();
    let connection = store.open_connection(false).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_mirror_metadata BEFORE UPDATE OF commit_sequence ON desktop_state_journal_metadata BEGIN SELECT RAISE(ABORT,'injected mirror failure'); END;").unwrap();
    assert!(writer
        .prepare("owner", &target, &temporary, &new, None)
        .is_err());
    assert_eq!(store.journal_evidence().unwrap(), before);
    assert!(store
        .recovery_mirror_is_verified("owner", &target, &old)
        .unwrap());
    assert!(!store
        .recovery_mirror_is_verified("owner", &target, &new)
        .unwrap());
    assert_eq!(fs::read_to_string(&target).unwrap(), old);
    drop(connection);
    drop(writer);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}
