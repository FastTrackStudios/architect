//! A project shared into another org through a Space (#443): the project
//! has one home org, which owns its note folder and its files root; a
//! collaborating org's people reach exactly that, and nothing else of the
//! home org's vault. (`fasttrackaudio/Projects/Crescendum` with
//! tombrooksmusic as the collaborator is the real case; merging the real
//! data is the person's call, never automatic.)
#![allow(clippy::unwrap_used)]

use architect_orgs::{OrgId, Role};
use architect_spaces::{EDITOR, Member, OrgSpaces, Space, SpaceId, SpacePolicy, Standing, Subject};

const NOW: i64 = 1_793_880_000_000_000;

fn crescendum() -> OrgSpaces {
    let tom = Member {
        subject: Subject::User("tom".to_owned()),
        role: Some(EDITOR.to_owned()),
        grants: Vec::new(),
        added_by: "user:cody".to_owned(),
        added_us: NOW,
        expires_us: None,
    };
    OrgSpaces {
        policy: SpacePolicy::default(),
        spaces: vec![Space {
            id: SpaceId::new("crescendum"),
            org: OrgId::new("fasttrackaudio"),
            name: "Crescendum".to_owned(),
            kind: "project".to_owned(),
            // Its note folder and its files root (#347), both the home org's.
            scope: vec![
                "vault/Projects/Crescendum/".to_owned(),
                "files/fasttrackaudio/Projects/Crescendum/".to_owned(),
            ],
            exclude: Vec::new(),
            roles: architect_spaces::builtin_roles(),
            members: vec![tom],
            created_by: "user:cody".to_owned(),
            created_us: NOW,
        }],
    }
}

#[test]
fn a_collaborating_org_reaches_the_shared_project_and_nothing_else() {
    let spaces = crescendum();
    // Tom is tombrooksmusic's, not fasttrackaudio's: an outsider here.
    let tom = Subject::User("tom".to_owned());
    let outside = Standing::of(&tom, None);
    assert_eq!(outside, Standing::External);
    for (resource, action) in [
        ("vault/Projects/Crescendum/Tracklist.md", "read"),
        ("vault/Projects/Crescendum/Songs/Overture.md", "write"),
        (
            "files/fasttrackaudio/Projects/Crescendum/Stems/kick.wav",
            "download",
        ),
    ] {
        assert!(
            spaces.allows(&tom, &outside, resource, action, NOW),
            "{action} {resource}"
        );
    }
    // The home org's other topics stay its own.
    for resource in [
        "vault/Health/Run.md",
        "vault/Projects/Other Album/project.md",
        "files/fasttrackaudio/Projects/Other Album/mix.wav",
    ] {
        assert!(
            !spaces.allows(&tom, &outside, resource, "read", NOW),
            "{resource}"
        );
    }
    // Managing it is the home org's: the policy caps outsiders.
    assert!(!spaces.allows(
        &tom,
        &outside,
        "vault/Projects/Crescendum/Tracklist.md",
        "admin",
        NOW
    ));
}

#[test]
fn someone_outside_the_space_reaches_none_of_it() {
    let spaces = crescendum();
    let eve = Subject::User("eve".to_owned());
    for standing in [Standing::External, Standing::Member(Role::Viewer)] {
        assert!(!spaces.allows(
            &eve,
            &standing,
            "vault/Projects/Crescendum/Tracklist.md",
            "read",
            NOW
        ));
    }
}
