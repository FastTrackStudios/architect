//! The third face: `#[architect::service(actions)]` puts every `Greeter`
//! method on an Action List with one call, and `architect-mcp` in named
//! mode offers each as its own tool — so a model calling `greet` gets
//! what an HTTP client calling `/greeter/greet` gets, from the same
//! backend.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use architect::Services as _;
use architect::action::{ActionList, Effect};
use architect_mcp::{Server, Tools};
use example_greeter::{GreetError, GreeterHttpClient, Memory, greeter_actions};
use serde_json::{Value, json};

async fn serve_http(app: architect::http::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        architect::http::axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// The model's side: one JSON-RPC message, the reply.
async fn rpc(server: &Server, method: &str, params: &Value) -> Value {
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let reply = server.handle_async(&request.to_string()).await.expect("a reply");
    serde_json::from_str(&reply).expect("json")
}

async fn call(server: &Server, tool: &str, arguments: Value) -> (bool, Value) {
    let reply = rpc(server, "tools/call", &json!({"name": tool, "arguments": arguments})).await;
    let result = &reply["result"];
    let text = result["content"][0]["text"].as_str().expect("text");
    (
        result["isError"].as_bool().expect("isError"),
        serde_json::from_str(text).expect("json text"),
    )
}

fn list_for(backend: &Memory) -> ActionList {
    let list = ActionList::new();
    greeter_actions::register(backend.clone(), &list).expect("registered");
    list.validate().expect("a sound list");
    list
}

/// One call registers every method, each described from its doc comment
/// and typed by its HTTP body.
#[test]
fn one_call_puts_the_service_on_the_list() {
    let list = list_for(&Memory::default());
    assert_eq!(greeter_actions::IDS, ["GREETER_GREET", "GREETER_COUNT"]);
    let greet = list.info("GREETER_GREET").expect("listed");
    assert_eq!(greet.description, "Greets a known name.");
    assert_eq!(greet.display_name, "Greet");
    assert_eq!(greet.category, "Greeter");
    assert_eq!(greet.effect, Effect::Mutating);
    assert!(greet.params_schema.as_deref().is_some_and(|s| s.contains("\"name\"")));
    assert_eq!(list.info("GREETER_COUNT").expect("listed").effect, Effect::ReadOnly);
    assert!(list.is_async("GREETER_GREET"));
    list.probe_hostile_args().expect("refuses garbage without running");
}

/// `greet` is listed and described as its own MCP tool, and running it
/// answers what HTTP answers — success, failure and the count both see.
#[tokio::test]
async fn greet_over_mcp_is_greet_over_http() {
    let backend = Memory::default();
    let base = serve_http(backend.clone().into_http_router()).await;
    let http = GreeterHttpClient::at(base);
    let server = Server::new("greeter", "0", list_for(&backend))
        .with_tools(Tools::Named)
        .with_tool_name("GREETER_GREET", "greet")
        .with_tool_name("GREETER_COUNT", "count");

    let listed = rpc(&server, "tools/list", &json!({})).await;
    let tools = listed["result"]["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(names, ["count", "greet"]);
    let greet = tools.iter().find(|t| t["name"] == "greet").expect("greet");
    assert_eq!(greet["description"], "Greets a known name.");
    assert_eq!(greet["inputSchema"]["type"], "object");
    assert!(greet["inputSchema"]["properties"]["name"].is_object(), "{greet}");
    assert_eq!(greet["annotations"]["readOnlyHint"], false);

    let over_http = http.greet("cody".into()).await.expect("greeted over HTTP");
    let (failed, over_mcp) = call(&server, "greet", json!({"name": "cody"})).await;
    assert!(!failed);
    assert_eq!(over_mcp, json!(over_http));
    assert_eq!(over_mcp, json!("hello cody"));

    let refused = http.greet("nobody".into()).await.map_err(architect::ClientError::into_app);
    assert_eq!(refused, Err(GreetError::Unknown("nobody".into())));
    let (failed, error) = call(&server, "greet", json!({"name": "nobody"})).await;
    assert!(failed);
    assert_eq!(error["error"], GreetError::Unknown("nobody".into()).to_string());

    // One backend behind both faces: each saw the other's greeting.
    let (_, count) = call(&server, "count", json!({})).await;
    assert_eq!(count, json!(2));
    assert_eq!(http.count().await.expect("counted"), 2);
}
