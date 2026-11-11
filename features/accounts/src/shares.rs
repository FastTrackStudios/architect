//! Share links, from the app (#374): `architect_spaces::ShareLinks` on
//! the node that hosts the current org, at `/org/{slug}/vox`, signed in
//! as the active account (the same host [`crate::Account::org_host`] as
//! the org's devices).
//!
//! A share is a note of the vault it shares, so it syncs with the vault;
//! the host also holds it the moment it answers, so a link works, and a
//! revoked one stops working, before the note has reached anyone else.

use architect::connect::{Endpoint, Pool};
use architect_spaces::{
    Listen, NewAudience, NewShare, Share, ShareChange, ShareComment, ShareLinksClient, ShareScope,
};

use crate::controller::{Accounts, Sheet};

/// What the Share sheet is about to share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareDraft {
    /// The vault, by the host's id for it; empty: the host's only one.
    pub vault: String,
    pub scope: ShareScope,
}

/// Which archive [`Accounts::share_archive`] downloads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveAsk {
    pub code: String,
    /// One audience's files, by its name; none: the whole share's.
    pub audience: Option<String>,
    /// `order`, `note` or `property:<key>`.
    pub by: String,
    /// One group's files, by its label; none: every group's.
    pub group: Option<String>,
}

/// `s` as a query value.
fn query_value(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl Accounts {
    async fn share_links(&self, host: Option<&str>) -> Result<(String, ShareLinksClient), String> {
        let (org, host) = self.org_and_host(host)?;
        let token = self
            .store()
            .active_session()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no account is signed in".to_owned())?
            .token;
        let endpoint =
            Endpoint::url(crate::enrolment::lane(&host, &org.slug)).with_bearer(Some(token));
        let client = Pool::new()
            .client(&endpoint)
            .await
            .map_err(|e| format!("{host}: {e}"))?;
        Ok((host, client))
    }

    /// The link of `share` on the current org's host.
    #[must_use]
    pub fn share_url(&self, share: &Share) -> Option<String> {
        self.org_host().map(|host| share.url(&host))
    }

    /// Every share of the host's vaults; kept for the sheet.
    ///
    /// # Errors
    ///
    /// Signed out, in no org, no host, or the host refused.
    pub async fn shares(&self, host: Option<&str>) -> Result<Vec<Share>, String> {
        let answer = async {
            let (_, client) = self.share_links(host).await?;
            client.shares().await.map_err(|e| e.to_string())
        }
        .await;
        self.state().shares = Some(answer.clone());
        self.changed();
        answer
    }

    /// The shares as last fetched.
    #[must_use]
    pub fn shares_seen(&self) -> Option<Result<Vec<Share>, String>> {
        self.state().shares.clone()
    }

    /// Make a share link.
    ///
    /// # Errors
    ///
    /// As [`Self::shares`], or the share is invalid (an empty scope, an
    /// expiry already past).
    pub async fn share_create(&self, host: Option<&str>, new: NewShare) -> Result<Share, String> {
        let (_, client) = self.share_links(host).await?;
        let share = client.create(new).await.map_err(|e| e.to_string())?;
        self.state().share_made = Some(share.clone());
        let _ = self.shares(None).await;
        Ok(share)
    }

    /// Change share `code`.
    ///
    /// # Errors
    ///
    /// As [`Self::shares`], no such share, or a revoked one.
    pub async fn share_update(
        &self,
        host: Option<&str>,
        code: &str,
        change: ShareChange,
    ) -> Result<Share, String> {
        let (_, client) = self.share_links(host).await?;
        let share = client
            .update(code.to_owned(), change)
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.shares(None).await;
        Ok(share)
    }

    /// Revoke share `code`: its link is 404 from the next request.
    ///
    /// # Errors
    ///
    /// As [`Self::shares`], or no such share.
    pub async fn share_revoke(&self, host: Option<&str>, code: &str) -> Result<Share, String> {
        let (_, client) = self.share_links(host).await?;
        let share = client
            .revoke(code.to_owned())
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.shares(None).await;
        Ok(share)
    }

    /// Add a named audience to share `code` (#510): the notes of its
    /// scope `filter` (a Bases filter expression) holds for, on a link
    /// of its own.
    ///
    /// # Errors
    ///
    /// As [`Self::shares`], no such share, a name it has, or a filter
    /// that does not parse.
    pub async fn share_audience_add(
        &self,
        host: Option<&str>,
        code: &str,
        new: NewAudience,
    ) -> Result<Share, String> {
        let (_, client) = self.share_links(host).await?;
        let share = client
            .add_audience(code.to_owned(), new)
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.shares(None).await;
        Ok(share)
    }

    /// Remove audience `name` from share `code`: its link is 404 from
    /// the next request; the share and its other audiences stay.
    ///
    /// # Errors
    ///
    /// As [`Self::shares`], or no such share or audience.
    pub async fn share_audience_remove(
        &self,
        host: Option<&str>,
        code: &str,
        name: &str,
    ) -> Result<Share, String> {
        let (_, client) = self.share_links(host).await?;
        let share = client
            .remove_audience(code.to_owned(), name.to_owned())
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.shares(None).await;
        Ok(share)
    }

    /// How far audience `name`'s link of share `code` plays (#528):
    /// `listen`, never past the share's; None: the share's.
    ///
    /// # Errors
    ///
    /// As [`Self::shares`], or no such share or audience.
    pub async fn share_audience_listen(
        &self,
        host: Option<&str>,
        code: &str,
        name: &str,
        listen: Option<Listen>,
    ) -> Result<Share, String> {
        let (_, client) = self.share_links(host).await?;
        let share = client
            .audience_listen(code.to_owned(), name.to_owned(), listen)
            .await
            .map_err(|e| e.to_string())?;
        let _ = self.shares(None).await;
        Ok(share)
    }

    /// Share `code` as the host has it now.
    ///
    /// # Errors
    ///
    /// As [`Self::shares`], or no such share.
    pub async fn share(&self, host: Option<&str>, code: &str) -> Result<Share, String> {
        self.shares(host)
            .await?
            .into_iter()
            .find(|s| s.code == code)
            .ok_or_else(|| format!("no share {code}"))
    }

    /// Download the zip of a share's files (#510) into the file `to`:
    /// the files of share `code`, or of its audience `audience`, in
    /// folders by `by` (`order`, `note`, `property:<key>`), one `group`
    /// of them or all. The same archive the link's "Download all"
    /// gives, streamed to the file as it comes. Answers the archive's
    /// URL and how many bytes were written.
    ///
    /// # Errors
    ///
    /// As [`Self::shares`], no such share, audience or group, nothing
    /// to download, or `to` could not be written.
    pub async fn share_archive(
        &self,
        host: Option<&str>,
        ask: &ArchiveAsk,
        to: &std::path::Path,
    ) -> Result<(String, u64), String> {
        use std::io::Write as _;
        let share = self.share(host, &ask.code).await?;
        let link = match ask
            .audience
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            Some(name) => share
                .audience_named(name)
                .map(architect_spaces::ShareAudience::path)
                .ok_or_else(|| format!("share {} has no audience `{name}`", ask.code))?,
            None => share.path(),
        };
        let mut path = format!("{link}_archive.zip?by={}", query_value(&ask.by));
        if let Some(group) = &ask.group {
            path.push_str("&group=");
            path.push_str(&query_value(group));
        }
        let (_, origin) = self.org_and_host(host)?;
        let token = self
            .store()
            .active_session()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no account is signed in".to_owned())?
            .token;
        let client = architect::http::HttpClient::new(origin.clone()).with_bearer(token);
        let mut file = std::fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
        let got = client.download(&path, |chunk| file.write_all(chunk)).await;
        match got {
            Ok(bytes) => Ok((format!("{}{path}", origin.trim_end_matches('/')), bytes)),
            Err(why) => {
                drop(file);
                let _ = std::fs::remove_file(to);
                Err(match why {
                    architect::ClientError::Transport { detail, .. } => detail,
                    other => format!("{other:?}"),
                })
            }
        }
    }

    /// The comments left on share `code`.
    ///
    /// # Errors
    ///
    /// As [`Self::shares`], or no such share.
    pub async fn share_comments(
        &self,
        host: Option<&str>,
        code: &str,
    ) -> Result<Vec<ShareComment>, String> {
        let (_, client) = self.share_links(host).await?;
        client
            .comments(code.to_owned())
            .await
            .map_err(|e| e.to_string())
    }

    /// Open the Share sheet on `draft` (and fetch the shares).
    pub fn open_share_sheet(&self, draft: ShareDraft) {
        {
            let mut state = self.state();
            state.share_draft = Some(draft);
            state.share_made = None;
        }
        self.open_sheet(Sheet::Share);
    }

    /// What the Share sheet is sharing, and the share it just made.
    #[must_use]
    pub fn share_draft(&self) -> (Option<ShareDraft>, Option<Share>) {
        let state = self.state();
        (state.share_draft.clone(), state.share_made.clone())
    }
}
