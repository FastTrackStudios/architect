---
title: H5 Secrets
tags: [showcase, showcase/H]
capability: H5
example: "Secrets and fetch"
---
A secret entered once and inserted by the host into a request header, never readable.

**Stands in for:** Copilot's API key, Remotely Save tokens.

## Try

1. Start its echo server first: `python3 -I examples/showcase/secrets-fetch/echo.py` (127.0.0.1:8765).
2. Run `Secrets and fetch: set token` (the app asks, in its own prompt), then `Secrets and fetch: call`.

## You should see

The local echo server reports the header arrived; the extension never saw the value.

**Example:** Secrets and fetch.

Its status on desktop, web and iOS is in [[Showcase]].
