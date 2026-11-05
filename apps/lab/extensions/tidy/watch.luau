-- Every document event from now on runs LAB_TIDY_HEARD.
return host.watch_documents({ action = "LAB_TIDY_HEARD" })
