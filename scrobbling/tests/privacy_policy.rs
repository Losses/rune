fn policy_text() -> String {
    include_str!("../../PRIVACY.md")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

#[test]
fn policy_does_not_promise_that_all_data_stays_on_device() {
    let policy = policy_text();
    for claim in [
        "does not collect, store, or transmit any personal information",
        "never sends personal information",
        "operates entirely on your device",
        "is not transmitted to external servers",
        "does not share information with external parties",
        "since all data remains on your device",
    ] {
        assert!(
            !policy.contains(claim),
            "Unqualified privacy claim: {claim}"
        );
    }
}

#[test]
fn policy_discloses_optional_scrobbling() {
    let policy = policy_text();
    for topic in [
        "optional",
        "scrobbling",
        "last.fm",
        "libre.fm",
        "listenbrainz",
        "username",
        "password",
        "session key",
        "user token",
        "api key",
        "artist",
        "track title",
        "omits the album artist",
        "duration",
        "timestamp",
        "library id",
    ] {
        assert!(
            policy.contains(topic),
            "Missing scrobbling disclosure: {topic}"
        );
    }
}

#[test]
fn local_storage_discloses_saved_scrobbling_accounts() {
    let policy = policy_text();
    let (_, storage) = policy
        .split_once("**local data storage**")
        .expect("local data storage section");
    let (storage, _) = storage
        .split_once("**third-party services**")
        .expect("end of local data storage section");

    for topic in [
        "account record",
        "service id",
        "username",
        "credential reference",
        "at startup",
        "when you log out",
        "removes the account record",
    ] {
        assert!(
            storage.contains(topic),
            "Missing local scrobbling disclosure: {topic}"
        );
    }
}
