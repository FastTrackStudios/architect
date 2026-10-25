//! The device address book over vox: two installs of one account find
//! each other and share the first one's org; nobody else sees them.

use architect::{LayerRouter, LocalServer, Scope};
use auth::transport::vox::AuthVoxService;
use auth::{
    AccountOrg, AuthFlowError, AuthServiceClient, DeviceRegistration, DeviceServiceClient,
    SignUpEmailPassword,
};
use uuid::Uuid;

async fn sign_up(client: &AuthServiceClient, email: &str) -> String {
    client
        .sign_up_email_password(SignUpEmailPassword {
            email: email.into(),
            password: "correct horse battery staple".into(),
            name: Some("Device Owner".into()),
            username: None,
            image: None,
            metadata_json: None,
            ip_address: None,
            user_agent: Some("devices-test".into()),
        })
        .await
        .expect("sign up")
        .token
}

fn registration(
    device_id: Uuid,
    name: &str,
    endpoint: &str,
    offer: Option<AccountOrg>,
) -> DeviceRegistration {
    DeviceRegistration {
        device_id,
        name: name.into(),
        platform: "ios".into(),
        endpoint_id: Some(endpoint.into()),
        relay_url: None,
        offer_org: offer,
    }
}

fn org(slug: &str) -> AccountOrg {
    AccountOrg {
        id: Uuid::new_v4(),
        slug: slug.into(),
        display_name: "Personal".into(),
    }
}

#[tokio::test]
async fn an_accounts_devices_find_each_other_and_share_the_first_org() {
    let auth = super::open_auth().await;
    let router = LayerRouter::new()
        .merge(auth::auth_service_layer(AuthVoxService::new(auth.clone())))
        .merge(auth::device_service_layer(AuthVoxService::new(auth)));
    let local = LocalServer::serve(router, Scope::new());
    let session: AuthServiceClient = local.establish().await.expect("auth client");
    let devices: DeviceServiceClient = local.establish().await.expect("device client");

    let me = sign_up(&session, "devices@example.com").await;
    let someone_else = sign_up(&session, "other@example.com").await;

    // The phone signs in first and offers its personal org.
    let phone = Uuid::new_v4();
    let phone_org = org("personal");
    let after_phone = devices
        .register_device(
            me.clone(),
            registration(
                phone,
                "Cody's iPhone",
                "phoneendpoint01",
                Some(phone_org.clone()),
            ),
        )
        .await
        .expect("register phone");
    assert_eq!(after_phone.account_org.as_ref(), Some(&phone_org));
    assert_eq!(after_phone.devices.len(), 1);

    // The laptop offers its own personal org too — the phone's stands.
    let laptop = Uuid::new_v4();
    let after_laptop = devices
        .register_device(
            me.clone(),
            registration(laptop, "Studio Mac", "macendpoint02", Some(org("personal"))),
        )
        .await
        .expect("register laptop");
    assert_eq!(
        after_laptop.account_org.as_ref(),
        Some(&phone_org),
        "the first device's org is the account's"
    );
    let endpoints: Vec<_> = after_laptop
        .devices
        .iter()
        .map(|d| d.endpoint_id.clone().unwrap_or_default())
        .collect();
    assert_eq!(endpoints, vec!["phoneendpoint01", "macendpoint02"]);

    // Re-registering updates in place: a new endpoint, no second row.
    let moved = devices
        .register_device(
            me.clone(),
            registration(phone, "Cody's iPhone", "phoneendpoint03", None),
        )
        .await
        .expect("re-register phone");
    assert_eq!(moved.devices.len(), 2);
    assert!(
        moved
            .devices
            .iter()
            .any(|d| d.endpoint_id.as_deref() == Some("phoneendpoint03"))
    );

    // Another account sees none of it, and cannot take the phone's id.
    let theirs = devices
        .list_devices(someone_else.clone())
        .await
        .expect("list theirs");
    assert!(theirs.account_org.is_none() && theirs.devices.is_empty());
    let stolen = devices
        .register_device(
            someone_else.clone(),
            registration(phone, "Not yours", "x", None),
        )
        .await;
    assert!(
        matches!(&stolen, Err(vox::VoxError::User(b)) if matches!(**b, AuthFlowError::PermissionDenied)),
        "{stolen:?}"
    );
    assert!(matches!(
        &devices.forget_device(someone_else, laptop).await,
        Err(vox::VoxError::User(b)) if matches!(**b, AuthFlowError::PermissionDenied)
    ));

    // Forgetting the laptop takes it off the list.
    devices
        .forget_device(me.clone(), laptop)
        .await
        .expect("forget laptop");
    let left = devices.list_devices(me.clone()).await.expect("list mine");
    assert_eq!(
        left.devices.iter().map(|d| d.device_id).collect::<Vec<_>>(),
        vec![phone]
    );

    // A bad token is refused before anything is read.
    assert!(devices.list_devices("not-a-token".into()).await.is_err());
}

