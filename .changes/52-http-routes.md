---
"architect" = "minor"
"architect-rpc-derive" = "minor"
---

`#[architect::service]` routes beyond `POST`: `#[http(get | put | post | delete | any, path = "notes/{id}", limit = "1GiB")]` picks a method's verb, path and body cap, and `{param}` / `{*rest}` segments in the method path or the service `path = "org/{slug}"` bind the same-named argument. The typed HTTP client fills the path in and sends the verb; methods without any of this keep their `POST` route.
