//! Share links, from the app (#374): `architect_spaces::ShareLinks` on
//! the node that hosts the current org, at `/org/{slug}/vox`, signed in
//! as the active account (the same host [`crate::Account::org_host`] as
//! the org's devices).
//!
//! A share is a note of the vault it shares, so it syncs with the vault;
//! the host also holds it the moment it answers, so a link works, and a
//! revoked one stops working, before the note has reached anyone else.

use architect::connect::{Endpoint, Pool};
use architect_spaces::{NewShare, Share, ShareChange, ShareComment, ShareLinksClient, ShareScope};

use crate::controller::{Accounts, Sheet};

/// What the Share sheet is about to share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareDraft {
    /// The vault, by the host's id for it; empty: the host's only one.
    pub vault: String,
    pub scope: ShareScope,
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