#[tokio::test]
async fn a_devices_relay_reaches_its_peers() {
    let auth = super::open_auth().await;
    let router = LayerRouter::new()
        .merge(auth::auth_service_layer(AuthVoxService::new(auth.clone())))
        .merge(auth::device_service_layer(AuthVoxService::new(auth)));
    let local = LocalServer::serve(router, Scope::new());
    let session: AuthServiceClient = local.establish().await.expect("auth client");
    let devices: DeviceServiceClient = local.establish().await.expect("device client");
    let me = sign_up(&session, "relay@example.com").await;

    let mut phone = registration(Uuid::new_v4(), "Phone", "phoneendpoint01", None);
    phone.relay_url = Some(" https://use1-1.relay.example/ ".into());
    devices
        .register_device(me.clone(), phone.clone())
        .await
        .expect("register");
    let listed = devices.list_devices(me.clone()).await.expect("list");
    assert_eq!(
        listed.devices[0].relay_url.as_deref(),
        Some("https://use1-1.relay.example/"),
        "trimmed and kept"
    );

    // Registering again without one clears it: the device moved off it.
    phone.relay_url = None;
    devices
        .register_device(me.clone(), phone)
        .await
        .expect("again");
    let listed = devices.list_devices(me).await.expect("list");
    assert_eq!(listed.devices[0].relay_url, None);
}

#[tokio::test]
async fn two_apps_on_one_device_are_one_device() {
    let auth = super::open_auth().await;
    let router = LayerRouter::new()
        .merge(auth::auth_service_layer(AuthVoxService::new(auth.clone())))
        .merge(auth::device_service_layer(AuthVoxService::new(auth)));
    let local = LocalServer::serve(router, Scope::new());
    let session: AuthServiceClient = local.establish().await.expect("auth client");
    let devices: DeviceServiceClient = local.establish().await.expect("device client");
    let me = sign_up(&session, "one-ipad@example.com").await;

    // Both apps derive the id from the device (`auth_client::this_device`).
    let ipad = Uuid::new_v4();
    let mut task = registration(ipad, "iPad", "taskendpoint0001", None);
    task.relay_url = Some("https://use1-1.relay.example/".into());
    devices
        .register_device(me.clone(), task)
        .await
        .expect("task registers");
    // Studio holds no node: it says nothing of an endpoint.
    let mut studio = registration(ipad, "iPad", "unused", None);
    studio.endpoint_id = None;
    let listed = devices
        .register_device(me, studio)
        .await
        .expect("studio registers");

    assert_eq!(listed.devices.len(), 1, "one iPad, not two");
    let only = &listed.devices[0];
    assert_eq!(
        only.endpoint_id.as_deref(),
        Some("taskendpoint0001"),
        "the node's endpoint stays"
    );
    assert_eq!(
        only.relay_url.as_deref(),
        Some("https://use1-1.relay.example/")
    );
}

#[tokio::test]
async fn a_registration_is_checked_before_it_is_stored() {
    let auth = super::open_auth().await;
    let router = LayerRouter::new()
        .merge(auth::auth_service_layer(AuthVoxService::new(auth.clone())))
        .merge(auth::device_service_layer(AuthVoxService::new(auth)));
    let local = LocalServer::serve(router, Scope::new());
    let session: AuthServiceClient = local.establish().await.expect("auth client");
    let devices: DeviceServiceClient = local.establish().await.expect("device client");
    let me = sign_up(&session, "checks@example.com").await;

    let mut nameless = registration(Uuid::new_v4(), "  ", "abc", None);
    assert!(matches!(
        &devices.register_device(me.clone(), nameless.clone()).await,
        Err(vox::VoxError::User(b)) if matches!(**b, AuthFlowError::InvalidInput(_))
    ));
    nameless.name = "Phone".into();
    nameless.endpoint_id = Some("not an id!".into());
    assert!(matches!(
        &devices.register_device(me.clone(), nameless.clone()).await,
        Err(vox::VoxError::User(b)) if matches!(**b, AuthFlowError::InvalidInput(_))
    ));
    nameless.endpoint_id = None;
    for bad in ["ftp://relay.example", "https://", "https:///path"] {
        nameless.relay_url = Some(bad.into());
        assert!(
            matches!(
                &devices.register_device(me.clone(), nameless.clone()).await,
                Err(vox::VoxError::User(b)) if matches!(**b, AuthFlowError::InvalidInput(_))
            ),
            "{bad} is refused"
        );
    }
    nameless.relay_url = None;
    nameless.offer_org = Some(AccountOrg {
        id: Uuid::new_v4(),
        slug: "Has Spaces".into(),
        display_name: "x".into(),
    });
    assert!(matches!(
        &devices.register_device(me.clone(), nameless).await,
        Err(vox::VoxError::User(b)) if matches!(**b, AuthFlowError::InvalidInput(_))
    ));
    let none = devices.list_devices(me).await.expect("list");
    assert!(
        none.devices.is_empty() && none.account_org.is_none(),
        "nothing half-stored"
    );
}
