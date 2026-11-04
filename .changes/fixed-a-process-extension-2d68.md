---
"architect" = "minor"
"architect-extensions" = "minor"
"architect-lab-board" = "none"
"architect-rpc-derive" = "minor"
---

Fixed a process extension stopping at 64 views: the host opens the process's vox lanes with room for `wire::MAX_VIEWS` (256) views at once, and a process at `wire::VIEWS_WIRE` (6, now `WIRE`) ends a view's request when it closes, so its place is given back. One view past `MAX_VIEWS` is refused with an error that says so (shown in the pane); an older process gets at most `MAX_VIEWS` views over its connection's life. Added `Process::open_views` and `architect::SUBSCRIPTION_RELAY`: an `#[architect::rpc]` `#[subscribe]` stream with arguments now ends its request when its backend drops the sink.
