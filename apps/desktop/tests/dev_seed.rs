//! The demo seed's share links (#541, `just dev-seed`): one per audience
//! kind, kept as share notes of the node's `demo` vault in
//! `scripts/dev-stack/seed-vault/_architect/shares/`, with the same codes
//! on every machine so the docs and the stack's test can name them.
//!
//! The notes are what `architect_spaces::share_note` writes for these
//! shares; this test fails when the two drift. `DEV_SEED_WRITE=1` writes
//! them again (after a change to the share note's format).

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use architect_spaces::share::{code_from, share_note_path};
use architect_spaces::{
    Audience, Share, ShareAccess, ShareAudience, ShareScope, parse_share_note, share_note,
};

/// The seeded links: public, link, org, and a link with two named audiences.
fn seeded() -> Vec<Share> {
    let share = |seed: u8, scope: ShareScope, title: &str, audience: Audience| Share {
        code: code_from([0x54, 0x1d, 0x5e, 0xed, 0, 0, 0, 0, 0, seed]),
        vault: "demo".to_owned(),
        scope,
        title: Some(title.to_owned()),
        audience,
        access: ShareAccess::Read,
        expires_at: None,
        revoked_at: None,
        created_at: 1_794_355_200,
        created_by: "dev-seed".to_owned(),
        domain: None,
        audiences: Vec::new(),
    };
    let mut setlist = share(
        4,
        ShareScope::Folder("Setlists/Fall Show".to_owned()),
        "Fall Show",
        Audience::Link,
    );
    setlist.audiences = [("violin", 5), ("cello", 6)]
        .into_iter()
        .map(|(name, seed)| ShareAudience {
            name: name.to_owned(),
            filter: format!("instrument == \"{name}\""),
            code: code_from([0x54, 0x1d, 0x5e, 0xed, 0, 0, 0, 0, 0, seed]),
            created_at: 1_794_355_200,
        })
        .collect();
    vec![
        share(
            1,
            ShareScope::Folder("Areas/Music".to_owned()),
            "Music notes",
            Audience::Public,
        ),
        share(
            2,
            ShareScope::Folder("Songs".to_owned()),
            "Songs",
            Audience::Link,
        ),
        share(
            3,
            ShareScope::Folder("Projects/Album".to_owned()),
            "The album",
            Audience::Org,
        ),
        setlist,
    ]
}

fn seed_vault() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/dev-stack/seed-vault")
}

#[test]
fn the_seeded_share_notes_are_what_share_note_writes() {
    let write = std::env::var_os("DEV_SEED_WRITE").is_some();
    for share in seeded() {
        let path = seed_vault().join(share_note_path(&share.code));
        let want = share_note(&share);
        if write {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e} (DEV_SEED_WRITE=1 writes it)", path.display()));
        assert_eq!(have, want, "{} drifted (DEV_SEED_WRITE=1)", path.display());
        assert_eq!(parse_share_note(&have).as_ref(), Some(&share));
    }
}

#[test]
fn the_seed_has_one_link_per_audience_kind_and_an_audience_filter() {
    let shares = seeded();
    let kinds: Vec<_> = shares.iter().map(|s| s.audience.label()).collect();
    for kind in [Audience::Public, Audience::Link, Audience::Org] {
        assert!(kinds.contains(&kind.label()), "{kinds:?}");
    }
    assert!(shares.iter().any(|s| s.audiences.len() == 2));
    // The codes the docs and xtask/tests/dev_stack.rs name.
    let codes: Vec<_> = shares
        .iter()
        .flat_map(|s| std::iter::once(&s.code).chain(s.audiences.iter().map(|a| &a.code)))
        .map(String::as_str)
        .collect();
    assert_eq!(
        codes,
        [
            "ksqx75iaaaaaaaab",
            "ksqx75iaaaaaaaac",
            "ksqx75iaaaaaaaad",
            "ksqx75iaaaaaaaae",
            "ksqx75iaaaaaaaaf",
            "ksqx75iaaaaaaaag"
        ]
    );
}
